#!/usr/bin/env python3
"""Restart from a broken snapshot. What is asserted:

* `plan_recovery` must re-anchor the recomputation start to
  `era_pivot_epoch_height + 2 * snapshot_epoch_count`, the same landing point
  the old `get_force_compute_index` plus `recover_latest_mpt_snapshot_if_needed`
  pair produced;
* the recovery flag the engine now computes per commit must take the same
  value per epoch as the one consensus used to compute per pivot index, and it
  must be cleared exactly at the end of the replay window;
* no transaction may be executed while the handshake runs.

Every non trivial branch of this path is behind `use_isolated_db_for_mpt_table`,
which is off by default, so the configuration below turns it on. With it off
`recover_latest_mpt_snapshot_if_needed` returns on its first statement and the
recovery flag is constantly false, which is the branch every existing crash and
reboot test happens to exercise.
"""

import os
import random
import re
import shutil
import sys

sys.path.insert(1, os.path.dirname(sys.path[0]))

from test_framework.test_framework import ConfluxTestFramework
from test_framework.util import sync_blocks, connect_sample_nodes, assert_equal
from conflux.rpc import RpcClient


SNAPSHOT_EPOCH_COUNT = 100
ERA_EPOCH_COUNT = 500

# Snapshots below this height keep their MPT inside their own database, the
# ones above it use the isolated MPT database. The startup self check reports
# the highest height of the former group as `max_snapshot_epoch_height_has_mpt`,
# so this pins that number at 1000 instead of leaving it None, which is what
# makes the recovery flag take both values during one replay.
MPT_TABLE_ISOLATION_HEIGHT = 1001
MAX_HEIGHT_HAS_MPT = 1000

# The recomputation start consensus proposes to the engine. Pinning it removes
# the dependence on where the "force recompute the last few epochs" scan lands,
# which is what makes the expected re-anchor point below a closed form.
FORCE_RECOMPUTE_HEIGHT = 950

NUM_BLOCKS = 1400

# The rule under test, from
# `StorageManager::recover_latest_mpt_snapshot_if_needed`:
#
#   era_pivot_epoch_height = (start - snapshot_epoch_count - 1)
#                            / era_epoch_count * era_epoch_count
#   if start > era_pivot_epoch_height + snapshot_epoch_count * 2:
#       start = era_pivot_epoch_height + snapshot_epoch_count * 2
#
# with start = 950, snapshot period 100 and era 500:
#   era anchor = (950 - 100 - 1) / 500 * 500 = 849 / 500 * 500 = 500
#   500 + 2 * 100 = 700 < 950, so the start is re-anchored to 700.
EXPECTED_ERA_ANCHOR = (
    (FORCE_RECOMPUTE_HEIGHT - SNAPSHOT_EPOCH_COUNT - 1)
    // ERA_EPOCH_COUNT
    * ERA_EPOCH_COUNT
)
EXPECTED_RECOMPUTE_START = EXPECTED_ERA_ANCHOR + 2 * SNAPSHOT_EPOCH_COUNT

# The rule the engine now applies per commit, from
# `StorageManager::recover_mpt_for_commit`:
#   recover = height > max_height_has_mpt + snapshot_epoch_count
EXPECTED_FLAG_FLIP_HEIGHT = MAX_HEIGHT_HAS_MPT + SNAPSHOT_EPOCH_COUNT

CONSTRUCT_PIVOT_STATE_RE = re.compile(
    r"construct_pivot_state: index (\d+) height (\d+) compute_epoch (\w+)\."
)
RECOVERY_FLAG_RE = re.compile(
    r"compute epoch recovery flag (\w+) at height (\d+)"
)
LEAVE_RECOVERY_RE = re.compile(
    r"leave recovery mode at replay window end height (\d+)"
)
PERSIST_STATE_RE = re.compile(
    r"latest snapshot epoch height: (\d+), temp snapshot status: (\S+), "
    r"max snapshot epoch height has mpt: (\S+),"
)
HANDSHAKE_START_RE = re.compile(r"construct_pivot_state: start=")
PROCESS_TX_RE = re.compile(r"Process tx epoch_id=")


