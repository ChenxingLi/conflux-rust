#!/usr/bin/env python3
import os
import re
import shutil
import sys
import time
import random

sys.path.insert(1, os.path.dirname(sys.path[0]))

from test_framework.test_framework import ConfluxTestFramework
from test_framework.simple_rpc_proxy import ReceivedErrorResponseError
from test_framework.util import sync_blocks, connect_nodes, connect_sample_nodes, assert_equal, wait_until
from test_framework.blocktools import encode_hex_0x
from conflux.rpc import RpcClient
from conflux.utils import sha3 as keccak

CONTRACT_PATH = "contracts/simple_storage.dat"

# `simple_storage` writes 1234 into `pos0` and 5678 into `pos1[0x3916...]` in
# its constructor, and `increment()` bumps `pos0` by one. The two keys are the
# storage slots those two values live in, the same ones tests/storage_rpc_test.py
# reads.
POS0_KEY = "0x0000000000000000000000000000000000000000000000000000000000000000"
POS1_KEY = "0x6661e9d6d8b923d5bbaab1b96e1dd51ff6ea2a93520fdc9eb75d059238b8c5e9"
POS0_INITIAL_VALUE = 1234
POS1_VALUE = "0x000000000000000000000000000000000000000000000000000000000000162e"

REBUILD_SUMMARY_PATTERN = re.compile(
    r"state index rebuild: (\d+) entries written, physical openable lower "
    r"bound (\d+); (\d+) periods and (\d+) single heights left out"
)
# Part of the warning the rebuild prints when the snapshot root recorded for
# an epoch differs from the merkle root of the snapshot registered for it.
ROOT_DISAGREEMENT_MARKER = "carries snapshot root"
SKIPPED_RANGE_PATTERN = re.compile(
    r"Writing no entry for heights (\d+)\.\.=(\d+)"
)
SKIPPED_STATE_SYNC_MARKER = "skip state sync"

