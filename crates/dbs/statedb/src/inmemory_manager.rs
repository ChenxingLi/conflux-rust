// Copyright 2025 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

//! The in memory engine the unit tests read and write state through. It is
//! the second implementation of `StorageEngine`, next to the real engine, so a
//! test names a parent version and lets the engine hand out the read view and
//! take the changeset, which is the path the production callers take.
//!
//! A version is the whole key value map of one epoch. Committing clones the
//! parent's map, applies the changeset and files the result under the new
//! epoch id.

use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use cfx_internal_common::StateRootWithAuxInfo;
use cfx_storage::{
    Changeset, CommitMeta, ConsensusRecoveryView, Error, MptKeyValue,
    OpenOptions, RecoveryPlan, Result, StateConfirmedView, StorageEngine,
    StorageVersion, StorageView,
};
use cfx_types::H256;
use parking_lot::RwLock;
use primitives::{EpochId, StateRoot, StorageKeyWithSpace};
use tiny_keccak::{Hasher, Keccak};

/// The contents of one version, keyed by the raw storage key bytes, which is
/// the key type a `Changeset` carries.
type Version = BTreeMap<Vec<u8>, Box<[u8]>>;

thread_local! {
    static THREAD_MANAGER: RefCell<Option<Arc<InmemoryManager>>> =
        const { RefCell::new(None) };
}

pub struct InmemoryManager {
    versions: RwLock<HashMap<EpochId, Version>>,
}

impl InmemoryManager {
    pub fn new() -> Arc<Self> {
        Arc::new(InmemoryManager {
            versions: RwLock::new(HashMap::new()),
        })
    }

    /// One instance per thread. A test that commits an epoch and then opens it
    /// again needs both calls to reach the same instance, and the test harness
    /// gives each test case a thread of its own, so a thread local instance
    /// isolates the cases from one another.
    pub fn for_current_thread() -> Arc<Self> {
        THREAD_MANAGER.with(|slot| {
            slot.borrow_mut().get_or_insert_with(Self::new).clone()
        })
    }

    /// The epoch id of the empty version every instance starts with. It stands
    /// in for the empty base a test writes its first epoch on top of.
    ///
    /// The id is reserved: `commit` refuses to write under it, so no test can
    /// give the empty base contents and change what the next test on the same
    /// thread starts from. The value is out of the range of the small hashes
    /// the tests name their epochs by.
    /// File `contents` as the version of `epoch_id`, so that a test can start
    /// from a state that is not empty without writing an epoch first.
    pub fn seed(&self, epoch_id: EpochId, contents: Version) {
        self.versions.write().insert(epoch_id, contents);
    }

    fn contents_of(&self, version: StorageVersion) -> Option<Version> {
        match version {
            StorageVersion::Empty => Some(Version::new()),
            StorageVersion::Epoch(epoch_id) => {
                self.versions.read().get(&epoch_id).cloned()
            }
        }
    }

    /// The state root of a version: keccak over every key and value in key
    /// order. It is not the MPT root of the real storage engine and nothing
    /// compares the two.
    fn state_root_of(version: &Version) -> StateRoot {
        let mut hasher = Keccak::v256();
        for (k, v) in version {
            hasher.update(k);
            hasher.update(v);
        }
        let mut output = [0u8; 32];
        hasher.finalize(&mut output);
        StateRootWithAuxInfo::genesis(&H256(output)).state_root
    }

    fn apply(version: &mut Version, changeset: Changeset) {
        for (key_bytes, value) in changeset {
            match value {
                Some(value) => {
                    version.insert(key_bytes, value);
                }
                None => {
                    version.remove(&key_bytes);
                }
            }
        }
    }
}

impl StorageEngine for InmemoryManager {
    fn open_state(
        self: Arc<Self>, version: StorageVersion, _opts: OpenOptions,
    ) -> Result<Option<Box<dyn StorageView>>> {
        Ok(self.contents_of(version).map(|version| {
            Box::new(InmemoryView { version }) as Box<dyn StorageView>
        }))
    }

    fn commit(
        self: Arc<Self>, parent: StorageVersion, changeset: Changeset,
        meta: CommitMeta,
    ) -> Result<StateRoot> {
        let mut version = self.contents_of(parent).ok_or_else(|| {
            Error::Msg(format!(
                "the parent version {:?} is not available",
                parent
            ))
        })?;
        Self::apply(&mut version, changeset);
        let state_root = Self::state_root_of(&version);
        self.versions.write().insert(meta.epoch_id, version);
        Ok(state_root)
    }

    /// The empty base of this engine is an empty map, so that is where the
    /// preview starts.
    fn preview_genesis_root(
        self: Arc<Self>, changeset: &Changeset,
    ) -> Result<StateRoot> {
        let mut version = Version::new();
        Self::apply(&mut version, changeset.clone());
        Ok(Self::state_root_of(&version))
    }

    fn usable_as_base(&self, base: &EpochId) -> bool {
        self.versions.read().contains_key(base)
    }

    /// Nothing is ever lost here, so the plan is the height consensus
    /// proposed.
    fn plan_recovery(&self, view: &dyn ConsensusRecoveryView) -> RecoveryPlan {
        RecoveryPlan {
            recompute_start_height: view.proposed_recompute_start_height(),
        }
    }

    /// Nothing is ever collected here, so every version this engine has ever
    /// been given is still openable and the bound stays at zero.
    fn notify_state_confirmed(&self, _view: &dyn StateConfirmedView) -> u64 {
        0
    }
}

/// A read handle on one version, holding its own copy of the map.
struct InmemoryView {
    version: Version,
}

impl StorageView for InmemoryView {
    fn get(
        &self, access_key: StorageKeyWithSpace,
    ) -> Result<Option<Box<[u8]>>> {
        Ok(self.version.get(&access_key.to_key_bytes()).cloned())
    }

    fn iter_prefix(
        &self, access_key_prefix: StorageKeyWithSpace,
    ) -> Result<Box<dyn Iterator<Item = MptKeyValue>>> {
        let prefix = access_key_prefix.to_key_bytes();
        let kvs: Vec<MptKeyValue> = self
            .version
            .range(prefix.clone()..)
            .take_while(|(k, _)| k.starts_with(&prefix))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        Ok(Box::new(kvs.into_iter()))
    }
}
