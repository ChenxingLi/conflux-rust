#!/usr/bin/env python3
"""Check what the rest of the tree imports from cfx-storage.

The export surface of cfx-storage is a whitelist: the interface items and
the construction configuration they are built from, plus five exceptions that
are named one by one. This script collects every `cfx_storage::` path used
outside the crate itself and sorts each one into

  * allowed   -- a whitelist item or a named exception,
  * pending   -- still exported, but a planned migration removes it; the task
                 that owns it is named in PENDING below,
  * violation -- neither of the above.

It exits non-zero when there is a violation. With --strict it also exits
non-zero when there is anything pending, which is what the end state of the
refactor has to reach.

Run it from the repository root:

    ./dev-support/check_storage_exports.py
    ./dev-support/check_storage_exports.py --strict
"""

import argparse
import os
import re
import sys

STORAGE_CRATE = os.path.join("crates", "dbs", "storage")

# `crates/cfxcore/core/benchmark/storage` is not a workspace member and does
# not build on master either, so it is reported apart and never counted.
UNCOUNTED = [os.path.join("crates", "cfxcore", "core", "benchmark")]

# The ten interface items, then the construction configuration they are built
# from. Everything in this set is allowed unconditionally.
WHITELIST = {
    # the interface proper
    "StorageView",
    "StorageEngine",
    "StorageVersion",
    "OpenOptions",
    "Changeset",
    "CommitMeta",
    "StateConfirmedView",
    # the recovery handshake types, which appear on StorageEngine's signature
    "ConsensusRecoveryView",
    "RecoveryPlan",
    # the construction configuration
    "StorageConfiguration",
    "ConsensusParam",
    "ProvideExtraSnapshotSyncConfig",
    "storage_dir",
    "defaults::DEFAULT_DEBUG_SNAPSHOT_CHECKER_THREADS",
    "defaults::DEFAULT_DELTA_MPTS_CACHE_RECENT_LFU_FACTOR",
    "defaults::DEFAULT_DELTA_MPTS_CACHE_SIZE",
    "defaults::DEFAULT_DELTA_MPTS_CACHE_START_SIZE",
    "defaults::DEFAULT_DELTA_MPTS_SLAB_IDLE_SIZE",
    "defaults::DEFAULT_MAX_OPEN_MPT",
    "defaults::DEFAULT_MAX_OPEN_SNAPSHOTS",
    "defaults::MAX_CACHED_TRIE_NODES_R_LFU_COUNTER",
}

# The five exceptions, each with the symbols it covers.
EXCEPTIONS = {
    "1. proof data types (6.3)": {
        "TrieProof",
        "TrieProofNode",
        "StateProof",
        "NodeMerkleProof",
        "StorageRootProof",
    },
    "2. the simple_mpt algorithm family (6.2)": {
        "SimpleMpt",
        "into_simple_mpt_key",
        "make_simple_mpt",
        "simple_mpt_merkle_root",
        "simple_mpt_proof",
    },
    "3. the concrete engine type and its adapter entry points (chapter 5)": {
        "StorageManager",
        "StorageState",
        "StateDbGetOriginalMethods",
        "StateExport",
        "FullSyncVerifier",
        "SnapshotDbManagerSqlite",
        "SnapshotInfo",
        "SnapshotKeptToProvideSyncStatus",
        "DeltaMptIterator",
    },
    "4. the delta MPT key prefix type and its derivation (4.2)": {
        "DeltaMptKeyPadding",
        "delta_mpt_padding",
    },
    "5. the engine's own error type (4.1)": {
        "Error",
        "Result",
    },
}

# Still exported, with the task that removes each one.
PENDING = {
    "KeyValueDbTrait": "6.1 ledger key value library, task A6",
    "KeyValueDbTraitRead": "6.1 ledger key value library, task A6",
    "KvdbRocksdb": "6.1 ledger key value library, task A6",
    "KvdbSqlite": "6.1 ledger key value library, task A6",
    "KvdbSqliteStatements": "6.1 ledger key value library, task A6",
    "MptKeyValue": "6.4 move to a common crate",
    "utils::access_mode": "6.4 moves out with StateDb",
    "utils::to_key_prefix_iter_upper_bound": "6.4 moves out with StateDb",
    "utils::guarded_value::*": "6.4 moves to a general utility crate",
    "defaults::DEFAULT_EXECUTION_PREFETCH_THREADS": "6.4 moves out",
    "CompressedPathRaw": "appendix C A8, verification.rs tests",
    "VanillaChildrenTable": "appendix C A8, verification.rs tests",
    "tests::new_state_manager_for_unit_test": "appendix C A8, test helpers",
}

