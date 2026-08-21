#!/usr/bin/env python3
"""Rebuild the engine's state index on a garbage collected disk and check
that every height the rebuild claims is still openable answers the same as an
archive node which never collected anything.

The rebuild runs once, on the first boot of a binary carrying the index, and
has to reconstruct each version's three merkle roots from what the node still
has. An earlier form of it walked the snapshot periods upwards and read the
intermediate layer's root out of a delta MPT three periods below, a database
garbage collection destroys as the retention window moves. A miss there is not
an error: the empty root is a legal value, and the key paddings derived from it
address the delta MPT at legal looking but wrong keys. The result is a version
which opens and answers with the state of the previous snapshot moment, for a
whole snapshot period of heights, with nothing in the log to say so.

So the check cannot be "the rebuild finished" or even "the index has entries";
it has to be the answers themselves, height by height, against a node which
still holds the full history. The probed account receives a transfer in nearly
every epoch, so its balance differs at every height, and an answer carried over
from the previous snapshot moment is a different number rather than the same
one.

Three nodes run on one chain: an archive reference which never collects, and
two nodes which do. The two collecting nodes differ only in
`provide_more_snapshot_for_sync`, which decides what garbage collection leaves
in the snapshot registry below the retention window, and with it the shape the
old defect took:

* `checkpoint`, the default, keeps the registry rows of the collected snapshots
  as info-only ones, so the registry still names every snapshot layer below the
  window even though the data behind them is gone. The old rebuild found those
  rows, wrote an entry for every height, and filled the roots it could not read
  with the empty root.
* empty keeps nothing, so those rows are removed outright and the layer below
  the window has no name left. The old rebuild then wrote no entry at all for
  the periods that name it, and published a bound below them all the same, so
  the lowest stretch of heights was claimed openable with nothing to open it
  with.

Both are subjected to the same four checks: the published bound is the one the
documented rule gives, which is the lower end of the snapshot run on disk
raised past any stretch of heights the rebuild wrote no entry for, every height
from the bound up answers exactly as the archive does, every height below it is
refused rather than answered, and the rebuild log carries no commitment row
miss and no height left out at or above the bound. Each check records what it saw instead
of ending the run, so one boot is judged on all four at once.
"""

import os
import re
import shutil
import sqlite3

import rlp

from conflux.config import default_config
from conflux.rpc import RpcClient
from conflux.utils import encode_hex, priv_to_addr
from test_framework.simple_rpc_proxy import ReceivedErrorResponseError
from test_framework.test_framework import ConfluxTestFramework
from test_framework.util import (
    assert_equal,
    connect_nodes,
    sync_blocks,
    wait_until,
)

# The era has to be long compared with the lag between the tip and the
# confirmed height, which is what decides how far above genesis garbage
# collection stops. A restart rebuilds the state availability boundary from the
# stable era genesis and only ever raises it, so an era short enough to leave
# the stable genesis above the collected disk's own bound would gate away
# exactly the heights this test reads. `check_migrated_node` asserts the
# relationship came out right rather than trusting it.
ERA_EPOCH_COUNT = 500
SNAPSHOT_EPOCH_COUNT = 25

# One transfer per epoch, in batches of one snapshot period, for as long as it
# takes garbage collection to run the rounds asked for below. Transfers keep
# going to the end of the run rather than stopping early, because the heights
# this test probes are the ones just below the tip.
CHUNK = SNAPSHOT_EPOCH_COUNT
MAX_CHUNKS = 120
TRANSFER_VALUE = 10 ** 12

# Garbage collection rounds the run has to have seen. Each round drops one
# snapshot period, so this is also how many periods sit below the retention
# window by the time the run stops.
MIN_COLLECTION_ROUNDS = 4

# How far the stable checkpoint has to have moved before the run stops. The
# snapshots kept to serve sync sit one era apart at and below it, so this is
# what decides whether the retention window has climbed far enough above them
# for the two configurations to leave the registry in two different states at
# all. Two eras.
MIN_CHECKPOINT_HEIGHT = 2 * ERA_EPOCH_COUNT

