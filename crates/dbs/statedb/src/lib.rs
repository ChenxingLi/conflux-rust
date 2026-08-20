// Copyright 2019 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

#[macro_use]
extern crate cfx_util_macros;
#[macro_use]
extern crate log;

pub mod global_params;
#[cfg(any(test, feature = "testonly_code"))]
mod inmemory_manager;
mod statedb_ext;
use cfx_types::H256;
use primitives::StorageValue;

use cfx_db_errors::statedb as error;

#[cfg(test)]
mod tests;

#[cfg(any(test, feature = "testonly_code"))]
pub use self::inmemory_manager::InmemoryManager;
pub use self::{
    error::{Error, Result},
    impls::StateDb as StateDbGeneric,
    statedb_ext::StateDbExt,
};
pub use cfx_storage::utils::access_mode;
pub type StateDb = StateDbGeneric;

// Put StateDb in mod to make sure that methods from statedb_ext don't access
// its fields directly.
mod impls {
    type Key = Vec<u8>;
    type Value = Option<Arc<[u8]>>;

    // Use BTreeMap so that we can delete ranges efficiently
    // see `delete_all`
    type AccessedEntries = BTreeMap<Key, EntryValue>;

    /// What a commit capable `StateDb` needs to hand its changeset to the
    /// engine. The parent version and the height come in through the
    /// construction; the engine opens the writable state of the next epoch
    /// itself when the changeset arrives.
    struct CommitContext {
        manager: Arc<dyn StorageEngine>,
        parent: StorageVersion,
        /// Height of the epoch this instance commits, i.e. the parent's height
        /// plus one.
        height: u64,
    }

    /// How one `StateDb` reads, and whether it can commit.
    enum Storage {
        /// Read only instance. Committing on it is an error.
        ReadOnly(Box<dyn StorageView>),

        /// Commit capable instance: a read only view of the parent version to
        /// read through, plus what the engine needs to commit on top of it.
        Commit {
            view: Box<dyn StorageView>,
            ctx: CommitContext,
        },
    }

    impl Storage {
        fn view(&self) -> &dyn StorageView {
            match self {
                Storage::ReadOnly(view) => &**view,
                Storage::Commit { view, .. } => &**view,
            }
        }
    }

    // Use generic type for better test-ability.
    pub struct StateDb {
        /// Contains the original storage key values for all loaded and
        /// modified key values.
        accessed_entries: RwLock<AccessedEntries>,

        /// Where the reads go and where the changeset goes. The underlying
        /// storage is updated only upon fn commit().
        storage: Storage,
    }

    impl StateDb {
        /// A read only instance. `commit` on it returns an error.
        pub fn new_readonly(view: Box<dyn StorageView>) -> Self {
            StateDb {
                accessed_entries: Default::default(),
                storage: Storage::ReadOnly(view),
            }
        }

        /// A commit capable instance on top of the epoch `parent` identifies.
        /// The read handle of that version is opened here and held until the
        /// commit; the writable state of the epoch being executed is opened by
        /// the engine when the changeset arrives.
        pub fn new_for_commit(
            manager: Arc<dyn StorageEngine>, parent: EpochId, height: u64,
        ) -> Result<Self> {
            let view = manager
                .clone()
                .open_state(StorageVersion::Epoch(parent), OpenOptions::new())?
                .ok_or_else(|| {
                    Error::Msg(format!(
                        "the parent version {:?} is not available",
                        parent
                    ))
                })?;
            Ok(StateDb {
                accessed_entries: Default::default(),
                storage: Storage::Commit {
                    view,
                    ctx: CommitContext {
                        manager,
                        parent: StorageVersion::Epoch(parent),
                        height,
                    },
                },
            })
        }

