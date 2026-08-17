// Copyright 2019 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

/// A block defines a list of transactions that it sees and the sequence of
/// the transactions (ledger). At the view of a block, after all
/// transactions being executed, the data associated with all addresses is
/// a State after the epoch defined by the block.
///
/// A writable state is copy-on-write reference to the base state in the
/// state manager. State is supposed to be owned by single user.
pub use super::impls::state::State;

/// The two values of the flag the read paths take, which decides whether a
/// proof is built alongside the value. Two constants rather than bare `true`
/// and `false`, so that a call site reads as the question it answers.
pub const WITH_PROOF: bool = true;
pub const NO_PROOF: bool = false;

/// A read-only handle on one version of the state. This is the interface the
/// execution engine and the transaction pool need: a point read and a prefix
/// lookup, nothing else.
///
/// `iter_prefix` returns a boxed iterator rather than a `Vec`. The three
/// layers of the state have to be merged and de-duplicated before the first
/// item can be produced, so the current implementations materialize the result
/// and hand back an iterator over it.
///
/// `iter_prefix` guarantees that a key appears at most once, carrying the value
/// from the newest layer that has it, and that keys shadowed by a tombstone do
/// not appear at all. It does NOT guarantee a globally sorted order: the items
/// come out as three concatenated sorted runs, one per layer. That order
/// reaches the execution path through `StateDb`'s range delete, so it is
/// consensus visible — do not "fix" it into a global sort without an
/// equivalence argument.
pub trait StorageView: Sync + Send {
    fn get(&self, access_key: StorageKeyWithSpace)
        -> Result<Option<Box<[u8]>>>;

    fn iter_prefix(
        &self, access_key_prefix: StorageKeyWithSpace,
    ) -> Result<Box<dyn Iterator<Item = MptKeyValue>>>;
}

/// The whole write set of one epoch, in key order. `Some(value)` writes the
/// value under the key, `None` deletes it. There is no range write: the range
/// delete of `StateDb` is materialized into point deletes before it reaches
/// here.
///
/// The key is the raw storage key bytes, i.e. `StorageKeyWithSpace::
/// to_key_bytes`, the key type `StateDb` caches under. The `BTreeMap` is what
/// fixes the ascending key order the engine writes in.
pub type Changeset = BTreeMap<Vec<u8>, Option<Box<[u8]>>>;

/// What the engine has to know about the epoch a changeset commits, besides
/// the changeset itself. `epoch_id` is the pivot block hash of that epoch, the
/// same identifier the open path is keyed by, and `height` is the height of
/// that same epoch.
#[derive(Clone, Copy, Debug)]
pub struct CommitMeta {
    pub epoch_id: EpochId,
    pub height: u64,
}

use super::{impls::errors::*, MptKeyValue};
use primitives::{EpochId, StorageKeyWithSpace};
use std::collections::BTreeMap;