# The genesis account, whose balance no transfer of this test changes. Reading
# it is how the state availability lower bound is asked for.
GENESIS_ADDRESS = "0x" + encode_hex(
    priv_to_addr(default_config["GENESIS_PRI_KEY"])
)

# Once collection has been frozen for the restart, this is how many snapshot
# periods it would take to reach the retention window again, i.e. never.
FROZEN_SNAPSHOT_COUNT = 100000

REFERENCE = 0
KEEP_CHECKPOINT_SNAPSHOTS = 1
KEEP_NO_SNAPSHOTS = 2

LOWER_BOUND_PATTERN = re.compile(r"lower_bound: (\d+)")
WATERMARK_PATTERN = re.compile(r"first_available_state_height (\d+)")
REBUILD_SUMMARY_PATTERN = re.compile(
    r"state index rebuild: (\d+) entries written, physical openable lower "
    r"bound (\d+); (\d+) periods and (\d+) single heights left out"
)
SKIPPED_RANGE_PATTERN = re.compile(
    r"Writing no entry for heights (\d+)\.\.=(\d+)"
)
# Warnings which say the rebuild could not read what it needed from the
# commitment rows, or read something which contradicts the snapshot registry.
# Neither is a normal outcome on a healthy disk: the commitment rows live in
# the ledger db, which garbage collection does not touch.
COMMITMENT_MISS_MARKER = "no commitment row for"
ROOT_DISAGREEMENT_MARKER = "carries snapshot root"
MISSING_PIVOT_MARKER = "names no epoch at height"

# The status byte of a snapshot registry row. Only `No` snapshots count as
# executable, the other two are kept below the retention window to serve sync.
KEPT_NO = 0

SNAPSHOT_KEPT_STATUS_NAMES = {0: "No", 1: "InfoOnly", 2: "InfoAndSnapshot"}


def snapshot_lower_bound_of(executable_heights, snapshot_epoch_count):
    """The lowest height whose snapshot data is still on disk, from the heights
    of the executable snapshots alone.

    Written out here from the rule rather than read back off the run: walk down
    from the newest snapshot while each neighbour is exactly one snapshot
    period below, and take the lowest height the walk reaches. That height
    counts, because the snapshot holds the whole state of its own epoch. A
    walk which reaches height 0 has nothing below it and gives 0.
    """
    if not executable_heights:
        return 0
    lowest = executable_heights[-1]
    for height in reversed(executable_heights[:-1]):
        if height + snapshot_epoch_count != lowest:
            break
        lowest = height
    return lowest


def published_lower_bound_of(snapshot_lower_bound, skipped_ranges):
    """The lowest height the rebuild may claim openable.

    Snapshot data on disk and an entry to open it with are both necessary and
    neither implies the other, so the bound is the higher of the two lower
    ends. The second one is the lowest height the rebuild wrote an entry for,
    which is the snapshot lower bound raised past every stretch of heights the
    rebuild reported writing no entry for, starting at that bound.

    A stretch reported as left out runs up to the height of the snapshot which
    ends that period, and that height gets an entry anyway, written from the
    snapshot itself. The stretch therefore raises the bound to its top rather
    than past it.
    """
    bound = snapshot_lower_bound
    raised = True
    while raised:
        raised = False
        for begin, end in skipped_ranges:
            if begin <= bound < end:
                bound = end
                raised = True
    return bound