class SyncCheckpointTests(ConfluxTestFramework):
    def set_test_params(self):
        self.num_nodes = 3
        self.conf_parameters = {
            "dev_snapshot_epoch_count": "200",
            "adaptive_weight_beta": "1",
            "timer_chain_block_difficulty_ratio": "2",
            "timer_chain_beta": "6",
            "era_epoch_count": "1000",
            "chunk_size_byte": "1000",
            "anticone_penalty_ratio": "5",
            # Make sure checkpoint synchronization is triggered during phase change.
            "dev_allow_phase_change_without_peer": "false",
            # Disable pos reference because pow blocks are generated too fast.
            "pos_reference_enable_height": "10000",
            "cip1559_transition_height": "10000",
            "keep_snapshot_before_stable_checkpoint": "false",
        }

    def setup_network(self):
        self.add_nodes(self.num_nodes)
        for i in range(self.num_nodes - 1):
            self.start_node(i, phase_to_wait=None)
        connect_sample_nodes(self.nodes[:-1], self.log, latency_max=1)
        for i in range(self.num_nodes - 1):
            self.nodes[i].wait_for_recovery(["NormalSyncPhase"], 10)

    def _generate_txs(self, peer, num):
        client = RpcClient(self.nodes[peer])
        txs = []
        for _ in range(num):
            addr = client.rand_addr()
            tx_gas = client.DEFAULT_TX_GAS
            tx = client.new_tx(receiver=addr, nonce=self.genesis_nonce, value=21000, gas=tx_gas, data=b'')
            self.genesis_nonce += 1
            txs.append(tx)
        return txs

    def _remove_state_index(self, index):
        """Leave a stopped node's data directory the way an upgrade from a
        binary without the engine's state index leaves it: everything else on
        disk, and no index. The index is a rocksdb, so it is a directory."""
        path = os.path.join(self.nodes[index].datadir, "blockchain_data",
                            "storage_db", "state_index_db")
        assert os.path.isdir(path), \
            "no state index at {}, so the first boot path this checks " \
            "would not be taken".format(path)
        shutil.rmtree(path)
        self.log.info("removed %s", path)

    def _log_size(self, index):
        return os.path.getsize(os.path.join(self.nodes[index].datadir, "conflux.log"))

    def _read_log(self, index, offset=0):
        with open(os.path.join(self.nodes[index].datadir, "conflux.log"),
                  encoding="utf8", errors="replace") as f:
            f.seek(offset)
            return f.read()

    def _check_index_was_rebuilt(self, log_text, snapshot_epoch):
        """Assert that the entry for `snapshot_epoch` came from the rebuild.

        `snapshot_epoch` is the landing epoch, and opening it needs its index
        entry. Only one thing other than the rebuild could have written that
        entry: a second download of the checkpoint, through
        `register_synced_snapshot_state`, which is what this method rules out.
        Re-execution could not have written it, since re-executing needs the
        blocks below the landing epoch, which a synced node never downloaded.
        """
        summary = REBUILD_SUMMARY_PATTERN.search(log_text)
        assert summary is not None, \
            "no state index rebuild summary in the boot log; the first boot " \
            "path this checks would not have been taken"
        written, published_bound = int(summary.group(1)), int(summary.group(2))
        self.log.info("the rebuild wrote %d entries and published bound %d",
                      written, published_bound)
        assert SKIPPED_STATE_SYNC_MARKER in log_text, \
            "the boot did not report finding the checkpoint state already on " \
            "disk, so it may have synced it again and registered the landing " \
            "entry through the sync rather than through the rebuild"
        assert ROOT_DISAGREEMENT_MARKER not in log_text, \
            "the rebuild found a commitment row and the snapshot registry " \
            "describing different states"
        # A snapshot period gets entries only if the data of the snapshot it
        # sits on, or of the snapshot one period below that, is still on
        # disk. A synced node has neither for the period ending at the
        # landing epoch, so that period is the only one whose entries may be
        # missing. Every height above the landing epoch must have an entry,
        # including the heights of the period `_check_synced_landing` reads.
        skipped = [(int(begin), int(end))
                   for begin, end in SKIPPED_RANGE_PATTERN.findall(log_text)]
        self.log.info("height ranges the rebuild left out: %s", skipped)
        assert_equal([r for r in skipped if r[1] > snapshot_epoch], [])
        # The landing epoch's parent snapshot was registered without its data files,
        # so nothing below the landing epoch can be opened.
        assert_equal(published_bound, snapshot_epoch)

    def _find_landing_epoch(self, full_node_client):
        """The lowest epoch the full node can still answer state queries for.
        Nothing below the synced snapshot was ever executed here, so this is
        the epoch the snapshot landed at."""
        lo = 1
        hi = full_node_client.epoch_number("latest_state")
        while lo < hi:
            mid = (lo + hi) // 2
            try:
                full_node_client.get_balance(full_node_client.GENESIS_ADDR, full_node_client.EPOCH_NUM(mid))
                hi = mid
            except ReceivedErrorResponseError:
                lo = mid + 1
        return lo

    def _check_synced_landing(self, stage, full_node_client, archive_node_client,
                              snapshot_epoch, snapshot_epoch_count, contract_addr, expected_pos0):
        """Check the landing epoch and the snapshot period which follows it.

        Called once right after the sync and once after a restart from the same
        data directory: the landing entry is the only copy of these coordinates
        on disk, and it is persisted, so a restart must not change any of the
        answers below."""
        self.log.info("Checking the landing at epoch %d (%s)", snapshot_epoch, stage)

        # Nothing below the synced snapshot was ever executed here.
        for epoch in [1, snapshot_epoch - 1]:
            try:
                full_node_client.get_balance(full_node_client.GENESIS_ADDR, full_node_client.EPOCH_NUM(epoch))
                raise AssertionError("should not have state for epoch {}".format(epoch))
            except ReceivedErrorResponseError as e:
                assert "State for epoch" in e.response.message
                assert "does not exist" in e.response.message

        # The synced snapshot is a merged image holding the whole state at that
        # epoch, so values at that epoch are served and agree with the archive
        # node. Contract storage is where most of the state lives, so a storage
        # key is queried next to the account balance.
        assert_equal(
            full_node_client.get_balance(full_node_client.GENESIS_ADDR, full_node_client.EPOCH_NUM(snapshot_epoch)),
            archive_node_client.get_balance(archive_node_client.GENESIS_ADDR, archive_node_client.EPOCH_NUM(snapshot_epoch)),
        )
        for key, expected in [(POS0_KEY, expected_pos0), (POS1_KEY, POS1_VALUE)]:
            full_value = full_node_client.get_storage_at(contract_addr, key, full_node_client.EPOCH_NUM(snapshot_epoch))
            archive_value = archive_node_client.get_storage_at(contract_addr, key, archive_node_client.EPOCH_NUM(snapshot_epoch))
            assert_equal(full_value, expected)
            assert_equal(full_value, archive_value)

        # Answers which decompose the state into its three layers cannot be
        # given at that epoch: the merged image does not have the layering the
        # block header commits to.
        try:
            full_node_client.get_storage_root(contract_addr, full_node_client.EPOCH_NUM(snapshot_epoch))
            raise AssertionError("should not have storage root for epoch {}".format(snapshot_epoch))
        except ReceivedErrorResponseError as e:
            assert "State for epoch" in e.response.message
            assert "does not exist" in e.response.message

        # The parent of the landed snapshot was never downloaded, so the whole
        # first period after the landing carries a blank intermediate layer:
        # `get_state_trees_for_next_epoch` falls back to the snapshot of
        # the synced epoch, and `get_state_trees` blanks it again on the
        # read-only opens. `check_freshly_synced_snapshot` refuses node merkle
        # queries on such a state and `cfx_getStorageRoot` is one, so the full
        # node refuses here while the archive node, which has the parent
        # snapshot and hence a real intermediate layer, answers normally.
        for epoch in [snapshot_epoch + 1, snapshot_epoch + snapshot_epoch_count]:
            archive_root = archive_node_client.get_storage_root(contract_addr, archive_node_client.EPOCH_NUM(epoch))
            assert archive_root["intermediate"] is not None, \
                "archive node has no intermediate layer for the contract at epoch {}".format(epoch)
            try:
                full_node_client.get_storage_root(contract_addr, full_node_client.EPOCH_NUM(epoch))
                raise AssertionError("should not answer a node merkle query for epoch {}".format(epoch))
            except ReceivedErrorResponseError as e:
                assert "freshly synced snapshot" in e.response.message, e.response.message
            # The values are served from the merged snapshot all the same, so
            # the blank intermediate layer is not a blank base: an epoch rolled
            # onto the empty genesis snapshot would answer None here.
            full_value = full_node_client.get_storage_at(contract_addr, POS0_KEY, full_node_client.EPOCH_NUM(epoch))
            assert full_value is not None
            assert_equal(
                full_value,
                archive_node_client.get_storage_at(contract_addr, POS0_KEY, archive_node_client.EPOCH_NUM(epoch)),
            )

        # One period later the shift finds its new snapshot layer locally: it
        # is the merged snapshot, which the epochs above carry as their
        # intermediate epoch. The layering is ordinary again from here on, and
        # the snapshot layer it reports is the merged one, which is the same
        # image the archive node built by executing every epoch.
        next_period_epoch = snapshot_epoch + snapshot_epoch_count + 1
        full_root = full_node_client.get_storage_root(contract_addr, full_node_client.EPOCH_NUM(next_period_epoch))
        archive_root = archive_node_client.get_storage_root(contract_addr, archive_node_client.EPOCH_NUM(next_period_epoch))
        assert full_root["snapshot"] is not None
        assert_equal(full_root["snapshot"], archive_root["snapshot"])

    def run_test(self):
        num_blocks = 2950
        snapshot_epoch_count = int(self.conf_parameters["dev_snapshot_epoch_count"])
        blocks = []

        # Block number i of the loop below is the block of epoch i + 1.
        #
        # The contract is deployed long before any epoch the sync could land
        # on, so that the merged snapshot which lands carries its storage. Its
        # storage is then written once halfway through every snapshot period.
        # The landing epoch is a snapshot period boundary, whichever one the
        # sync picks, so the period right below it always holds one of these
        # writes: an archive node opening the epochs right after the landing
        # therefore has this contract in its intermediate layer, which is the
        # delta MPT of that period.
        deploy_at_block = 10
        deploy_receipt_at_block = 40

        # Generate checkpoint
        archive_node_client = RpcClient(self.nodes[0])
        self.genesis_nonce = archive_node_client.get_nonce(archive_node_client.GENESIS_ADDR)
        bytecode_file = os.path.join(os.path.dirname(os.path.dirname(os.path.realpath(__file__))), CONTRACT_PATH)
        assert os.path.isfile(bytecode_file)
        bytecode = open(bytecode_file).read()
        create_tx = None
        contract_addr = None
        update_txs = []
        for i in range(num_blocks):
            txs = self._generate_txs(0, random.randint(1, 2))
            if i == deploy_at_block:
                create_tx = archive_node_client.new_contract_tx(
                    receiver="", data_hex=bytecode, nonce=self.genesis_nonce,
                    storage_limit=20000, epoch_height=archive_node_client.epoch_number())
                self.genesis_nonce += 1
                txs.append(create_tx)
            if i == deploy_receipt_at_block:
                wait_until(lambda: archive_node_client.get_transaction_receipt(create_tx.hash_hex()) is not None)
                receipt = archive_node_client.get_transaction_receipt(create_tx.hash_hex())
                assert_equal(int(receipt["outcomeStatus"], 0), 0)
                contract_addr = receipt["contractCreated"]
                self.log.info("simple_storage deployed at %s", contract_addr)
            if contract_addr is not None and (i + 1) % snapshot_epoch_count == snapshot_epoch_count // 2:
                update_tx = archive_node_client.new_contract_tx(
                    receiver=contract_addr, data_hex=encode_hex_0x(keccak(b"increment()")),
                    nonce=self.genesis_nonce, epoch_height=archive_node_client.epoch_number())
                self.genesis_nonce += 1
                update_txs.append(update_tx)
                txs.append(update_tx)
            block_hash = archive_node_client.generate_block_with_fake_txs(txs)
            blocks.append(block_hash)
        wait_until(lambda: archive_node_client.get_transaction_receipt(update_txs[-1].hash_hex()) is not None)
        for update_tx in update_txs:
            assert_equal(int(archive_node_client.get_transaction_receipt(update_tx.hash_hex())["outcomeStatus"], 0), 0)
        sync_blocks(self.nodes[:-1])
        self.log.info("All archive nodes synced")

       
        full_node_index = self.num_nodes - 1
        self.start_node(full_node_index, ["--full"], phase_to_wait=None)
        for i in range(self.num_nodes - 1):
            connect_nodes(self.nodes, full_node_index, i)

        self.log.info("Wait for full node to sync, index=%d", full_node_index)
        self.nodes[full_node_index].wait_for_phase(["NormalSyncPhase"], wait_time=240)

        sync_blocks(self.nodes, sync_count=False)

        full_node_client = RpcClient(self.nodes[full_node_index])

        # At epoch 1, block header exists while body not synchronized
        try:
            self.log.info("block at epoch 1: %s", full_node_client.block_by_epoch(full_node_client.EPOCH_NUM(1)))
        except ReceivedErrorResponseError as e:
            assert 'Internal error' == e.response.message

        # There is no state from epoch 1 to snapshot_epoch
        # Note, state of genesis epoch always exists
        wait_until(lambda: full_node_client.epoch_number() == archive_node_client.epoch_number() and
                   full_node_client.epoch_number("latest_state") == archive_node_client.epoch_number("latest_state"))

        # Wait for execution to complete.
        time.sleep(1)

        snapshot_epoch = self._find_landing_epoch(full_node_client)
        assert_equal(snapshot_epoch % snapshot_epoch_count, 0)
        # `increment()` was executed once halfway through every snapshot period
        # below the landing epoch.
        expected_pos0 = "0x{:064x}".format(POS0_INITIAL_VALUE + snapshot_epoch // snapshot_epoch_count)
        self._check_synced_landing("after the sync", full_node_client, archive_node_client,
                                   snapshot_epoch, snapshot_epoch_count, contract_addr, expected_pos0)

        # There should be states after checkpoint
        for block_hash in blocks[1000: -4]:
            executed_info1 = self.nodes[full_node_index].test_getExecutedInfo(block_hash)
            executed_info2 = self.nodes[0].test_getExecutedInfo(block_hash)
            assert_equal(executed_info1, executed_info2)

        # The landing entry is the only copy of the landing coordinates on
        # disk, so a restart from the same data directory has nothing else to
        # fall back on. Every answer above has to come out the same way.
        self.log.info("Restarting the full node from its own data directory")
        self.nodes[full_node_index].stop_node()
        self.nodes[full_node_index].wait_until_stopped()
        self.start_node(full_node_index, None, phase_to_wait=None)
        for i in range(self.num_nodes - 1):
            connect_nodes(self.nodes, full_node_index, i)
        self.nodes[full_node_index].wait_for_phase(["NormalSyncPhase"], wait_time=240)
        wait_until(lambda: full_node_client.epoch_number("latest_state") == archive_node_client.epoch_number("latest_state"))
        self._check_synced_landing("after a restart", full_node_client, archive_node_client,
                                   snapshot_epoch, snapshot_epoch_count, contract_addr, expected_pos0)

        # The same disk on the first boot of a binary which has the version
        # index, which is what upgrading a node that synced under an older one
        # looks like. The index is gone and the rebuild has to put the landing
        # back into it from the snapshot registry, where the landing period's
        # own layers were never registered, because they were never downloaded.
        self.log.info("Restarting the full node with its state index removed")
        self.nodes[full_node_index].stop_node()
        self.nodes[full_node_index].wait_until_stopped()
        self._remove_state_index(full_node_index)
        rebuild_log_offset = self._log_size(full_node_index)
        self.start_node(full_node_index, None, phase_to_wait=None)
        for i in range(self.num_nodes - 1):
            connect_nodes(self.nodes, full_node_index, i)
        self.nodes[full_node_index].wait_for_phase(["NormalSyncPhase"], wait_time=240)
        wait_until(lambda: full_node_client.epoch_number("latest_state") == archive_node_client.epoch_number("latest_state"))
        self._check_index_was_rebuilt(
            self._read_log(full_node_index, rebuild_log_offset), snapshot_epoch)
        self._check_synced_landing("after the state index was rebuilt", full_node_client, archive_node_client,
                                   snapshot_epoch, snapshot_epoch_count, contract_addr, expected_pos0)

        self.nodes[full_node_index].stop_node()
        self.nodes[full_node_index].wait_until_stopped()

        num_blocks = 1500
        for i in range(num_blocks):
            txs = self._generate_txs(0, random.randint(1, 2))
            block_hash = archive_node_client.generate_block_with_fake_txs(txs)
            blocks.append(block_hash)
        sync_blocks(self.nodes[:-1])
        self.log.info("All archive nodes synced")

        self.start_node(full_node_index, None, phase_to_wait=None)
        for i in range(self.num_nodes - 1):
            connect_nodes(self.nodes, full_node_index, i)

        self.log.info("Wait for full node to sync, index=%d", full_node_index)
        self.nodes[full_node_index].wait_for_phase(["NormalSyncPhase"], wait_time=240)

        sync_blocks(self.nodes, sync_count=False)

        wait_until(lambda: full_node_client.epoch_number() == archive_node_client.epoch_number() and
                   full_node_client.epoch_number("latest_state") == archive_node_client.epoch_number("latest_state"))
        time.sleep(1)

        for block_hash in blocks[3000: -4]:
            executed_info1 = self.nodes[full_node_index].test_getExecutedInfo(block_hash)
            executed_info2 = self.nodes[0].test_getExecutedInfo(block_hash)
            assert_equal(executed_info1, executed_info2)


if __name__ == "__main__":
    SyncCheckpointTests().main()
