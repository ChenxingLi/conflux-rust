#!/usr/bin/env python3
"""Locate the epoch heights at which the three storage layers shift.

`storage_root_test.py` proves that the rotation happens and that the layer
which freezes into `intermediate` is the previous period's delta, but it only
ever samples `latest_state`, so it stays true if every boundary moves one epoch
along the chain. This test pins the boundaries to concrete heights.

The expected heights are derived from the engine's own arithmetic, not from
what the node is observed to do: `StateIndex::height_to_delta_height` maps a
commit height to the delta depth recorded for it, and
`get_state_trees_for_next_epoch` shifts when the parent's delta depth equals
the snapshot period. The two are ported below, so a boundary that moved is a
disagreement between the formula and the node, not an expected value that was
read off a run.
"""
import os

import eth_utils

from conflux.config import default_config
from conflux.rpc import RpcClient
from conflux.utils import sha3 as keccak, priv_to_addr
from test_framework.blocktools import encode_hex_0x
from test_framework.test_framework import ConfluxTestFramework
from test_framework.util import *

CONTRACT_PATH = "contracts/simple_storage.dat"

FULLNODE0 = 0
SNAPSHOT_EPOCH_COUNT = 50

# Deferred execution: `latest_state` trails the newest mined epoch by this much.
DEFERRED_STATE_EPOCH_COUNT = 5


def height_to_delta_height(height, snapshot_epoch_count):
    """Port of `StateIndex::height_to_delta_height`."""
    if height == 0:
        return 0
    return (height - 1) % snapshot_epoch_count + 1


def shifts_at(height, snapshot_epoch_count):
    """Does opening `height` on top of its parent rotate the three layers?

    `get_state_trees_for_next_epoch` takes the parent's delta depth out of the
    parent's index entry and compares it with the snapshot period. The parent
    of `height` is `height - 1`, and its entry holds
    `height_to_delta_height(height - 1)`.
    """
    if height == 0:
        return False
    return height_to_delta_height(height - 1, snapshot_epoch_count) \
        == snapshot_epoch_count


