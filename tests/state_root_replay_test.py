#!/usr/bin/env python3
"""Replay a recorded chain and compare the consensus visible state root of every
epoch against the roots the recording produced.

Storage layout entries now reach the storage through the cache, replayed with
everything else, instead of through a second write path that bypassed it.  That
is equivalent to the bypassing write only if a later write of a key overrides an
earlier one, so the scenario below rewrites storage layouts in every way the
executor can and every epoch has to commit the state root the reference
implementation committed.

The golden file
---------------
``state_root_replay_data/golden.json`` holds, per epoch height, the state root
and the receipts root a reference binary produced for the scenario below.  It
is bound to the consensus rules, to the configuration pinned in
``set_test_params``, to the PoS genesis pinned next to it, and to the scenario
itself.  A change to any of them invalidates the file and it has to be recorded
again::

    tests/state_root_replay_test.py --record-golden \\
        --conflux-binary <path to the reference conflux binary>

Recording writes the file back in place.  Record with a binary whose state
roots are known good -- normally one built from the commit the change under
test is compared against -- never with the binary under test, or the test
degrades into comparing a run with itself.  Recording twice in a row must
produce the same file; if it does not, something in the scenario has stopped
being a function of the scenario and the golden file is worthless until that is
fixed.

``state_root_replay_data/pos_genesis`` is the other half of the recording.  The
framework generates the PoS genesis afresh on every run, the genesis state
credits and registers those validators, and the genesis state root therefore
moves with them; a pinned copy holds it still.  It was produced by::

    pos-genesis-tool random --num-validator=2 --num-genesis-validator=1 \\
        --chain-id=10 --initial-seed=<64 zeros>

Replacing it changes the genesis and invalidates the golden file with it.

Comparing two binaries directly
-------------------------------
``--reference-binary <path>`` compares against a second node running that
binary instead of against the golden file.  It needs no recording, so it stays
usable after a consensus rule change has made the golden file stale; what it
cannot do is run on its own in CI, which is what the golden file is for.

Epochs are identified by height, not by block hash; ``mine`` says why.
"""

import json
import os
import shutil
import sys

import eth_utils
import rlp

sys.path.insert(1, os.path.dirname(os.path.realpath(__file__)))

from conflux.config import default_config
from conflux.rpc import RpcClient
from conflux.utils import sha3 as keccak, priv_to_addr
from test_framework.test_framework import ConfluxTestFramework
from test_framework.util import (
    assert_equal, connect_nodes, set_node_pos_config, sync_blocks, wait_until)

SNAPSHOT_EPOCH_COUNT = 20
ERA_EPOCH_COUNT = 200

# cfx_parameters::consensus::DEFERRED_STATE_EPOCH_COUNT
DEFERRED_STATE_EPOCH_COUNT = 5

# The block timestamps are scripted so that the header content is a function of
# the scenario alone.
BASE_TIMESTAMP = 1700000000

CROSS_SPACE_CALL = "0x0888000000000000000000000000000000000006"
TRANSFER_EVM_SELECTOR = "da8d5daf"

DATA_DIR = os.path.join(
    os.path.dirname(os.path.realpath(__file__)), "state_root_replay_data")
GOLDEN_FILE = os.path.join(DATA_DIR, "golden.json")
# The pinned PoS genesis, installed over the one the framework generates; see
# the module docstring for how it was produced and what replacing it costs.
POS_GENESIS_DIR = os.path.join(DATA_DIR, "pos_genesis")

SIMPLE_STORAGE_PATH = "contracts/simple_storage.dat"
MAPPING_PATH = "contracts/mapping_bytecode.dat"

# Addresses the mapping contract writes its slots for.  Fixed so that the
# scenario is reproducible.
WARD_ADDRESSES = [
    "0x1000000000000000000000000000000000000001",
    "0x1000000000000000000000000000000000000002",
    "0x1000000000000000000000000000000000000003",
    "0x1000000000000000000000000000000000000004",
    "0x1000000000000000000000000000000000000005",
    "0x1000000000000000000000000000000000000006",
]