USE_RE = re.compile(r"\buse\s+cfx_storage\s*::")
INLINE_RE = re.compile(
    r"(?<![\w:])cfx_storage\s*::\s*((?:\w+\s*::\s*)*(?:\w+|\*))")


def strip_line_comments(text):
    out = []
    for line in text.split("\n"):
        i = line.find("//")
        if i >= 0:
            line = line[:i]
        out.append(line)
    return "\n".join(out)


def split_top_level(body):
    items, depth, cur = [], 0, ""
    for ch in body:
        if ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
        if ch == "," and depth == 0:
            items.append(cur)
            cur = ""
        else:
            cur += ch
    if cur.strip():
        items.append(cur)
    return items


def expand(prefix, body):
    """Expand one brace tree into the leaf paths it names."""
    leaves = []
    for item in split_top_level(body):
        item = item.strip()
        if not item:
            continue
        i = item.find("{")
        if i < 0:
            leaves.append(prefix + item)
        else:
            leaves.extend(expand(prefix + item[:i], item[i + 1:-1]))
    return leaves


def normalize(path):
    """Drop the rename of `as`, and all whitespace."""
    path = re.sub(r"\s+", " ", path).strip()
    path = re.split(r"\bas\b", path)[0]
    return re.sub(r"\s+", "", path)


def paths_in_file(path):
    text = strip_line_comments(open(path, encoding="utf-8").read())
    found = []
    for m in USE_RE.finditer(text):
        i, depth, j = m.end(), 0, m.end()
        while j < len(text):
            c = text[j]
            if c == "{":
                depth += 1
            elif c == "}":
                depth -= 1
            elif c == ";" and depth == 0:
                break
            j += 1
        found.extend(expand("", text[i:j]))
    for m in INLINE_RE.finditer(text):
        if text[:m.start()].rstrip().endswith("use"):
            continue
        found.append(m.group(1))
    return [normalize(f) for f in found if normalize(f)]


def classify(path):
    if path in WHITELIST:
        return "allowed", "whitelist"
    for name, symbols in EXCEPTIONS.items():
        if path in symbols:
            return "allowed", "exception " + name
    if path in PENDING:
        return "pending", PENDING[path]
    return "violation", ""


def collect(root):
    counted, uncounted = {}, {}
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d != ".git" and d != "target"]
        rel_dir = os.path.relpath(dirpath, root)
        if rel_dir.startswith(STORAGE_CRATE):
            continue
        bucket = counted
        for prefix in UNCOUNTED:
            if rel_dir.startswith(prefix):
                bucket = uncounted
        for filename in filenames:
            if not filename.endswith(".rs"):
                continue
            full = os.path.join(dirpath, filename)
            rel = os.path.relpath(full, root)
            for path in paths_in_file(full):
                bucket.setdefault(path, set()).add(rel)
    return counted, uncounted


def report(title, entries):
    if not entries:
        return
    print(title)
    for path in sorted(entries):
        where, why = entries[path]
        print("  %s" % path)
        if why:
            print("      %s" % why)
        for f in sorted(where):
            print("      %s" % f)
    print("")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--root", default=".", help="repository root, default the cwd")
    parser.add_argument(
        "--strict", action="store_true",
        help="also fail on the pending migrations")
    args = parser.parse_args()

    counted, uncounted = collect(args.root)

    violations, pending = {}, {}
    allowed = 0
    for path, files in counted.items():
        verdict, why = classify(path)
        if verdict == "violation":
            violations[path] = (files, why)
        elif verdict == "pending":
            pending[path] = (files, why)
        else:
            allowed += 1

    report("Not on the whitelist and not a named exception:", violations)
    report("Exported until a planned migration removes it:", pending)

    unc_violations = {}
    for path, files in uncounted.items():
        verdict, why = classify(path)
        if verdict != "allowed":
            unc_violations[path] = (files, why)
    report("Outside the acceptance set, not counted:", unc_violations)

    print("%d symbols allowed, %d pending, %d violations."
          % (allowed, len(pending), len(violations)))
    if violations:
        return 1
    if args.strict and pending:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
