#!/usr/bin/env python3
import os
import sys
import time
import random
sys.path.insert(1, os.path.dirname(sys.path[0]))

from test_framework.test_framework import ConfluxTestFramework
from test_framework.simple_rpc_proxy import ReceivedErrorResponseError
from test_framework.util import sync_blocks, connect_nodes, connect_sample_nodes, assert_equal, assert_blocks_valid, \
    wait_until
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
POS0_VALUE_AFTER_INCREMENT = "0x00000000000000000000000000000000000000000000000000000000000004d3"
POS1_VALUE = "0x000000000000000000000000000000000000000000000000000000000000162e"

class SyncCheckpointTests(ConfluxTestFramework):
    def set_test_params(self):
        self.num_nodes = 3
        self.conf_parameters = {
            "dev_snapshot_epoch_count": "10",
            "adaptive_weight_beta": "1",
            "timer_chain_block_difficulty_ratio": "2",
            "timer_chain_beta": "6",
            "era_epoch_count": "50",
            "chunk_size_byte": "1000",
            "anticone_penalty_ratio": "5",
            # Make sure checkpoint synchronization is triggered during phase change.
            "dev_allow_phase_change_without_peer": "false",
            # Disable pos reference because pow blocks are generated too fast.
            "pos_reference_enable_height": "10000",
            "cip1559_transition_height": "10000",
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

    def run_test(self):
        num_blocks = 200
        snapshot_epoch = 150
        snapshot_epoch_count = int(self.conf_parameters["dev_snapshot_epoch_count"])

        # Block number i of the loop below is the block of epoch i + 1.
        #
        # The contract is deployed long before the landing epoch, so that the
        # merged snapshot which lands at `snapshot_epoch` carries its storage.
        # Its storage is written again in the last snapshot period before the
        # landing (epochs 141..150 here), so that an archive node opening the
        # epochs right after the landing has this contract in its intermediate
        # layer: the intermediate layer of epoch 151 is the delta MPT of
        # epochs 141..150.
        deploy_at_block = 10
        deploy_receipt_at_block = 40
        update_at_block = 144

        # Generate checkpoint on node[0]
        archive_node_client = RpcClient(self.nodes[0])
        self.genesis_nonce = archive_node_client.get_nonce(archive_node_client.GENESIS_ADDR)
        bytecode_file = os.path.join(os.path.dirname(os.path.dirname(os.path.realpath(__file__))), CONTRACT_PATH)
        assert os.path.isfile(bytecode_file)
        bytecode = open(bytecode_file).read()
        create_tx = None
        update_tx = None
        contract_addr = None
        blocks_in_era = []
        for i in range(num_blocks):
            txs = self._generate_txs(0, random.randint(50, 100))
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
            if i == update_at_block:
                update_tx = archive_node_client.new_contract_tx(
                    receiver=contract_addr, data_hex=encode_hex_0x(keccak(b"increment()")),
                    nonce=self.genesis_nonce, epoch_height=archive_node_client.epoch_number())
                self.genesis_nonce += 1
                txs.append(update_tx)
            block_hash = archive_node_client.generate_block_with_fake_txs(txs)
            if i >= snapshot_epoch:
                blocks_in_era.append(block_hash)
        wait_until(lambda: archive_node_client.get_transaction_receipt(update_tx.hash_hex()) is not None)
        assert_equal(int(archive_node_client.get_transaction_receipt(update_tx.hash_hex())["outcomeStatus"], 0), 0)
        assert_equal(
            archive_node_client.get_storage_at(contract_addr, POS0_KEY, archive_node_client.EPOCH_NUM(snapshot_epoch)),
            POS0_VALUE_AFTER_INCREMENT,
        )
        sync_blocks(self.nodes[:-1])
        self.log.info("All archive nodes synced")

        # Start node[full_node_index] as full node to sync checkpoint
        # Change phase from CatchUpSyncBlockHeader to CatchUpCheckpoint
        # only when there is at least one connected peer.
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
        assert full_node_client.epoch_number() >= snapshot_epoch
        wait_until(lambda: full_node_client.epoch_number() == archive_node_client.epoch_number() and
                   full_node_client.epoch_number("latest_state") == archive_node_client.epoch_number("latest_state"))
        # Nothing below the synced snapshot was ever executed here.
        for i in range(1, snapshot_epoch):
            try:
                full_node_client.get_balance(full_node_client.GENESIS_ADDR, full_node_client.EPOCH_NUM(i))
                raise AssertionError("should not have state for epoch {}".format(i))
            except ReceivedErrorResponseError as e:
                assert "State for epoch" in e.response.message
                assert "does not exist" in e.response.message

        # The synced snapshot is a merged image holding the whole state at that
        # epoch, so values at that epoch are served and agree with the archive
        # node.
        assert_equal(
            full_node_client.get_balance(full_node_client.GENESIS_ADDR, full_node_client.EPOCH_NUM(snapshot_epoch)),
            archive_node_client.get_balance(archive_node_client.GENESIS_ADDR, archive_node_client.EPOCH_NUM(snapshot_epoch)),
        )

        # The first of the two keys below was last written at epoch 145, i.e.
        # inside the last snapshot period before the landing, the second one at
        # the epoch the contract was deployed.
        for key, expected in [(POS0_KEY, POS0_VALUE_AFTER_INCREMENT), (POS1_KEY, POS1_VALUE)]:
            full_value = full_node_client.get_storage_at(contract_addr, key, full_node_client.EPOCH_NUM(snapshot_epoch))
            archive_value = archive_node_client.get_storage_at(contract_addr, key, archive_node_client.EPOCH_NUM(snapshot_epoch))
            assert_equal(full_value, expected)
            assert_equal(full_value, archive_value)

        # Answers which decompose the state into its three layers cannot be
        # given at that epoch: the merged image does not have the layering the
        # block header commits to.
        try:
            full_node_client.get_storage_root(full_node_client.GENESIS_ADDR, full_node_client.EPOCH_NUM(snapshot_epoch))
            raise AssertionError("should not have storage root for epoch {}".format(snapshot_epoch))
        except ReceivedErrorResponseError as e:
            assert "State for epoch" in e.response.message
            assert "does not exist" in e.response.message

        # Wait for execution to complete.
        time.sleep(1)

        # The parent of the landed snapshot was never downloaded, so the whole
        # first period after the landing carries a blank intermediate layer:
        # `get_state_trees_for_next_epoch` falls back to the snapshot of
        # the synced epoch, and `get_state_trees` blanks it again on the
        # read-only opens. `check_freshly_synced_snapshot` refuses node merkle
        # queries on such a state and `cfx_getStorageRoot` is one, so the full
        # node refuses here while the archive node, which has the parent
        # snapshot and hence a real intermediate layer, answers normally.
        for epoch in range(snapshot_epoch + 1, snapshot_epoch + snapshot_epoch_count + 1):
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
            assert_equal(
                full_node_client.get_storage_at(contract_addr, POS0_KEY, full_node_client.EPOCH_NUM(epoch)),
                POS0_VALUE_AFTER_INCREMENT,
            )
        self.log.info("Epochs %d..%d roll on the degenerate layering",
                      snapshot_epoch + 1, snapshot_epoch + snapshot_epoch_count)

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

        # There should be states after checkpoint
        idx = 0
        for i in range(snapshot_epoch + 1, full_node_client.epoch_number() - 3):
            full_balance = full_node_client.get_balance(full_node_client.GENESIS_ADDR, full_node_client.EPOCH_NUM(i))
            archive_balance = archive_node_client.get_balance(archive_node_client.GENESIS_ADDR, archive_node_client.EPOCH_NUM(i))
            assert_equal(full_balance, archive_balance)
            executed_info1 = self.nodes[full_node_index].test_getExecutedInfo(blocks_in_era[idx])
            executed_info2 = self.nodes[0].test_getExecutedInfo(blocks_in_era[idx])
            assert_equal(executed_info1, executed_info2)
            idx += 1

        # Blocks within execution defer (5 epochs) and reward_defer (12 epochs) do not have state_valid
        available_blocks = blocks_in_era[:-17]
        assert_blocks_valid(self.nodes[:-1], available_blocks)
        assert_blocks_valid(self.nodes[-1:], available_blocks)


if __name__ == "__main__":
    SyncCheckpointTests().main()
