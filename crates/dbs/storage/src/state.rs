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
use cfx_types::AddressWithSpace;

pub type WithProof = primitives::static_bool::Yes;
pub type NoProof = primitives::static_bool::No;

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
/// delete of `StateDb` is materialized into point deletes before it ever
/// reaches here, which it already was under the old replay path.
///
/// The key is the raw storage key bytes, i.e. `StorageKeyWithSpace::
/// to_key_bytes`, which is exactly the key type `StateDb` caches under. Using
/// a `BTreeMap` keeps the iteration order of the old replay loop, which walked
/// `accessed_entries` — a `BTreeMap` over the same bytes — in ascending key
/// order.
pub type Changeset = BTreeMap<Vec<u8>, Option<Box<[u8]>>>;

/// What the engine has to know about the epoch a changeset commits, besides
/// the changeset itself. `epoch_id` is the pivot block hash of that epoch, the
/// same identifier the open path is keyed by.
///
/// There is no `height` here yet. It reaches `StateDb` through its
/// construction, and until then the engine reads the height off the state
/// object, where the open path put it.
#[derive(Clone, Copy, Debug)]
pub struct CommitMeta {
    pub epoch_id: EpochId,
}

// The trait is created to separate the implementation to another file, and the
// concrete struct is put into inner mod, because the implementation is
// anticipated to be too complex to present in the same file of the API.
pub trait StateTrait: StorageView {
    // Actions.
    fn set(
        &mut self, access_key: StorageKeyWithSpace, value: Box<[u8]>,
    ) -> Result<()>;
    fn delete(&mut self, access_key: StorageKeyWithSpace) -> Result<()>;

    fn read_all_with_callback(
        &mut self, _access_key_prefix: StorageKeyWithSpace,
        _callback: &mut dyn FnMut(MptKeyValue), _only_account_key: bool,
    ) -> Result<()> {
        Err(Error::Msg("Not implemented".into()))
    }

    // Finalize
    /// It's costly to compute state root however it's only necessary to compute
    /// state root once before committing.
    fn compute_state_root(&mut self) -> Result<StateRootWithAuxInfo>;
    fn get_state_root(&self) -> Result<StateRootWithAuxInfo>;
    fn commit(&mut self, epoch: EpochId) -> Result<StateRootWithAuxInfo>;

    /// Apply a whole changeset, in key order. This is the old `StateDb` replay
    /// loop, moved behind the interface: the same `set` and `delete` calls in
    /// the same order, so it writes the same bytes.
    fn apply_changeset(&mut self, changeset: &Changeset) -> Result<()> {
        for (key_bytes, value) in changeset {
            let access_key = StorageKeyWithSpace::from_key_bytes::<
                SkipInputCheck,
            >(key_bytes);
            match value {
                Some(value) => self.set(access_key, value.clone())?,
                None => self.delete(access_key)?,
            }
        }
        Ok(())
    }

    /// The commit entry point: one changeset plus the meta of the epoch it
    /// belongs to, in a single call. Apply, compute the root, persist.
    ///
    /// The root is read back rather than recomputed when it is already there,
    /// which is the genesis path: `compute_state_root` ran on this very state
    /// object before the epoch id existed. Every other epoch leaves the delta
    /// nodes dirty, `get_state_root` fails on them, and the root is computed
    /// here.
    ///
    /// This lives on the state object because `StateDb` holds one. It belongs
    /// on the engine, and moves there once `StateDb` holds a read only view
    /// plus the parent version and the height.
    fn commit_changeset(
        &mut self, changeset: Changeset, meta: CommitMeta,
    ) -> Result<StateRootWithAuxInfo> {
        self.apply_changeset(&changeset)?;

        let state_root = match self.get_state_root() {
            Ok(root) => root,
            Err(_) => self.compute_state_root()?,
        };

        self.commit(meta.epoch_id)?;

        Ok(state_root)
    }
}

// We skip the accessed_entries for getting original value.
pub trait StateDbGetOriginalMethods {
    fn get_original_raw_with_proof(
        &self, key: StorageKeyWithSpace,
    ) -> Result<(Option<Box<[u8]>>, StateProof)>;

    fn get_original_storage_root(
        &self, address: &AddressWithSpace,
    ) -> Result<StorageRoot>;

    fn get_original_storage_root_with_proof(
        &self, address: &AddressWithSpace,
    ) -> Result<(StorageRoot, StorageRootProof)>;
}

use super::{
    impls::{errors::*, state_proof::StateProof},
    MptKeyValue, StateRootWithAuxInfo,
};
use crate::StorageRootProof;
use primitives::{EpochId, SkipInputCheck, StorageKeyWithSpace, StorageRoot};
use std::collections::BTreeMap;
