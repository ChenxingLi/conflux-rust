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

pub const WITH_PROOF: bool = true;
pub const NO_PROOF: bool = false;

/// A read-only handle on one version of the state: a point read and a prefix
/// lookup, nothing else.
pub trait StorageView: Sync + Send {
    fn get(&self, access_key: StorageKeyWithSpace)
        -> Result<Option<Box<[u8]>>>;

    /// `iter_prefix` yields each key at most once, with the value the version
    /// holds for it, and never yields a key the version has deleted. The order
    /// is unspecified.
    fn iter_prefix(
        &self, access_key_prefix: StorageKeyWithSpace,
    ) -> Result<Box<dyn Iterator<Item = MptKeyValue>>>;
}

/// The whole write set of one epoch, `None` deletes the key.
pub type Changeset = BTreeMap<Vec<u8>, Option<Box<[u8]>>>;

/// The epoch a changeset commits.
#[derive(Clone, Copy, Debug)]
pub struct CommitMeta {
    pub epoch_id: EpochId,
    pub height: u64,
}

use super::{impls::errors::*, MptKeyValue};
use primitives::{EpochId, StorageKeyWithSpace};
use std::collections::BTreeMap;