        /// The commit capable instance of the genesis epoch. Its parent is
        /// the empty base, which has no epoch id of its own, so it cannot go
        /// through `new_for_commit`. The height is 0, the genesis epoch's own
        /// height on the convention the open path uses.
        pub fn new_for_genesis(
            manager: Arc<dyn StorageEngine>,
        ) -> Result<Self> {
            let view = manager
                .clone()
                .open_state(StorageVersion::Empty, OpenOptions::new())?
                .ok_or_else(|| {
                    Error::Msg("the empty base is always open".into())
                })?;
            Ok(StateDb {
                accessed_entries: Default::default(),
                storage: Storage::Commit {
                    view,
                    ctx: CommitContext {
                        manager,
                        parent: StorageVersion::Empty,
                        height: 0,
                    },
                },
            })
        }

        /// A commit capable instance on the in memory engine's empty base.
        /// The engine is the one shared by every unit test on this thread, so
        /// an epoch committed here can be opened again by
        /// `new_for_unit_test_with_epoch`.
        #[cfg(any(test, feature = "testonly_code"))]
        pub fn new_for_unit_test() -> Self {
            use super::inmemory_manager::InmemoryManager;

            Self::new_for_genesis(InmemoryManager::for_current_thread())
                .expect("the empty base is always open")
        }

        /// A commit capable instance on top of an epoch some earlier unit test
        /// step committed into the in memory engine of this thread. It panics
        /// on an epoch that was never committed.
        #[cfg(any(test, feature = "testonly_code"))]
        pub fn new_for_unit_test_with_epoch(epoch_id: &EpochId) -> Self {
            use super::inmemory_manager::InmemoryManager;

            Self::new_for_commit(
                InmemoryManager::for_current_thread(),
                *epoch_id,
                0,
            )
            .expect("the epoch was committed by an earlier test step")
        }

        #[cfg(test)]
        pub fn get_from_cache(&self, key: &Vec<u8>) -> Value {
            self.accessed_entries
                .read()
                .get(key)
                .and_then(|v| v.current_value.clone())
        }

        /// Update the accessed_entries while getting the value.
        pub(crate) fn get_raw(
            &self, key: StorageKeyWithSpace,
        ) -> Result<Option<Arc<[u8]>>> {
            let key_bytes = key.to_key_bytes();
            let r;
            let accessed_entries_read_guard = self.accessed_entries.read();
            if let Some(v) = accessed_entries_read_guard.get(&key_bytes) {
                r = v.current_value.clone();
            } else {
                drop(accessed_entries_read_guard);
                let mut accessed_entries = self.accessed_entries.write();
                let entry = accessed_entries.entry(key_bytes);
                if let Occupied(o) = &entry {
                    r = o.get().current_value.clone();
                } else {
                    r = self.storage.view().get(key)?.map(Into::into);
                    entry.or_insert(EntryValue::new(r.clone()));
                };
            };
            trace!("get_raw key={:?}, value={:?}", key, r);
            Ok(r)
        }

        #[cfg(feature = "testonly_code")]
        pub fn get_raw_test(
            &self, key: StorageKeyWithSpace,
        ) -> Result<Option<Arc<[u8]>>> {
            self.get_raw(key)
        }

        /// Set the value under `key` to `value` in `accessed_entries`.
        /// This method will read from db if `key` is not present.
        /// This method will also update the latest checkpoint if necessary.
        fn modify_single_value(
            &mut self, key: StorageKeyWithSpace, value: Option<Box<[u8]>>,
        ) -> Result<()> {
            let key_bytes = key.to_key_bytes();
            let mut entry =
                self.accessed_entries.get_mut().entry(key_bytes.clone());
            let value = value.map(Into::into);

            match &mut entry {
                Occupied(o) => {
                    // set `current_value` to `value` and keep the old value
                    Some(std::mem::replace(
                        &mut o.get_mut().current_value,
                        value,
                    ))
                }

                // Vacant
                &mut Vacant(_) => {
                    let original_value =
                        self.storage.view().get(key)?.map(Into::into);

                    entry.or_insert(EntryValue::new_modified(
                        original_value,
                        value,
                    ));

                    None
                }
            };

            Ok(())
        }