class StateIndexMigrationTest(ConfluxTestFramework):
    def set_test_params(self):
        self.num_nodes = 3
        self.conf_parameters["adaptive_weight_beta"] = "1"
        self.conf_parameters["timer_chain_block_difficulty_ratio"] = "3"
        self.conf_parameters["timer_chain_beta"] = "10"
        self.conf_parameters["era_epoch_count"] = str(ERA_EPOCH_COUNT)
        self.conf_parameters["dev_snapshot_epoch_count"] = str(
            SNAPSHOT_EPOCH_COUNT
        )
        # The collecting nodes keep nothing beyond the retention rules; the
        # reference node's value is raised in `setup_network`.
        self.conf_parameters["additional_maintained_snapshot_count"] = "0"
        # The single MPT form keeps a full history copy on the side which
        # answers reads the layered state cannot, which would answer for every
        # height this test expects the layered index to answer for.
        self.conf_parameters["enable_single_mpt_storage"] = "false"
        self.conf_parameters["node_type"] = '"archive"'
        self.rpc_timewait = 120

    def setup_network(self):
        self.add_nodes(self.num_nodes)
        # A reference which never collects: the retention window it would keep
        # reaches below genesis, so every maintenance round returns before
        # removing anything.
        self.set_conf(
            REFERENCE,
            "additional_maintained_snapshot_count",
            str(FROZEN_SNAPSHOT_COUNT),
        )
        # The two shapes the registry can be left in below the window.
        self.set_conf(
            KEEP_CHECKPOINT_SNAPSHOTS,
            "provide_more_snapshot_for_sync",
            '"checkpoint"',
        )
        self.set_conf(
            KEEP_NO_SNAPSHOTS, "provide_more_snapshot_for_sync", '""'
        )
        # The garbage collector reads keep_snapshot_before_stable_checkpoint
        # only in the branch taken for a checkpoint entry of
        # provide_more_snapshot_for_sync. With that list empty the switch
        # would silently do nothing, which is why startup refuses to run with
        # the switch left at its default of true.
        self.set_conf(
            KEEP_NO_SNAPSHOTS, "keep_snapshot_before_stable_checkpoint", "false"
        )
        self.start_nodes()
        for i in range(self.num_nodes - 1):
            connect_nodes(self.nodes, i, i + 1)
        sync_blocks(self.nodes)

    # ---------------- node and disk helpers ----------------

    def conf_path(self, index):
        return os.path.join(self.nodes[index].datadir, "conflux.conf")

    def set_conf(self, index, key, value):
        """Rewrite one key of one node's configuration file. The files are
        written from a single dict shared by all nodes, so per node
        configuration has to be patched in after that and before the node
        starts."""
        path = self.conf_path(index)
        with open(path, encoding="utf8") as f:
            lines = f.read().splitlines()
        replaced = False
        for i, line in enumerate(lines):
            if line.split("=", 1)[0].strip() == key:
                lines[i] = "{}={}".format(key, value)
                replaced = True
        if not replaced:
            lines.append("{}={}".format(key, value))
        with open(path, "w", encoding="utf8") as f:
            f.write("\n".join(lines) + "\n")

    def storage_dir(self, index):
        return os.path.join(
            self.nodes[index].datadir, "blockchain_data", "storage_db"
        )

    def index_path(self, index):
        return os.path.join(self.storage_dir(index), "state_index_db")

    def registry_path(self, index):
        return os.path.join(self.storage_dir(index), "snapshot_info_db")

    def log_size(self, index):
        return os.path.getsize(
            os.path.join(self.nodes[index].datadir, "conflux.log")
        )

    def read_log(self, index, offset=0):
        with open(
            os.path.join(self.nodes[index].datadir, "conflux.log"),
            encoding="utf8",
            errors="replace",
        ) as f:
            f.seek(offset)
            return f.read()

    def read_snapshot_registry(self, index):
        """Every row of a stopped node's snapshot registry, as
        (height, kept status). The file is copied first: it is a write ahead
        log database, and the reader has to be allowed to replay the log."""
        work_dir = os.path.join(
            self.options.tmpdir, "registry_node{}".format(index)
        )
        os.makedirs(work_dir, exist_ok=True)
        source = self.registry_path(index)
        target = os.path.join(work_dir, "snapshot_info_db")
        for suffix in ("", "-wal", "-shm"):
            if os.path.exists(source + suffix):
                shutil.copyfile(source + suffix, target + suffix)
        connection = sqlite3.connect(target)
        rows = connection.execute(
            "SELECT key, value FROM snapshot_info"
        ).fetchall()
        connection.close()
        registry = []
        for _key, value in rows:
            fields = rlp.decode(bytes(value))
            status = int.from_bytes(fields[0], "big") if fields[0] else 0
            height = int.from_bytes(fields[4], "big") if fields[4] else 0
            registry.append((height, status))
        registry.sort()
        return registry

    def collection_rounds(self, index):
        """The distinct watermarks garbage collection has computed so far, in
        the order it reached them."""
        watermarks = []
        for value in WATERMARK_PATTERN.findall(self.read_log(index)):
            value = int(value)
            if value > 0 and (not watermarks or value > watermarks[-1]):
                watermarks.append(value)
        return watermarks

    def read_lower_bound(self, client, address):
        """The state availability lower bound, read out of the error a read
        below it produces, or None while the bound is still at genesis."""
        try:
            client.get_balance(address, client.EPOCH_NUM(1))
        except ReceivedErrorResponseError as e:
            error = e.response
            text = "{} {}".format(
                error.message, getattr(error, "data", None) or ""
            )
            match = LOWER_BOUND_PATTERN.search(text)
            assert match is not None, (
                "no lower_bound in the out-of-bound error text: " + text
            )
            return int(match.group(1))
        return None

    def balances(self, client, address, heights):
        """The balance at each height, or the string `refused` where the node
        answered with an error instead of a number."""
        out = {}
        for height in heights:
            try:
                out[height] = client.get_balance(
                    address, client.EPOCH_NUM(height)
                )
            except ReceivedErrorResponseError:
                out[height] = "refused"
        return out

    # ---------------- the run ----------------

    def build_chain(self, client, receiver):
        """Grow the chain, one transfer per epoch, until garbage collection has
        run its rounds on both collecting nodes."""
        nonce = client.get_nonce(client.GENESIS_ADDR)
        rounds = {KEEP_CHECKPOINT_SNAPSHOTS: [], KEEP_NO_SNAPSHOTS: []}
        for chunk in range(MAX_CHUNKS):
            for _ in range(CHUNK):
                client.send_tx(
                    client.new_tx(
                        receiver=receiver,
                        nonce=nonce,
                        value=TRANSFER_VALUE,
                        gas=client.DEFAULT_TX_GAS,
                        data=b"",
                    )
                )
                nonce += 1
            client.generate_blocks(CHUNK, num_txs=2)
            sync_blocks(self.nodes, timeout=120)
            for index in rounds:
                rounds[index] = self.collection_rounds(index)
            checkpoint = client.epoch_number("latest_checkpoint")
            if chunk % 4 == 0:
                self.log.info(
                    "chunk %d: latest_state=%d latest_checkpoint=%d "
                    "collection rounds %s / %s",
                    chunk,
                    client.epoch_number("latest_state"),
                    checkpoint,
                    len(rounds[KEEP_CHECKPOINT_SNAPSHOTS]),
                    len(rounds[KEEP_NO_SNAPSHOTS]),
                )
            enough_rounds = all(
                len(values) >= MIN_COLLECTION_ROUNDS
                for values in rounds.values()
            )
            if enough_rounds and checkpoint >= MIN_CHECKPOINT_HEIGHT:
                break
        self.log.info(
            "collection watermarks, checkpoint keeper node: %s",
            rounds[KEEP_CHECKPOINT_SNAPSHOTS],
        )
        self.log.info(
            "collection watermarks, no keeper node: %s",
            rounds[KEEP_NO_SNAPSHOTS],
        )
        for index, values in rounds.items():
            assert len(values) >= MIN_COLLECTION_ROUNDS, (
                "node {} saw only {} garbage collection rounds {}; the disk "
                "the rebuild is supposed to face was never produced".format(
                    index, len(values), values
                )
            )
        assert (
            client.epoch_number("latest_checkpoint") >= MIN_CHECKPOINT_HEIGHT
        ), (
            "the stable checkpoint only reached {} in {} blocks, so the "
            "retention window never climbed past the snapshots kept to serve "
            "sync and the two configurations under test face the same "
            "disk".format(
                client.epoch_number("latest_checkpoint"), MAX_CHUNKS * CHUNK
            )
        )

    def wipe_index(self, index):
        """What an upgrade looks like from the engine's side: the private index
        does not exist yet. The index is a rocksdb, so it is a directory."""
        path = self.index_path(index)
        assert os.path.isdir(path), (
            "no state index at {}; the first boot path this test exercises "
            "would not be taken".format(path)
        )
        shutil.rmtree(path)
        self.log.info("node %d: removed %s", index, path)

    def restart_migrating(self, index, receiver):
        """Freeze collection, drop the index, and boot. Returns the snapshot
        registry as it stood at the stop, which is what the rebuild reads, and
        the part of the log the boot produced.

        Before stopping, the node is held to the same height by height
        comparison the rebuilt node is held to afterwards. The index it has at
        that point was written commit by commit, so a difference there is the
        run failing to set the situation up rather than the rebuild getting
        anything wrong, and a difference afterwards is the rebuild's alone."""
        client = RpcClient(self.nodes[index])
        reference_client = RpcClient(self.nodes[REFERENCE])
        bound = self.read_lower_bound(client, GENESIS_ADDRESS)
        assert bound is not None, (
            "node {} still serves every height, so garbage collection left it "
            "nothing to rebuild from a collected disk".format(index)
        )
        tip = min(
            client.epoch_number("latest_state"),
            reference_client.epoch_number("latest_state"),
        )
        heights = list(range(bound, tip + 1))
        reference = self.balances(reference_client, receiver, heights)
        answers = self.balances(client, receiver, heights)
        mismatches = [
            (height, reference[height], answers[height])
            for height in heights
            if reference[height] != answers[height]
        ]
        assert_equal(mismatches, [])
        self.log.info(
            "node %d before the restart: bound %d, heights %d..%d all answer "
            "as the archive does",
            index,
            bound,
            bound,
            tip,
        )

        self.stop_node(index)
        self.nodes[index].wait_until_stopped()
        registry = self.read_snapshot_registry(index)
        self.wipe_index(index)
        # Collection must not move the bound while the assertions below read
        # heights relative to it.
        self.set_conf(
            index,
            "additional_maintained_snapshot_count",
            str(FROZEN_SNAPSHOT_COUNT),
        )
        offset = self.log_size(index)
        self.start_node(index, phase_to_wait=None)
        self.nodes[index].wait_for_recovery(["NormalSyncPhase"], 240)
        # The boot replays the segment of the chain the rebuild leaves
        # uncovered, so the node is behind the reference until that is done and
        # the heights compared below would otherwise be cut short.
        reference_tip = reference_client.epoch_number("latest_state")
        wait_until(
            lambda: client.epoch_number("latest_state") >= reference_tip,
            timeout=300,
        )
        return registry, self.read_log(index, offset)

    def check_migrated_node(
        self, index, label, registry, log_text, receiver, expect_kept_rows
    ):
        """The four checks, against one node which has just rebuilt its index.

        The four are independent observations of the same boot, so a failing
        one records what it saw and the rest still run; the collected list is
        what the test fails on. Statements about the run having produced the
        situation under test at all are asserted on the spot instead, because
        every check below reads them.
        """
        self.log.info("=== %s (node %d) ===", label, index)
        problems = []

        executable = [h for h, status in registry if status == KEPT_NO]
        by_status = {}
        for height, status in registry:
            by_status.setdefault(status, []).append(height)
        self.log.info(
            "snapshot registry on disk at the restart: %s",
            {
                SNAPSHOT_KEPT_STATUS_NAMES[status]: heights
                for status, heights in sorted(by_status.items())
            },
        )
        snapshot_bound = snapshot_lower_bound_of(
            executable, SNAPSHOT_EPOCH_COUNT
        )
        self.log.info(
            "executable snapshot heights %s give a snapshot lower bound of %d",
            executable,
            snapshot_bound,
        )
        assert snapshot_bound > 0, (
            "the executable snapshots run down to genesis, so the rebuild has "
            "no bound to publish and nothing below it to refuse"
        )
        # The two configurations are worth running separately only if garbage
        # collection really did leave the registry in two different states.
        kept_rows = [
            (height, SNAPSHOT_KEPT_STATUS_NAMES[status])
            for height, status in registry
            if status != KEPT_NO
        ]
        if expect_kept_rows:
            assert kept_rows, (
                "no snapshot registry row was kept to serve sync, so this "
                "node's disk is the same shape as the one the other "
                "configuration produces"
            )
        else:
            assert not kept_rows, (
                "rows {} were kept to serve sync although nothing was "
                "configured to keep them".format(kept_rows)
            )

        summary = REBUILD_SUMMARY_PATTERN.search(log_text)
        assert summary is not None, (
            "no state index rebuild summary in the boot log; the first boot "
            "path was not taken"
        )
        written, published_bound, skipped_periods, skipped_heights = (
            int(group) for group in summary.groups()
        )
        self.log.info(
            "rebuild wrote %d entries, published bound %d, left out %d "
            "periods and %d single heights",
            written,
            published_bound,
            skipped_periods,
            skipped_heights,
        )

        skipped_ranges = [
            (int(begin), int(end))
            for begin, end in SKIPPED_RANGE_PATTERN.findall(log_text)
        ]
        self.log.info("height ranges left out: %s", skipped_ranges)

        # Check one: the published bound is the one the rule gives.
        expected_bound = published_lower_bound_of(
            snapshot_bound, skipped_ranges
        )
        self.log.info(
            "snapshot lower bound %d, raised past the stretches left out to "
            "an expected published bound of %d",
            snapshot_bound,
            expected_bound,
        )
        if published_bound != expected_bound:
            problems.append(
                "the rebuild published bound {} where the executable "
                "snapshots {} and the height ranges left out {} give "
                "{}".format(
                    published_bound, executable, skipped_ranges, expected_bound
                )
            )

        # The heights the rest of this reads are the ones just above the
        # published bound, so the boot must have taken that bound as the state
        # availability lower bound. It only ever raises the bound it rebuilt
        # from the stable era genesis, so an era short enough to leave the
        # stable genesis above the collected disk's bound gates those heights
        # away and leaves nothing to read.
        reference_client = RpcClient(self.nodes[REFERENCE])
        client = RpcClient(self.nodes[index])
        serving_bound = self.read_lower_bound(client, GENESIS_ADDRESS)
        assert serving_bound == published_bound, (
            "the node serves from height {} while the rebuild published {}; "
            "with era_epoch_count {} the stable era genesis ended up above "
            "the height garbage collection had reached, so the heights this "
            "test reads are gated away for a reason that has nothing to do "
            "with the rebuild".format(
                serving_bound, published_bound, ERA_EPOCH_COUNT
            )
        )

        # Check two: the rebuild read every commitment row it asked for, and
        # every height it did leave out lies below the bound it published.
        if COMMITMENT_MISS_MARKER in log_text:
            problems.append(
                "the rebuild could not read a commitment row; the roots of "
                "the affected heights come from the ledger db, which garbage "
                "collection does not touch, so this is not a normal outcome"
            )
        if ROOT_DISAGREEMENT_MARKER in log_text:
            problems.append(
                "a commitment row disagreed with the snapshot registry about "
                "the merkle root of the snapshot layer"
            )
        if MISSING_PIVOT_MARKER in log_text:
            problems.append(
                "a snapshot registry row gave no epoch at a height inside "
                "its own period"
            )
        # The top of a stretch left out is a snapshot height, which gets an
        # entry of its own; only the heights below that top are uncovered.
        above_bound = [
            (begin, end)
            for begin, end in skipped_ranges
            if end - 1 >= published_bound
        ]
        if above_bound:
            problems.append(
                "the rebuild wrote no entry for heights {} while publishing "
                "{} as the lowest openable height, so those heights are "
                "claimed openable and have no entry to open them "
                "with".format(above_bound, published_bound)
            )

        # Check three: from the bound to the tip, the same answers as the
        # archive.
        tip = min(
            client.epoch_number("latest_state"),
            reference_client.epoch_number("latest_state"),
        )
        assert tip > published_bound, (
            "the tip {} is not above the published bound {}".format(
                tip, published_bound
            )
        )
        heights = list(range(published_bound, tip + 1))
        reference = self.balances(reference_client, receiver, heights)
        refused_by_reference = [
            h for h in heights if reference[h] == "refused"
        ]
        assert_equal(refused_by_reference, [])
        self.log.info(
            "archive answers for all %d heights %d..%d",
            len(heights),
            heights[0],
            tip,
        )

        # The probe only distinguishes a correct answer from one carried over
        # from the previous snapshot moment if the two are different numbers.
        # The carried over answer seen when the defect was live was the balance
        # at the height just below the bound.
        stale_candidate = reference_client.get_balance(
            receiver, reference_client.EPOCH_NUM(published_bound - 1)
        )
        first_period = [
            h for h in heights if h < published_bound + SNAPSHOT_EPOCH_COUNT
        ]
        discriminating = [
            h for h in first_period if reference[h] != stale_candidate
        ]
        self.log.info(
            "balance at height %d, the one a stale answer would repeat: %d; "
            "%d of the %d heights of the first period above the bound differ "
            "from it",
            published_bound - 1,
            stale_candidate,
            len(discriminating),
            len(first_period),
        )
        assert len(discriminating) >= len(first_period) - 2, (
            "the probed balance barely moves over the period above the bound, "
            "so an answer carried over from height {} would look like the "
            "right one".format(published_bound - 1)
        )

        answers = self.balances(client, receiver, heights)
        mismatches = [
            (h, reference[h], answers[h])
            for h in heights
            if reference[h] != answers[h]
        ]
        if mismatches:
            self.log.info(
                "heights whose answer differs from the archive (first 40 of "
                "%d), as (height, archive, this node): %s",
                len(mismatches),
                mismatches[:40],
            )
            problems.append(
                "{} of the {} heights from the published bound to the tip "
                "answer differently from the archive, first {}".format(
                    len(mismatches), len(heights), mismatches[:8]
                )
            )
        else:
            self.log.info(
                "heights %d..%d all answer as the archive does",
                heights[0],
                tip,
            )

        # Check four: below the bound the node refuses rather than answers.
        below = [1] + list(
            range(max(1, published_bound - SNAPSHOT_EPOCH_COUNT),
                  published_bound)
        )
        below_answers = self.balances(client, receiver, below)
        answered_below = [
            (h, below_answers[h])
            for h in below
            if below_answers[h] != "refused"
        ]
        if answered_below:
            problems.append(
                "heights below the published bound {} were answered rather "
                "than refused, as (height, answer): {}".format(
                    published_bound, answered_below[:8]
                )
            )
        reference_below = self.balances(reference_client, receiver, below)
        refused_by_reference_below = [
            h for h in below if reference_below[h] == "refused"
        ]
        assert_equal(refused_by_reference_below, [])
        self.log.info(
            "heights %s are refused here and answered by the archive",
            below,
        )

        assert not problems, "{} (node {}):\n  {}".format(
            label, index, "\n  ".join(problems)
        )

    def run_test(self):
        reference_client = RpcClient(self.nodes[REFERENCE])
        receiver = reference_client.rand_addr()
        self.log.info("probed account: %s", receiver)

        self.build_chain(reference_client, receiver)
        tip = reference_client.epoch_number("latest_state")
        self.log.info(
            "chain built to latest_state=%d; bounds now: reference=%s, "
            "checkpoint keeper=%s, no keeper=%s",
            tip,
            self.read_lower_bound(reference_client, GENESIS_ADDRESS),
            self.read_lower_bound(
                RpcClient(self.nodes[KEEP_CHECKPOINT_SNAPSHOTS]),
                GENESIS_ADDRESS,
            ),
            self.read_lower_bound(
                RpcClient(self.nodes[KEEP_NO_SNAPSHOTS]), GENESIS_ADDRESS
            ),
        )
        assert (
            self.read_lower_bound(reference_client, GENESIS_ADDRESS) is None
        ), (
            "the reference node collected states; it cannot serve as the "
            "archive this test compares against"
        )

        for index, label, expect_kept_rows in (
            (
                KEEP_CHECKPOINT_SNAPSHOTS,
                "provide_more_snapshot_for_sync = checkpoint",
                True,
            ),
            (
                KEEP_NO_SNAPSHOTS,
                "provide_more_snapshot_for_sync = empty",
                False,
            ),
        ):
            registry, log_text = self.restart_migrating(index, receiver)
            self.check_migrated_node(
                index, label, registry, log_text, receiver, expect_kept_rows
            )

        self.log.info("Pass")


if __name__ == "__main__":
    StateIndexMigrationTest().main()