class RestartFromBrokenSnapshotTest(ConfluxTestFramework):
    def set_test_params(self):
        self.num_nodes = 2
        self.conf_parameters = {
            "dev_snapshot_epoch_count": str(SNAPSHOT_EPOCH_COUNT),
            "era_epoch_count": str(ERA_EPOCH_COUNT),
            "adaptive_weight_beta": "1",
            "timer_chain_block_difficulty_ratio": "2",
            "timer_chain_beta": "6",
            "anticone_penalty_ratio": "5",
            "chunk_size_byte": "1000",
            "dev_allow_phase_change_without_peer": "false",
            # Disable pos reference because pow blocks are generated too fast.
            "pos_reference_enable_height": "10000",
            "cip1559_transition_height": "10000",
            # Without this the whole recovery path is short circuited.
            "use_isolated_db_for_mpt_table": "true",
            "use_isolated_db_for_mpt_table_height": str(
                MPT_TABLE_ISOLATION_HEIGHT
            ),
            # Keep every snapshot for the length of this test, so that the
            # inputs of the handshake do not depend on where the confirmation
            # meter happens to be.
            "additional_maintained_snapshot_count": "20",
            # Consensus policy, applied before the handshake: it pins the
            # proposed recomputation start below the newest snapshot, which is
            # the condition under which the engine has to rebuild the latest
            # MPT snapshot.
            "force_recompute_height_during_construct_pivot": str(
                FORCE_RECOMPUTE_HEIGHT
            ),
        }

    def setup_network(self):
        self.add_nodes(self.num_nodes)
        for i in range(self.num_nodes):
            self.start_node(i, phase_to_wait=None)
        connect_sample_nodes(self.nodes, self.log, latency_max=1)
        for i in range(self.num_nodes):
            self.nodes[i].wait_for_recovery(["NormalSyncPhase"], 10)

    def _generate_txs(self, peer, num):
        client = RpcClient(self.nodes[peer])
        txs = []
        for _ in range(num):
            addr = client.rand_addr()
            tx = client.new_tx(
                receiver=addr,
                nonce=self.genesis_nonce,
                value=21000,
                gas=client.DEFAULT_TX_GAS,
                data=b"",
            )
            self.genesis_nonce += 1
            txs.append(tx)
        return txs

    def _snapshot_dir(self, node_index):
        return os.path.join(
            self.nodes[node_index].datadir,
            "blockchain_data",
            "storage_db",
            "snapshot",
        )

    def _snapshot_heights_on_disk(self, node_index, client, max_height):
        """The snapshot heights whose key value database is on disk."""
        snapshot_dir = self._snapshot_dir(node_index)
        heights = []
        for height in range(
            SNAPSHOT_EPOCH_COUNT, max_height + 1, SNAPSHOT_EPOCH_COUNT
        ):
            pivot_hash = client.block_by_epoch(hex(height))["hash"]
            path = os.path.join(snapshot_dir, "sqlite_" + pivot_hash[2:])
            if os.path.isdir(path):
                heights.append(height)
        return heights

    def _read_new_log(self, node_index, offset):
        log_path = os.path.join(self.nodes[node_index].datadir, "conflux.log")
        with open(log_path, "r", encoding="utf8", errors="replace") as f:
            f.seek(offset)
            return f.read().splitlines()

    def run_test(self):
        client0 = RpcClient(self.nodes[0])
        node_index = 1
        client1 = RpcClient(self.nodes[node_index])

        self.genesis_nonce = client0.get_nonce(client0.GENESIS_ADDR)
        block_hashes = []
        for _ in range(NUM_BLOCKS):
            txs = self._generate_txs(0, random.randint(1, 2))
            block_hashes.append(client0.generate_block_with_fake_txs(txs))
        sync_blocks(self.nodes)
        tip_height = client0.epoch_number()
        self.log.info("all nodes synced, tip height %d", tip_height)
        assert_equal(client1.epoch_number(), tip_height)

        snapshot_heights = self._snapshot_heights_on_disk(
            node_index, client0, tip_height
        )
        self.log.info("snapshots on disk before the crash: %s", snapshot_heights)
        # The scenario needs the snapshot which carries `max_height_has_mpt` to
        # be on disk, and it needs it to be lower than the newest one, or the
        # handshake takes the "still not use mpt database" early return.
        assert (
            MAX_HEIGHT_HAS_MPT in snapshot_heights
        ), "snapshot at the mpt table isolation boundary is missing"
        assert (
            max(snapshot_heights) > MAX_HEIGHT_HAS_MPT
        ), "no snapshot above the mpt table isolation boundary"

        log_path = os.path.join(self.nodes[node_index].datadir, "conflux.log")

        # The broken snapshot is made in two steps, both of which a real crash
        # can produce:
        #   1. the node is killed with SIGKILL, so nothing gets a chance to be
        #      shut down in order;
        #   2. the newest key value snapshot directory is removed while its
        #      snapshot info row survives. That is the shape `scan_persist_state`
        #      calls a missing snapshot: the newest snapshot's data did not
        #      survive the crash, its registration did. On the way back up the
        #      engine drops the row and destroys the matching delta database, so
        #      the newest snapshot and every state above it are gone.
        self.stop_node(node_index, kill=True)
        self.nodes[node_index].wait_until_stopped()
        log_offset = os.path.getsize(log_path)

        newest_snapshot_height = max(snapshot_heights)
        newest_snapshot_hash = client0.block_by_epoch(
            hex(newest_snapshot_height)
        )["hash"]
        newest_snapshot_path = os.path.join(
            self._snapshot_dir(node_index), "sqlite_" + newest_snapshot_hash[2:]
        )
        self.log.info(
            "removing the newest snapshot at height %d: %s",
            newest_snapshot_height,
            newest_snapshot_path,
        )
        shutil.rmtree(newest_snapshot_path)

        self.start_node(node_index, phase_to_wait=None)
        self.nodes[node_index].wait_for_phase(["NormalSyncPhase"], wait_time=240)
        self.log.info("node %d back in NormalSyncPhase", node_index)

        lines = self._read_new_log(node_index, log_offset)
        self.log.info("read %d log lines from the restart", len(lines))

        self._check_handshake_inputs(lines)
        replay = self._check_recompute_start(lines)
        self._check_recovery_flags(lines, replay)
        self._check_no_execution_during_handshake(lines)

        # End to end: the replayed states have to agree with the archive node.
        sync_blocks(self.nodes, timeout=120)
        assert_equal(client1.epoch_number(), client0.epoch_number())
        # `EXPECTED_RECOMPUTE_START` is a height, and `block_hashes[i]` is the
        # block of epoch i + 1, so that height sits at index
        # `EXPECTED_RECOMPUTE_START - 1`.
        for block_hash in block_hashes[EXPECTED_RECOMPUTE_START - 1:-8]:
            executed1 = self.nodes[node_index].test_getExecutedInfo(block_hash)
            executed0 = self.nodes[0].test_getExecutedInfo(block_hash)
            assert_equal(executed1, executed0)
        self.log.info("executed info matches the archive node")

    def _check_handshake_inputs(self, lines):
        """The inputs the startup self check handed to the handshake.

        These are not the property under test, they are the preconditions the
        expected values were derived under. Asserting them keeps a changed
        environment from quietly turning the rest of the test into a tautology.
        """
        found = None
        for line in lines:
            m = PERSIST_STATE_RE.search(line)
            if m is not None:
                found = m
                break
        assert found is not None, "the handshake never reported its inputs"
        latest_snapshot_height = int(found.group(1))
        max_height_has_mpt = found.group(3)
        self.log.info(
            "handshake inputs: latest snapshot height %d, "
            "max snapshot epoch height has mpt %s",
            latest_snapshot_height,
            max_height_has_mpt,
        )
        assert_equal(max_height_has_mpt, "Some(%d)" % MAX_HEIGHT_HAS_MPT)
        # `recovery_latest_mpt_snapshot_if_needed` needs the proposed start to
        # sit at or below the newest snapshot for the rebuild to be required,
        # it must not clamp the proposed start, and the era anchor it computes
        # must not sit above the newest snapshot.
        assert FORCE_RECOMPUTE_HEIGHT <= latest_snapshot_height, (
            "the proposed start %d is above the newest snapshot %d, so no "
            "rebuild would be required"
            % (FORCE_RECOMPUTE_HEIGHT, latest_snapshot_height)
        )
        assert (
            FORCE_RECOMPUTE_HEIGHT
            <= latest_snapshot_height + 2 * SNAPSHOT_EPOCH_COUNT
        )
        assert EXPECTED_ERA_ANCHOR <= latest_snapshot_height

    def _check_recompute_start(self, lines):
        """The first height which is really recomputed is the re-anchor point."""
        replay = []
        for line in lines:
            m = CONSTRUCT_PIVOT_STATE_RE.search(line)
            if m is not None:
                replay.append((int(m.group(2)), m.group(3) == "true"))
        assert len(replay) > 0, "construct_pivot_state logged no epoch"

        heights = [h for h, _ in replay]
        assert_equal(heights, list(range(heights[0], heights[0] + len(heights))))

        recomputed = [h for h, compute in replay if compute]
        assert len(recomputed) > 0, "no epoch was recomputed at all"
        self.log.info(
            "construct_pivot_state walked heights %d..%d, recomputed %d..%d",
            heights[0],
            heights[-1],
            recomputed[0],
            recomputed[-1],
        )
        assert_equal(recomputed[0], EXPECTED_RECOMPUTE_START)
        # Everything from the re-anchor point up is recomputed, nothing below.
        assert_equal(
            recomputed, list(range(EXPECTED_RECOMPUTE_START, heights[-1] + 1))
        )
        return recomputed

    def _check_recovery_flags(self, lines, replay):
        """The per commit flag, and the point at which the mode is cleared."""
        flags = []
        cleared = []
        for line in lines:
            m = RECOVERY_FLAG_RE.search(line)
            if m is not None:
                flags.append((int(m.group(2)), m.group(1) == "true"))
                continue
            m = LEAVE_RECOVERY_RE.search(line)
            if m is not None:
                cleared.append(int(m.group(1)))
        assert len(flags) > 0, "the engine never reported a recovery flag"

        flag_heights = [h for h, _ in flags]
        self.log.info(
            "recovery flag reported for heights %d..%d, true from %s",
            flag_heights[0],
            flag_heights[-1],
            next((h for h, f in flags if f), None),
        )
        # One report per replayed epoch, in order, and none outside the replay.
        assert_equal(flag_heights, replay)
        for height, flag in flags:
            assert_equal(
                (height, flag),
                (height, height > EXPECTED_FLAG_FLIP_HEIGHT),
            )

        # The mode is armed once and cleared once, by the commit at the last
        # height of the replay window.
        assert_equal(len(cleared), 1)
        assert_equal(cleared[0], replay[-1])

    def _check_no_execution_during_handshake(self, lines):
        """No transaction is executed while `plan_recovery` runs.

        The window checked here starts where `construct_pivot_state` starts and
        ends at its first per epoch line, so it strictly contains the
        handshake: the handshake is the last thing that happens before that
        first line.
        """
        start = None
        end = None
        for i, line in enumerate(lines):
            if start is None and HANDSHAKE_START_RE.search(line):
                start = i
            if start is not None and CONSTRUCT_PIVOT_STATE_RE.search(line):
                end = i
                break
        assert start is not None, "construct_pivot_state never started"
        assert end is not None, "construct_pivot_state logged no epoch"
        executed = [
            line for line in lines[start:end] if PROCESS_TX_RE.search(line)
        ]
        assert_equal(executed, [])
        self.log.info(
            "no epoch was executed in the %d log lines of the handshake",
            end - start,
        )


if __name__ == "__main__":
    RestartFromBrokenSnapshotTest().main()