        pub(crate) fn set_raw(
            &mut self, key: StorageKeyWithSpace, value: Box<[u8]>,
            debug_record: Option<&mut ComputeEpochDebugRecord>,
        ) -> Result<()> {
            if let Some(record) = debug_record {
                record.state_ops.push(StateOp::StorageLevelOp {
                    op_name: "set".into(),
                    key: key.to_key_bytes(),
                    maybe_value: Some(value.clone().into()),
                })
            }

            self.modify_single_value(key, Some(value))
        }

        pub fn delete(
            &mut self, key: StorageKeyWithSpace,
            debug_record: Option<&mut ComputeEpochDebugRecord>,
        ) -> Result<()> {
            if let Some(record) = debug_record {
                record.state_ops.push(StateOp::StorageLevelOp {
                    op_name: "delete".into(),
                    key: key.to_key_bytes(),
                    maybe_value: None,
                })
            }

            self.modify_single_value(key, None)
        }

        pub fn read_all(
            &mut self, key_prefix: StorageKeyWithSpace,
            debug_record: Option<&mut ComputeEpochDebugRecord>,
        ) -> Result<Vec<MptKeyValue>> {
            self.delete_all::<access_mode::Read>(key_prefix, debug_record)
        }

        pub fn delete_all<AM: access_mode::AccessMode>(
            &mut self, key_prefix: StorageKeyWithSpace,
            debug_record: Option<&mut ComputeEpochDebugRecord>,
        ) -> Result<Vec<MptKeyValue>> {
            let key_bytes = key_prefix.to_key_bytes();
            if let Some(record) = debug_record {
                record.state_ops.push(StateOp::StorageLevelOp {
                    op_name: if AM::READ_ONLY {
                        "iterate"
                    } else {
                        "delete_all"
                    }
                    .into(),
                    key: key_bytes.clone(),
                    maybe_value: None,
                })
            }
            let accessed_entries = self.accessed_entries.get_mut();
            // First, all new keys in the subtree shall be deleted.
            let iter_range_upper_bound =
                to_key_prefix_iter_upper_bound(&key_bytes);
            let iter_range = match &iter_range_upper_bound {
                None => accessed_entries
                    .range_mut::<[u8], _>((Included(&*key_bytes), Unbounded)),

                // delete_all will not delete any key which doesn't exist before
                // the operation. Therefore we don't need to
                // check the accessed_entries prior to the
                // operation.
                Some(upper_bound) => accessed_entries.range_mut::<[u8], _>((
                    Included(&*key_bytes),
                    Excluded(&**upper_bound),
                )),
            };
            let mut deleted_kvs = vec![];
            for (k, v) in iter_range {
                if v.current_value != None {
                    deleted_kvs.push((
                        k.clone(),
                        (&**v.current_value.as_ref().unwrap()).into(),
                    ));
                    if !AM::READ_ONLY {
                        v.current_value = None;
                    }
                }
            }
            // Then, remove all un-modified existing keys.
            // We must update the accessed_entries.
            for (k, v) in self.storage.view().iter_prefix(key_prefix)? {
                let entry = accessed_entries.entry(k.clone());
                let was_vacant = if let Occupied(_) = &entry {
                    // Nothing to do for existing entry, because we have
                    // already scanned through accessed_entries.
                    false
                } else {
                    true
                };
                if was_vacant {
                    deleted_kvs.push((k.clone(), v.clone()));
                    if !AM::READ_ONLY {
                        entry.or_insert(EntryValue::new_modified(
                            Some((&*v).into()),
                            None,
                        ));
                    }
                }
            }

            Ok(deleted_kvs)
        }

        pub fn get_account_storage_entries(
            &mut self, address: &AddressWithSpace,
            debug_record: Option<&mut ComputeEpochDebugRecord>,
        ) -> Result<BTreeMap<cfx_types::StorageKey, cfx_types::StorageValue>>
        {
            let mut storage = BTreeMap::new();

            let storage_prefix =
                StorageKey::new_storage_root_key(&address.address)
                    .with_space(address.space);

            let kv_pairs = self.read_all(storage_prefix, debug_record)?;
            for (key, value) in kv_pairs {
                let storage_key_with_space =
                    StorageKeyWithSpace::from_key_bytes::<SkipInputCheck>(&key);
                if let StorageKey::StorageKey {
                    address_bytes: _,
                    storage_key,
                } = storage_key_with_space.key
                {
                    let h256_storage_key = H256::from_slice(storage_key);
                    let storage_value_with_owner: StorageValue =
                        rlp::decode(&value)?;
                    storage.insert(
                        h256_storage_key,
                        storage_value_with_owner.value,
                    );
                } else {
                    trace!("Not an storage key: {:?}", storage_key_with_space);
                    continue;
                }
            }

            Ok(storage)
        }

