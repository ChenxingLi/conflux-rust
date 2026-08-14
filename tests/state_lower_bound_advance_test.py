#!/usr/bin/env python3
"""Watch the state availability lower bound follow repeated snapshot garbage
collection.

`StateAvailabilityBoundary` keeps a mirror of pivot chain hashes indexed from
its `lower_bound`. Trimming the mirror and moving that bound happen in the same
statement, and the value both come from is the garbage collection watermark the
storage engine reports back from `notify_state_confirmed`. The call that applies
it lives on the consensus side and has no compiler check behind it, so a lost
call shows up only as a mirror that grows and never shrinks.

The bound itself is observable in two independent places, and this test checks
both against the same run:

* the RPC error text for a read below the bound, which carries the whole
  `StateAvailabilityBoundary` debug output including `lower_bound`;
* the node log, where each maintenance round prints the watermark it computed as
  `first_available_state_height`.

Seeing the bound take at least three strictly increasing values is what
distinguishes repeated trimming from a single one-off advance.
"""

import os
import re

from conflux.config import default_config
from conflux.rpc import RpcClient
from conflux.utils import encode_hex, priv_to_addr
from test_framework.simple_rpc_proxy import ReceivedErrorResponseError
from test_framework.test_framework import ConfluxTestFramework
from test_framework.util import assert_equal

ERA_EPOCH_COUNT = 50
SNAPSHOT_EPOCH_COUNT = ERA_EPOCH_COUNT // 2

# Blocks per batch, and the number of batches. The bound moves one snapshot
# period at a time, so this has to cover several periods with room to spare for
# the lag between generating a block and the epoch being confirmed.
BATCH_SIZE = 50
BATCH_COUNT = 14

# The epoch read to provoke the out-of-bound error. Any epoch below the bound
# does, and epoch 1 stays below it for the whole run.
PROBE_EPOCH = 1

# Distinct values the bound has to take before the run counts as having seen
# repeated collection rather than a single advance.
MIN_DISTINCT_BOUNDS = 3

LOWER_BOUND_PATTERN = re.compile(r"lower_bound: (\d+)")
WATERMARK_PATTERN = re.compile(r"first_available_state_height (\d+)")
# A round that failed leaves the watermark where it was, which looks exactly
# like a mirror that is not shrinking.
MAINTENANCE_FAILURE_MARKER = "maintenance failed"