# Plain accounts created by transfers.
FRESH_ACCOUNTS = [
    "0x1500000000000000000000000000000000000001",
    "0x1500000000000000000000000000000000000002",
    "0x1500000000000000000000000000000000000003",
]

# eSpace accounts created through the cross space call.
FRESH_EVM_ACCOUNTS = [
    "0x2000000000000000000000000000000000000001",
    "0x2000000000000000000000000000000000000002",
]


def selector(signature):
    return eth_utils.encode_hex(keccak(signature.encode()))[2:10]


def abi_address(addr):
    return addr.replace("0x", "").rjust(64, "0")


def abi_bytes20(addr):
    # `bytes20` is encoded left aligned, unlike `address`.
    return addr.replace("0x", "").ljust(64, "0")


def abi_uint(value):
    return hex(value)[2:].rjust(64, "0")


def read_bytecode(relative_path):
    path = os.path.join(
        os.path.dirname(os.path.realpath(__file__)), relative_path)
    assert os.path.isfile(path), path
    return open(path).read().strip()


class StateRootReplayTest(ConfluxTestFramework):
    def set_test_params(self):
        self.num_nodes = 1
        # Everything the scenario's state roots depend on is pinned here.  A
        # recorded golden file is only valid for these values.
        self.conf_parameters["dev_snapshot_epoch_count"] = str(
            SNAPSHOT_EPOCH_COUNT)
        self.conf_parameters["era_epoch_count"] = str(ERA_EPOCH_COUNT)
        self.conf_parameters["evm_transaction_block_ratio"] = str(1)
        # CIP-151 turns self destruction into the EIP-6780 style soft variant,
        # under which an account is only really removed when it was created in
        # the same transaction.  The scenario wants the plain removal, because
        # deleting an account with storage under it is one of the shapes the
        # storage layout collector has to handle.
        self.conf_parameters["cip151_transition_height"] = str(2 ** 31)

    def add_options(self, parser):
        parser.add_argument(
            "--record-golden",
            dest="record_golden",
            default=False,
            action="store_true",
            help="Run the scenario and write the golden file instead of "
                 "asserting against it")
        parser.add_argument(
            "--perturb",
            dest="perturb",
            default=False,
            action="store_true",
            help="Alter one storage write in the scenario.  Used to check that "
                 "the comparison is not vacuous: with this flag the run must "
                 "fail against an unperturbed golden file")
        parser.add_argument(
            "--reference-binary",
            dest="reference_binary",
            default=None,
            type=str,
            help="Path to a second conflux binary.  Instead of comparing "
                 "against the golden file, run it as a second node on the same "
                 "chain and compare the two nodes epoch by epoch.  Compares two "
                 "implementations directly, without a recording in between")

    def after_options_parsed(self):
        super().after_options_parsed()
        if self.options.reference_binary is not None:
            assert not self.options.perturb, \
                "perturbing changes the chain both nodes see, so it is no " \
                "control for the cross binary comparison"
            self.num_nodes = 2

    @property
    def cross_binary(self):
        return self.options.reference_binary is not None

    def install_pinned_pos_genesis(self):
        """Replace the freshly generated PoS genesis with the pinned one.

        `add_nodes` runs `pos-genesis-tool random`, whose validator addresses
        end up in the Conflux genesis state.  Overwriting its output and
        rewriting the per node PoS configuration from it keeps the genesis, and
        therefore every state root, the same from run to run.  Keys and
        registrations are replaced together so that the node still owns the
        validator it is registered as.
        """
        tmpdir = self.options.tmpdir
        shutil.copyfile(
            os.path.join(POS_GENESIS_DIR, "initial_nodes.json"),
            os.path.join(tmpdir, "initial_nodes.json"))
        shutil.copyfile(
            os.path.join(POS_GENESIS_DIR, "public_key"),
            os.path.join(tmpdir, "public_key"))
        for name in os.listdir(os.path.join(POS_GENESIS_DIR, "private_keys")):
            shutil.copyfile(
                os.path.join(POS_GENESIS_DIR, "private_keys", name),
                os.path.join(tmpdir, "private_keys", name))
        for n in range(self.num_nodes):
            set_node_pos_config(
                tmpdir, n,
                pos_round_time_ms=self.pos_parameters["round_time_ms"])

    def setup_network(self):
        binary = [self.options.conflux] * self.num_nodes
        if self.cross_binary:
            binary[1] = self.options.reference_binary
            self.log.info("node 0 runs %s", binary[0])
            self.log.info("node 1 runs %s", binary[1])
        # Only the first node is a PoS genesis validator, so that the pinned
        # genesis is the same whether the test runs one node or two.
        self.add_nodes(self.num_nodes, genesis_nodes=1, binary=binary)
        self.install_pinned_pos_genesis()
        for n in range(self.num_nodes):
            self.start_node(n, ["--archive"])
            self.nodes[n].wait_for_phase(["NormalSyncPhase"])
        if self.cross_binary:
            connect_nodes(self.nodes, 0, 1)
        self.rpc = RpcClient(self.nodes[0])

    # ------------------------------------------------------------------
    # chain driving
    # ------------------------------------------------------------------

    def mine(self, txs=()):
        """Append one block to the linear chain, carrying exactly `txs`.

        Against the golden file the block header has to be a function of the
        scenario alone, which is what `generateBlockWithNonceAndTimestamp`
        offers: the timestamp is taken from the caller instead of the clock.
        Blocks it produces cannot be relayed, though -- the node re-mines the
        nonce after the header hash has already been cached, so the hash it
        answers with is not the hash of the block it broadcasts, and a second
        node asks for a block the first one does not know.  The cross binary
        mode therefore takes the ordinary generation path, whose blocks travel;
        it compares two nodes with each other and does not need the header to
        be reproducible.

        That re-mining is also why epochs are keyed by height everywhere below:
        the hash this RPC answers with is not a quantity another node would
        agree with, while the chain is strictly linear, so height identifies
        the epoch.
        """
        parent = self.chain[-1]
        if self.cross_binary:
            block_hash = self.rpc.generate_custom_block(parent, [], list(txs))
        else:
            index = len(self.chain)
            block_hash = self.nodes[0].test_generateBlockWithNonceAndTimestamp(
                parent,
                [],
                eth_utils.encode_hex(rlp.encode(list(txs))),
                hex(index),
                BASE_TIMESTAMP + index,
                False)
        self.chain.append(block_hash)
        return block_hash

    def mine_empty(self, count):
        for _ in range(count):
            self.mine()

    def next_nonce(self):
        nonce = self.nonce
        self.nonce += 1
        return nonce

    def contract_tx(self, receiver, data_hex, value=0, storage_limit=20000,
                    gas=1000000):
        tx = self.rpc.new_contract_tx(
            receiver=receiver,
            data_hex=data_hex,
            priv_key=self.priv_key,
            nonce=self.next_nonce(),
            gas=gas,
            value=value,
            storage_limit=storage_limit,
            epoch_height=0)
        self.sent_txs.append(tx)
        return tx

    def transfer_tx(self, receiver, value):
        tx = self.rpc.new_tx(
            receiver=receiver,
            priv_key=self.priv_key,
            nonce=self.next_nonce(),
            value=value,
            epoch_height=0)
        self.sent_txs.append(tx)
        return tx

    def deploy(self, bytecode, storage_limit=20000):
        """Deploy in its own block and return the created address."""
        tx = self.contract_tx("", bytecode, storage_limit=storage_limit)
        self.mine([tx])
        # The epoch is executed five epochs later; mine past the deferral so
        # the receipt, and with it the created address, is readable.
        self.mine_empty(6)
        receipt = self.rpc.get_transaction_receipt(tx.hash_hex())
        assert receipt is not None, "deployment was not executed"
        assert_equal(int(receipt["outcomeStatus"], 0), 0)
        return receipt["contractCreated"]

    # ------------------------------------------------------------------
    # the scenario
    # ------------------------------------------------------------------

    def build_chain(self):
        """Drive the chain the golden file is recorded from.

        Every operation which makes the executor rewrite a storage layout entry
        appears at least once, both while the account's layout still lives in
        the delta trie and after a snapshot has pushed it into the lower
        layers.
        """
        self.priv_key = default_config["GENESIS_PRI_KEY"]
        self.sender = eth_utils.encode_hex(priv_to_addr(self.priv_key))
        self.nonce = 0
        self.sent_txs = []
        self.chain = [self.nodes[0].cfx_getBestBlockHash()]
        self.genesis_hash = self.chain[0]

        simple_storage = read_bytecode(SIMPLE_STORAGE_PATH)
        mapping_code = read_bytecode(MAPPING_PATH) + abi_uint(1)

        set1 = selector("set1(address)")
        set2 = selector("set2(address)")
        set0 = selector("set0(address)")
        increment = selector("increment()")
        set_fresh = selector("setFresh()")
        destroy = selector("destroy()")

        # -- contract creation together with storage writes in the same epoch.
        # The constructor writes two slots and sets the layout, so the layout
        # key is written by the execution and then rewritten by the layout
        # pass: exactly the "later write overrides the earlier one" case.
        storage_a = self.deploy(simple_storage)
        self.log.info("simple storage A at %s", storage_a)

        mapping_b = self.deploy(mapping_code)
        self.log.info("mapping B at %s", mapping_b)

        # -- storage overwrite, several writes to the same account in one block
        # and across blocks.
        self.mine([self.contract_tx(storage_a, "0x" + increment)])
        self.mine([
            self.contract_tx(storage_a, "0x" + increment),
            self.contract_tx(storage_a, "0x" + set_fresh),
        ])

        # -- fresh storage slots under one account, spread over blocks.
        for i, ward in enumerate(WARD_ADDRESSES):
            value_selector = set2 if i % 2 else set1
            if self.options.perturb and i == 0:
                # The reverse control: one slot gets a different value.  Every
                # epoch from here on must disagree with the golden file.
                value_selector = set2
            self.mine([
                self.contract_tx(mapping_b, "0x" + value_selector + abi_address(ward)),
            ])

        # -- plain accounts created by transfer: new account keys without a
        # storage layout.
        self.mine([self.transfer_tx(a, 10**18) for a in FRESH_ACCOUNTS])

        # -- eSpace account creation, the other branch of the layout collector.
        for evm_account in FRESH_EVM_ACCOUNTS:
            data = "0x" + TRANSFER_EVM_SELECTOR + abi_bytes20(evm_account)
            self.mine([self.contract_tx(CROSS_SPACE_CALL, data, value=10**17)])

        # -- storage deletion: writing zero removes the slot and leaves the
        # account's layout to be rewritten anyway.
        self.mine([
            self.contract_tx(mapping_b, "0x" + set0 + abi_address(WARD_ADDRESSES[0])),
        ])
        self.mine([
            self.contract_tx(mapping_b, "0x" + set0 + abi_address(WARD_ADDRESSES[1])),
            self.contract_tx(mapping_b, "0x" + set0 + abi_address(WARD_ADDRESSES[2])),
        ])

        # -- cross the first snapshot boundary so that what follows reads its
        # layouts out of the intermediate and snapshot layers instead of the
        # delta trie.
        self.mine_to_height(2 * SNAPSHOT_EPOCH_COUNT + 6)

        self.mine([self.contract_tx(storage_a, "0x" + increment)])
        self.mine([
            self.contract_tx(mapping_b, "0x" + set1 + abi_address(WARD_ADDRESSES[0])),
            self.contract_tx(mapping_b, "0x" + set2 + abi_address(WARD_ADDRESSES[3])),
        ])
        self.mine([
            self.contract_tx(mapping_b, "0x" + set0 + abi_address(WARD_ADDRESSES[3])),
        ])

        # -- a second instance of the same code, created after the boundary.
        storage_c = self.deploy(simple_storage)
        self.log.info("simple storage C at %s", storage_c)
        self.mine([self.contract_tx(storage_c, "0x" + increment)])

        # -- account self destruction with storage under it.
        self.mine([self.contract_tx(storage_a, "0x" + destroy)])
        self.mine_empty(6)
        self.mine([self.transfer_tx(FRESH_ACCOUNTS[0], 10**17)])

        # -- cross a third snapshot boundary and let every epoch execute.
        self.mine_to_height(3 * SNAPSHOT_EPOCH_COUNT + 12)

        self.contracts = {"a": storage_a, "b": mapping_b, "c": storage_c}
        self.log.info("chain built, %d blocks", len(self.chain) - 1)

    def mine_to_height(self, height):
        while len(self.chain) - 1 < height:
            self.mine()

    def verify_scenario(self):
        """Check the scenario did what it claims, so that an agreement on the
        state roots is agreement about a chain which actually rewrote storage
        layouts, and not about a chain of failed transactions."""
        for tx in self.sent_txs:
            receipt = self.rpc.get_transaction_receipt(tx.hash_hex())
            assert receipt is not None, \
                "transaction %s was never executed" % tx.hash_hex()
            assert_equal(int(receipt["outcomeStatus"], 0), 0)

        epoch = hex(self.last_executed_height)
        root_a = self.rpc.get_storage_root(self.contracts["a"], epoch)
        root_b = self.rpc.get_storage_root(self.contracts["b"], epoch)
        root_c = self.rpc.get_storage_root(self.contracts["c"], epoch)
        self.log.info("storage roots A=%s B=%s C=%s", root_a, root_b, root_c)

        # The self destructed contract had storage in the snapshot layer and
        # is now masked by a tombstone: the account was really removed, which
        # is the account deletion shape the layout collector has to handle.
        assert "TOMBSTONE" in root_a.values(), \
            "contract A was not removed, %s" % root_a
        assert root_a["snapshot"] is not None, \
            "contract A never reached the snapshot layer, %s" % root_a

        # The account written throughout the run reached the snapshot layer,
        # so its layout entry was rewritten while living below the delta trie.
        assert root_b["snapshot"] is not None and \
            root_b["intermediate"] is not None, \
            "the scenario did not cross the snapshot boundaries as intended, " \
            "%s" % root_b

        # The contract created after the first boundaries is alive and holds
        # storage.
        assert root_c["intermediate"] is not None or \
            root_c["delta"] not in (None, "TOMBSTONE"), \
            "contract C has no storage, %s" % root_c

        for evm_account in FRESH_EVM_ACCOUNTS:
            balance = self.nodes[0].eth_getBalance(evm_account, "latest")
            assert int(balance, 0) > 0, \
                "eSpace account %s was not created" % evm_account

        self.log.info(
            "scenario verified: %d transactions executed, all succeeded",
            len(self.sent_txs))

    # ------------------------------------------------------------------
    # collecting and comparing
    # ------------------------------------------------------------------

    def collect_roots(self, node_index=0):
        """Read back the committed roots of every executed epoch.

        The chain is linear, so the block at index `h` is the pivot of epoch
        `h`.  `test_getExecutedInfo` answers with the very commitment consensus
        stores for the epoch, the state root being the hash of the three layer
        roots.
        """
        roots = []
        for height in range(1, self.last_executed_height + 1):
            receipts_root, state_root = \
                self.nodes[node_index].test_getExecutedInfo(self.chain[height])
            roots.append({
                "height": height,
                "state_root": state_root,
                "receipts_root": receipts_root,
            })
        assert len(roots) > 3 * SNAPSHOT_EPOCH_COUNT, \
            "too few executed epochs: %d" % len(roots)
        return roots

    def report_first_divergence(self, expected, observed, expected_name,
                                observed_name):
        """Compare two epoch keyed root tables.  Returns the first height they
        disagree on, or None."""
        common = sorted(set(expected) & set(observed))
        assert len(common) > 3 * SNAPSHOT_EPOCH_COUNT, \
            "the two sides share too few epochs: %d" % len(common)
        for height in common:
            if expected[height] == observed[height]:
                continue
            self.log.error(
                "epoch %d diverges: %s state root %s receipts root %s, "
                "%s state root %s receipts root %s",
                height,
                expected_name,
                expected[height]["state_root"],
                expected[height]["receipts_root"],
                observed_name,
                observed[height]["state_root"],
                observed[height]["receipts_root"])
            return height
        self.log.info(
            "%s and %s agree on the state root of all %d compared epochs",
            expected_name, observed_name, len(common))
        return None

    def wait_for_execution(self):
        """Wait until every epoch this test compares has been executed.

        Which epochs carry a commitment at an arbitrary moment depends on how
        far the background execution has got, so the range is fixed here rather
        than discovered: everything outside the execution deferral window.
        """
        self.last_executed_height = \
            len(self.chain) - 1 - DEFERRED_STATE_EPOCH_COUNT
        wait_until(
            lambda: self.rpc.epoch_number("latest_state")
            >= self.last_executed_height,
            timeout=60)
        self.log.info("epochs 1..%d executed", self.last_executed_height)

    def run_test(self):
        self.build_chain()
        self.wait_for_execution()
        self.verify_scenario()
        roots = self.collect_roots()
        self.log.info("collected %d executed epochs", len(roots))

        if self.options.record_golden:
            self.record(roots)
        elif self.cross_binary:
            self.compare_with_reference_node(roots)
        else:
            self.compare_with_golden(roots)

    def record(self, roots):
        assert not self.options.perturb, "refusing to record a perturbed run"
        assert not self.cross_binary, \
            "record from a single node, so that what is recorded is one " \
            "binary's answer"
        record = {
            "scenario": os.path.basename(__file__),
            "conf_parameters": self.conf_parameters,
            "genesis_hash": self.genesis_hash,
            "epochs": roots,
        }
        with open(GOLDEN_FILE, "w") as f:
            json.dump(record, f, indent=1, sort_keys=True)
            f.write("\n")
        self.log.info("golden file written to %s", GOLDEN_FILE)

    def compare_with_golden(self, roots):
        with open(GOLDEN_FILE) as f:
            golden = json.load(f)

        # A different genesis or a different configuration means the golden
        # file describes a different chain and the comparison below would be
        # meaningless rather than failing informatively.
        assert_equal(golden["conf_parameters"], self.conf_parameters)
        assert_equal(golden["genesis_hash"], self.genesis_hash)

        divergence = self.report_first_divergence(
            {e["height"]: e for e in golden["epochs"]},
            {e["height"]: e for e in roots},
            "the recording", "this run")
        if divergence is not None:
            raise AssertionError(
                "state root diverges from the recording at epoch %d"
                % divergence)

    def compare_with_reference_node(self, roots):
        """Compare the two binaries on one and the same chain.

        Node 1 receives node 0's blocks over the network, so both sides execute
        the very same blocks, and their commitments are compared directly with
        no recording in between.
        """
        sync_blocks(self.nodes, sync_state=False)
        reference = RpcClient(self.nodes[1])
        wait_until(
            lambda: reference.epoch_number("latest_state")
            >= self.last_executed_height,
            timeout=120)

        divergence = self.report_first_divergence(
            {e["height"]: e for e in roots},
            {e["height"]: e for e in self.collect_roots(node_index=1)},
            "node 0", "node 1")
        if divergence is not None:
            raise AssertionError(
                "the two binaries diverge at epoch %d" % divergence)

        # Node 1 also judges node 0's headers, which carry node 0's deferred
        # state root.  Consensus only keeps that verdict for part of the chain,
        # so the blocks it has no verdict on are skipped rather than demanded.
        judged = 0
        for height in range(1, self.last_executed_height + 1):
            try:
                _, state_valid = \
                    self.nodes[1].test_getBlockStatus(self.chain[height])
            except Exception:
                continue
            assert state_valid, \
                "node 1 rejects the deferred state root of the block at " \
                "height %d" % height
            judged += 1
        assert judged > 0, "node 1 judged no block's deferred state root"
        self.log.info(
            "node 1 accepts the deferred state root of the %d blocks it has a "
            "verdict on", judged)


if __name__ == "__main__":
    StateRootReplayTest().main()