        /// Load the storage layout for state commits.
        /// Modification to storage layout is the same as modification of
        /// any other key-values. But as required by MPT structure we
        /// must commit storage layout for any storage changes under the
        /// same account. To load the storage layout, we first load from
        /// the local changes (i.e. accessed_entries), then from the
        /// storage if it's untouched.
        fn load_storage_layout(
            storage_layouts_to_rewrite: &mut HashMap<
                (Vec<u8>, Space),
                StorageLayout,
            >,
            accept_account_deletion: bool, address: &[u8], space: Space,
            storage: &dyn StorageView, accessed_entries: &AccessedEntries,
        ) -> Result<()> {
            if storage_layouts_to_rewrite
                .contains_key(&(address.to_vec(), space))
            {
                return Ok(());
            }
            let storage_layout_key =
                StorageKey::StorageRootKey(address).with_space(space);
            let current_storage_layout = match accessed_entries
                .get(&storage_layout_key.to_key_bytes())
            {
                Some(entry) => match &entry.current_value {
                    // We don't rewrite storage layout for account to
                    // delete.
                    None => {
                        if accept_account_deletion {
                            return Ok(());
                        } else {
                            // This is defensive checking, against certain
                            // cases when we are not deleting the account
                            // for sure.
                            bail!(Error::IncompleteDatabase(
                                Address::from_slice(address)
                            ));
                        }
                    }

                    Some(value_ref) => StorageLayout::from_bytes(&*value_ref)?,
                },
                None => match storage.get(storage_layout_key)? {
                    // A new account must set StorageLayout before accessing
                    // the storage.
                    None => bail!(Error::IncompleteDatabase(
                        Address::from_slice(address)
                    )),
                    Some(raw) => StorageLayout::from_bytes(raw.as_ref())?,
                },
            };
            storage_layouts_to_rewrite
                .insert((address.into(), space), current_storage_layout);

            Ok(())
        }

        pub fn set_storage_layout(
            &mut self, address: &AddressWithSpace,
            storage_layout: StorageLayout,
            debug_record: Option<&mut ComputeEpochDebugRecord>,
        ) -> Result<()> {
            self.set_raw(
                StorageKey::new_storage_root_key(&address.address)
                    .with_space(address.space),
                storage_layout.to_bytes().into_boxed_slice(),
                debug_record,
            )
        }

        /// storage_layout is special, because it must always present if there
        /// is any storage value changed. The entry goes into
        /// `accessed_entries` at the end of the cache building, so that it
        /// reaches the engine in the same changeset as everything else.
        ///
        /// The entry is marked `force_write` because the layout key must reach
        /// the underlying storage even when its value is unchanged, which is
        /// the common case: the layout is rewritten for every account whose
        /// storage changed, and most of them never touch the layout itself.
        fn insert_storage_layout_entry(
            &mut self, address: &[u8], space: Space, layout: &StorageLayout,
            debug_record: Option<&mut ComputeEpochDebugRecord>,
        ) {
            let key = StorageKey::StorageRootKey(address).with_space(space);
            let key_bytes = key.to_key_bytes();
            let value = layout.to_bytes().into_boxed_slice();
            if let Some(record) = debug_record {
                record.state_ops.push(StateOp::StorageLevelOp {
                    op_name: "commit_storage_layout".into(),
                    key: key_bytes.clone(),
                    maybe_value: Some(value.clone().into()),
                })
            };

            let value: Value = Some(value.into());
            match self.accessed_entries.get_mut().entry(key_bytes) {
                Occupied(mut o) => {
                    // The later write overrides the earlier one: whatever the
                    // execution left under this key, the layout value is the
                    // one replayed.
                    let entry = o.get_mut();
                    entry.current_value = value;
                    entry.force_write = true;
                }
                Vacant(v) => {
                    // The layout was loaded from the underlying storage, so
                    // the original value is the value inserted here.
                    let mut entry = EntryValue::new(value);
                    entry.force_write = true;
                    v.insert(entry);
                }
            }
        }