class StateLowerBoundAdvanceTest(ConfluxTestFramework):
    def set_test_params(self):
        self.num_nodes = 1
        self.conf_parameters["adaptive_weight_beta"] = "1"
        self.conf_parameters["timer_chain_block_difficulty_ratio"] = "3"
        self.conf_parameters["timer_chain_beta"] = "10"
        self.conf_parameters["era_epoch_count"] = str(ERA_EPOCH_COUNT)
        self.conf_parameters["dev_snapshot_epoch_count"] = str(
            SNAPSHOT_EPOCH_COUNT
        )
        # Nothing kept beyond what the retention rules require, so collection
        # runs on every snapshot period.
        self.conf_parameters["additional_maintained_snapshot_count"] = "0"
        # The single MPT form keeps a full history copy on the side, which
        # answers reads below the bound and hides the error this test reads the
        # bound from.
        self.conf_parameters["enable_single_mpt_storage"] = "false"
        self.conf_parameters["node_type"] = '"archive"'
        self.rpc_timewait = 120

    def setup_network(self):
        self.setup_nodes()

    def run_test(self):
        client = RpcClient(self.nodes[0])
        genesis_address = "0x" + encode_hex(
            priv_to_addr(default_config["GENESIS_PRI_KEY"])
        )

        rpc_bounds = []
        for batch in range(BATCH_COUNT):
            client.generate_empty_blocks(BATCH_SIZE)
            bound = self.read_lower_bound(client, genesis_address)
            self.log.info(
                "batch %d: latest_state=%d latest_checkpoint=%d lower_bound=%s",
                batch,
                client.epoch_number("latest_state"),
                client.epoch_number("latest_checkpoint"),
                bound,
            )
            if bound is not None:
                rpc_bounds.append(bound)

        self.log.info("lower_bound values read over RPC: %s", rpc_bounds)
        rpc_distinct = self.rising_values(
            rpc_bounds, "RPC lower_bound", never_decreasing=True
        )
        self.log.info("distinct RPC lower_bound values: %s", rpc_distinct)
        assert len(rpc_distinct) >= MIN_DISTINCT_BOUNDS, (
            f"lower_bound took only {len(rpc_distinct)} distinct values "
            f"{rpc_distinct} over {BATCH_COUNT * BATCH_SIZE} blocks; the pivot "
            "hash mirror was trimmed at most that many times"
        )

        log_text = self.read_node_log()
        assert MAINTENANCE_FAILURE_MARKER not in log_text, (
            "a maintenance round failed; the watermark then stays where it was "
            "and the mirror does not shrink, which is the same symptom this "
            "test is looking for"
        )

        watermarks = [
            int(value) for value in WATERMARK_PATTERN.findall(log_text)
        ]
        assert watermarks, (
            "no first_available_state_height line in the node log; either no "
            "maintenance round ran or the log level dropped the line"
        )
        # The logged watermark is recomputed from the confirmed height on every
        # round, so it is only the record kept behind it that is pinned to move
        # one way. Take the running maxima here and leave the never-decreasing
        # requirement to the bound read over RPC.
        watermark_distinct = self.rising_values(
            watermarks, "logged first_available_state_height"
        )
        self.log.info(
            "distinct logged watermarks: %s", watermark_distinct
        )
        assert len(watermark_distinct) >= MIN_DISTINCT_BOUNDS, (
            f"the logged watermark took only {len(watermark_distinct)} "
            f"distinct values {watermark_distinct}"
        )

        # The two observations are the same number seen at two places: every
        # bound the RPC reported was computed by some maintenance round and
        # logged there.
        missing = [
            bound for bound in rpc_distinct if bound not in watermark_distinct
        ]
        assert_equal(missing, [])

        # The mirror is indexed from the bound, so a trim that dropped the
        # wrong prefix would leave every entry off by that much and the hash
        # comparison right above the bound would fail. Reading there is what
        # tells a correct trim from a merely advanced bound.
        #
        # The bound is read again rather than taken from the last sample: block
        # generation has stopped, so this is where it stays.
        last_bound = self.read_lower_bound(client, genesis_address)
        assert last_bound is not None
        balance = client.get_balance(
            genesis_address, client.EPOCH_NUM(last_bound + 1)
        )
        self.log.info(
            "balance readable at epoch %d, just above the bound: %d",
            last_bound + 1,
            balance,
        )
        assert balance > 0

        self.log.info("Pass")

    def read_lower_bound(self, client, address):
        """Read `lower_bound` out of the out-of-bound RPC error, or None while
        the bound is still at or below the probed epoch."""
        try:
            client.get_balance(address, client.EPOCH_NUM(PROBE_EPOCH))
        except ReceivedErrorResponseError as e:
            error = e.response
            assert_equal(error.code, -32016)
            text = "{} {}".format(
                error.message, getattr(error, "data", None) or ""
            )
            assert "out-of-bound" in text, (
                f"unexpected error for epoch {PROBE_EPOCH}: {text}"
            )
            match = LOWER_BOUND_PATTERN.search(text)
            assert match is not None, (
                f"no lower_bound in the out-of-bound error text: {text}"
            )
            return int(match.group(1))
        return None

    def read_node_log(self):
        with open(
            os.path.join(self.nodes[0].datadir, "conflux.log"), "r"
        ) as f:
            return f.read()

    def rising_values(self, values, what, never_decreasing=False):
        """Collect the running maxima of a series, that is the values at which
        it rose. With `never_decreasing`, a series that goes down at any point
        fails instead."""
        rising = []
        for value in values:
            if rising and value < rising[-1]:
                if never_decreasing:
                    raise AssertionError(f"{what} went backwards: {values}")
                continue
            if not rising or value > rising[-1]:
                rising.append(value)
        return rising


if __name__ == "__main__":
    StateLowerBoundAdvanceTest().main()