class StorageShiftBoundaryTest(ConfluxTestFramework):
    def set_test_params(self):
        self.num_nodes = 1
        self.conf_parameters["dev_snapshot_epoch_count"] = str(SNAPSHOT_EPOCH_COUNT)
        self.conf_parameters["cip151_transition_height"] = str(99999999)

    def setup_network(self):
        self.add_nodes(self.num_nodes)
        self.start_node(FULLNODE0, ["--archive"])
        self.rpc = RpcClient(self.nodes[FULLNODE0])
        self.nodes[FULLNODE0].wait_for_phase(["NormalSyncPhase"])

    def run_test(self):
        n = SNAPSHOT_EPOCH_COUNT

        # Expected boundaries, from the formula alone.
        # height_to_delta_height(h - 1) == n has the solutions h - 1 == k * n
        # with k >= 1, i.e. h == n + 1, 2n + 1, ...
        expected_boundaries = [
            h for h in range(1, 2 * n + 4) if shifts_at(h, n)
        ]
        assert_equal(expected_boundaries, [n + 1, 2 * n + 1])
        self.log.info("expected shift boundaries: %s", expected_boundaries)

        priv_key = default_config["GENESIS_PRI_KEY"]
        sender = eth_utils.encode_hex(priv_to_addr(priv_key))

        bytecode_file = os.path.join(
            os.path.dirname(os.path.realpath(__file__)), CONTRACT_PATH)
        assert os.path.isfile(bytecode_file)
        bytecode = open(bytecode_file).read()
        _, contract_addr = self.deploy_contract(sender, priv_key, bytecode)
        self.call_contract(sender, priv_key, contract_addr,
                           encode_hex_0x(keccak(b"increment()")))

        # Every write to this contract has to land in the first snapshot
        # period, and nowhere else: the layer the contract's storage sits in is
        # what the scans below read the boundary off.
        assert_greater_than(n, self.rpc.epoch_number())

        # First boundary. Scan the whole first period plus a few epochs past
        # it, so "the first epoch whose intermediate layer answers" is a claim
        # about the whole range and not about a window around a guess.
        self.advance_to_state(n + 3)
        roots = self.scan(contract_addr, 1, n + 3)

        frozen = roots[n]["delta"]
        assert_is_hash_string(frozen)

        for h in range(1, n + 1):
            assert_equal(roots[h]["intermediate"], None)
            assert_equal(roots[h]["snapshot"], None)
        for h in range(n + 1, n + 4):
            # the first period's delta is now the intermediate layer, and
            # nothing has written the contract since, so its delta is empty
            assert_equal(roots[h]["intermediate"], frozen)
            assert_equal(roots[h]["delta"], None)
            assert_equal(roots[h]["snapshot"], None)

        first_intermediate = self.first_epoch_with(roots, "intermediate")
        self.log.info("intermediate layer first answers at epoch %d",
                      first_intermediate)
        assert_equal(first_intermediate, n + 1)

        # Second boundary. The intermediate layer of the second period holds no
        # write to this contract, so at the boundary the contract's storage
        # leaves the intermediate layer and appears in the snapshot layer.
        self.advance_to_state(2 * n + 3)
        roots = self.scan(contract_addr, n + 1, 2 * n + 3)

        for h in range(n + 1, 2 * n + 1):
            assert_equal(roots[h]["intermediate"], frozen)
            assert_equal(roots[h]["snapshot"], None)
            assert_equal(roots[h]["delta"], None)
        for h in range(2 * n + 1, 2 * n + 4):
            # the snapshot root differs from the delta root it was built from,
            # the key padding is not the same
            assert_is_hash_string(roots[h]["snapshot"])
            assert_equal(roots[h]["intermediate"], None)
            assert_equal(roots[h]["delta"], None)

        first_snapshot = self.first_epoch_with(roots, "snapshot")
        self.log.info("snapshot layer first answers at epoch %d",
                      first_snapshot)
        assert_equal(first_snapshot, 2 * n + 1)

        # Same two boundaries seen from the file system: a snapshot is taken at
        # the epoch which the shift promotes, and at no epoch next to it. The
        # snapshot of the epoch at height `2n` is only built once the third
        # period's delta is deep enough: `State::commit` asks
        # `check_make_snapshot` only when depth >= snapshot_epoch_count / 3,
        # hence the extra epochs here.
        self.advance_to_state(2 * n + n // 3 + 5)
        for boundary in (n, 2 * n):
            wait_until(lambda: self.snapshot_dir_exists(boundary))
            self.log.info("snapshot directory present for epoch %d", boundary)
            assert not self.snapshot_dir_exists(boundary - 1)
            assert not self.snapshot_dir_exists(boundary + 1)

        self.log.info("Pass")

    def first_epoch_with(self, roots, layer):
        for h in sorted(roots):
            if roots[h][layer] is not None:
                return h
        raise AssertionError("no epoch answers on the {} layer".format(layer))

    def scan(self, contract_addr, start, end):
        """Storage roots of `contract_addr` for every epoch in [start, end]."""
        return {
            h: self.rpc.get_storage_root(contract_addr, epoch=hex(h))
            for h in range(start, end + 1)
        }

    def advance_to_state(self, target_epoch):
        mined = self.rpc.epoch_number()
        if mined < target_epoch + DEFERRED_STATE_EPOCH_COUNT:
            self.rpc.generate_blocks(
                target_epoch + DEFERRED_STATE_EPOCH_COUNT - mined)
        wait_until(lambda: self.rpc.epoch_number(self.rpc.EPOCH_LATEST_STATE)
                   >= target_epoch)

    def snapshot_dir_exists(self, height):
        epoch_hash = self.rpc.block_by_epoch(hex(height))["hash"]
        path = os.path.join(
            self.nodes[FULLNODE0].datadir, "blockchain_data", "storage_db",
            "snapshot", "sqlite_" + epoch_hash[2:])
        return os.path.exists(path)

    def deploy_contract(self, sender, priv_key, data_hex):
        tx = self.rpc.new_contract_tx(receiver="", data_hex=data_hex,
                                      sender=sender, priv_key=priv_key,
                                      storage_limit=20000)
        assert_equal(self.rpc.send_tx(tx, True), tx.hash_hex())
        receipt = self.rpc.get_transaction_receipt(tx.hash_hex())
        address = receipt["contractCreated"]
        assert_is_hex_string(address)
        return receipt, address

    def call_contract(self, sender, priv_key, contract, data_hex):
        tx = self.rpc.new_contract_tx(receiver=contract, data_hex=data_hex,
                                      sender=sender, priv_key=priv_key)
        assert_equal(self.rpc.send_tx(tx, True), tx.hash_hex())
        return self.rpc.get_transaction_receipt(tx.hash_hex())


if __name__ == "__main__":
    StorageShiftBoundaryTest().main()