        /// Collect the storage layouts which have to be rewritten because of
        /// the modifications recorded in `accessed_entries`. This pass only
        /// reads: the layouts it loads from the underlying storage are under
        /// keys absent from `accessed_entries`, hence keys which the replay
        /// never writes.
        fn collect_storage_layouts_to_rewrite(
            &self,
        ) -> Result<HashMap<(Vec<u8>, Space), StorageLayout>> {
            let mut storage_layouts_to_rewrite = HashMap::new();
            let accessed_entries_guard = self.accessed_entries.read();
            let accessed_entries = &*accessed_entries_guard;
            for (k, v) in accessed_entries {
                if !v.is_modified() {
                    continue;
                }

                let storage_key =
                    StorageKeyWithSpace::from_key_bytes::<SkipInputCheck>(k);

                match &storage_key.key {
                    StorageKey::StorageKey { address_bytes, .. } => {
                        Self::load_storage_layout(
                            &mut storage_layouts_to_rewrite,
                            /* accept_account_deletion = */
                            v.current_value.is_none(),
                            address_bytes,
                            storage_key.space,
                            self.storage.view(),
                            &accessed_entries,
                        )?;
                    }
                    StorageKey::AccountKey(address_bytes)
                        if (address_bytes.is_builtin_address()
                            || address_bytes.is_contract_address()
                            || storage_key.space == Space::Ethereum)
                            && v.original_value.is_none() =>
                    {
                        let result = Self::load_storage_layout(
                            &mut storage_layouts_to_rewrite,
                            /* accept_account_deletion = */ false,
                            address_bytes,
                            storage_key.space,
                            self.storage.view(),
                            &accessed_entries,
                        );
                        if result.is_err() {
                            debug!(
                                "Contract address {:?} (space {:?}) is created without storage_layout. \
                                It's probably created by a balance transfer.",
                                Address::from_slice(address_bytes), storage_key.space
                            );
                        }
                    }
                    StorageKey::CodeKey { address_bytes, .. }
                        if v.original_value.is_none() =>
                    {
                        Self::load_storage_layout(
                            &mut storage_layouts_to_rewrite,
                            /* accept_account_deletion = */ false,
                            address_bytes,
                            storage_key.space,
                            self.storage.view(),
                            &accessed_entries,
                        )?;
                    }
                    _ => {}
                }
            }

            Ok(storage_layouts_to_rewrite)
        }

        /// Turn the whole cache into the changeset handed to the engine.
        ///
        /// This method does NOT clear the cache: `preview_genesis_root`
        /// computes a root without persisting anything, so the commit which
        /// follows it has to be able to build the changeset again. Clearing is
        /// the job of whoever actually persisted the changeset.
        fn build_changeset(
            &mut self, mut debug_record: Option<&mut ComputeEpochDebugRecord>,
        ) -> Result<Changeset> {
            let storage_layouts_to_rewrite =
                self.collect_storage_layouts_to_rewrite()?;
            // Set storage layout for contracts with storage modification or
            // contracts with storage_layout initialization or modification.
            // The entries go into the cache, they are not written to the
            // underlying storage here.
            for ((address, space), layout) in &storage_layouts_to_rewrite {
                self.insert_storage_layout_entry(
                    address,
                    *space,
                    layout,
                    debug_record.as_deref_mut(),
                );
            }

            let mut changeset = Changeset::new();
            let accessed_entries = &*self.accessed_entries.get_mut();
            for (k, v) in accessed_entries {
                if !v.should_write() {
                    continue;
                }

                changeset.insert(
                    k.clone(),
                    v.current_value.as_ref().map(|value| (&**value).into()),
                );
            }
            Ok(changeset)
        }

        /// The first phase of the genesis commit: the state root this
        /// instance's changeset would produce, computed before the epoch has
        /// an id, without persisting anything. The genesis construction needs
        /// the root to build the block header whose hash *is* the epoch id, so
        /// it cannot call `commit` yet.
        ///
        /// This deliberately leaves `accessed_entries` alone. Nothing was
        /// persisted, so the `commit` which follows has to be able to build
        /// the very same changeset a second time — clearing here would leave
        /// it with an empty one.
        pub fn preview_genesis_root(
            &mut self, debug_record: Option<&mut ComputeEpochDebugRecord>,
        ) -> Result<StateRoot> {
            let changeset = self.build_changeset(debug_record)?;
            match &self.storage {
                Storage::Commit { ctx, .. } => {
                    // The engine previews on the empty base without looking at
                    // the parent, so any other parent would silently get back
                    // a root computed on the wrong base.
                    if ctx.parent != StorageVersion::Empty {
                        return Err(Error::Msg(format!(
                            "preview_genesis_root needs the empty base as the \
                             parent, asked for {:?}",
                            ctx.parent
                        )));
                    }
                    ctx.manager
                        .clone()
                        .preview_genesis_root(&changeset)
                        .map_err(Into::into)
                }
                Storage::ReadOnly(_) => Err(Error::ReadOnlyStateDb),
            }
        }

        /// Hand the whole changeset to the engine and let it apply, hash and
        /// persist it under `epoch_id`. The answer is the consensus
        /// commitment, the state root triplet alone.
        pub fn commit(
            &mut self, epoch_id: EpochId,
            debug_record: Option<&mut ComputeEpochDebugRecord>,
        ) -> Result<StateRoot> {
            let changeset = self.build_changeset(debug_record)?;
            let state_root = match &mut self.storage {
                Storage::ReadOnly(_) => return Err(Error::ReadOnlyStateDb),
                Storage::Commit { ctx, .. } => ctx.manager.clone().commit(
                    ctx.parent,
                    changeset,
                    CommitMeta {
                        epoch_id,
                        height: ctx.height,
                    },
                )?,
            };
            // Mark all modification applied.
            self.accessed_entries = Default::default();
            Ok(state_root)
        }
    }

    struct EntryValue {
        original_value: Value,
        current_value: Value,
        /// Write this entry to the underlying storage even when the current
        /// value equals the original one. Only set for the storage layout
        /// entries, which the MPT structure requires to be written whenever
        /// any storage under the same account changes.
        force_write: bool,
    }

    impl EntryValue {
        fn new(value: Value) -> Self {
            let value_clone = value.clone();
            Self {
                original_value: value,
                current_value: value_clone,
                force_write: false,
            }
        }

        fn new_modified(original_value: Value, current_value: Value) -> Self {
            Self {
                original_value,
                current_value,
                force_write: false,
            }
        }

        fn is_modified(&self) -> bool {
            self.original_value.ne(&self.current_value)
        }

        fn should_write(&self) -> bool {
            self.force_write || self.is_modified()
        }
    }

    use super::*;
    use cfx_internal_common::debug::{ComputeEpochDebugRecord, StateOp};
    use cfx_storage::{
        utils::{access_mode, to_key_prefix_iter_upper_bound},
        Changeset, CommitMeta, MptKeyValue, OpenOptions, StorageEngine,
        StorageVersion, StorageView,
    };
    use cfx_types::{
        address_util::AddressUtil, Address, AddressWithSpace, Space,
    };
    use hashbrown::HashMap;
    use parking_lot::RwLock;
    use primitives::{
        EpochId, SkipInputCheck, StateRoot, StorageKey, StorageKeyWithSpace,
        StorageLayout,
    };
    use std::{
        collections::{
            btree_map::Entry::{Occupied, Vacant},
            BTreeMap,
        },
        ops::Bound::{Excluded, Included, Unbounded},
        sync::Arc,
    };
}
