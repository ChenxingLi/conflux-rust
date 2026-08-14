// Copyright 2019 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

pub type DeltaDbManager = DeltaDbManagerRocksdb;
pub type SnapshotDbManager = SnapshotDbManagerSqlite;
pub type SnapshotDb = <SnapshotDbManager as SnapshotDbManagerTrait>::SnapshotDb;

pub struct StateTrees {
    pub snapshot_db: SnapshotDb,
    pub snapshot_epoch_id: EpochId,
    pub snapshot_merkle_root: MerkleHash,
    /// None means that the intermediate_trie is empty, or in a special
    /// situation that we use the snapshot at intermediate epoch directly,
    /// so we don't need to look up intermediate trie.
    pub maybe_intermediate_trie: Option<Arc<DeltaMpt>>,
    pub intermediate_trie_root: Option<NodeRefDeltaMpt>,
    pub intermediate_trie_root_merkle: MerkleHash,
    /// A None value indicate the special case when snapshot_db is actually the
    /// snapshot_db from the intermediate_epoch_id.
    pub maybe_intermediate_trie_key_padding: Option<DeltaMptKeyPadding>,
    /// Delta trie can't be none since we may commit into it.
    pub delta_trie: Arc<DeltaMpt>,
    pub delta_trie_root: Option<NodeRefDeltaMpt>,
    pub delta_trie_key_padding: DeltaMptKeyPadding,
    /// Information for making new snapshot when necessary.
    pub maybe_delta_trie_height: Option<u32>,
    pub maybe_height: Option<u64>,
    pub intermediate_epoch_id: EpochId,

    // TODO: this field is added only for the hack to get pivot chain from a
    // TODO: snapshot to its parent snapshot.
    pub parent_epoch_id: EpochId,
}

#[derive(MallocSizeOfDerive)]
pub struct StateManager {
    storage_manager: Arc<StorageManager>,
    single_mpt_storage_manager: Option<Arc<SingleMptStorageManager>>,
    pub number_committed_nodes: AtomicUsize,
    /// A clone of the handle owned by `storage_manager`. `commit` writes into
    /// it and the open path resolves coordinates out of it, see
    /// `resolve_coordinates`.
    #[ignore_malloc_size_of = "rocksdb handle owned by StorageManager"]
    state_index_db: Arc<StateIndexDb>,
}

impl Drop for StateManager {
    fn drop(&mut self) { self.storage_manager.graceful_shutdown(); }
}

impl StateManager {
    pub fn new(conf: StorageConfiguration) -> Result<Self> {
        debug!("Storage conf {:?}", conf);
        // Make sure sqlite temp directory is using the data disk instead of the
        // system disk.
        std::env::set_var("SQLITE_TMPDIR", conf.path_snapshot_dir.clone());

        let single_mpt_storage_manager = if conf.enable_single_mpt_storage {
            Some(SingleMptStorageManager::new_arc(
                conf.path_storage_dir.join("single_mpt"),
                conf.single_mpt_space,
                conf.full_state_start_height().expect("enabled"),
                conf.single_mpt_cache_start_size,
                conf.single_mpt_cache_size,
                conf.single_mpt_slab_idle_size,
            ))
        } else {
            None
        };

        let storage_manager = StorageManager::new_arc(conf)?;
        let state_index_db = storage_manager.state_index_db();

        Ok(Self {
            storage_manager,
            single_mpt_storage_manager,
            number_committed_nodes: Default::default(),
            state_index_db,
        })
    }

    /// The engine's own state index.
    pub(crate) fn state_index_db(&self) -> &StateIndexDb {
        &self.state_index_db
    }

    /// Has the private index still to be rebuilt from what is on disk? The
    /// answer is the migration mark, which means "the index has been rebuilt
    /// once already". It is absent on the first boot of a binary carrying the
    /// index, and on any boot following a crash in the middle of the rebuild.
    ///
    /// Kept separate from `migrate_state_index` so that "this happens only on
    /// the first boot after the upgrade" is visible at the call site rather
    /// than buried in the engine.
    pub fn needs_state_index_migration(&self) -> Result<bool> {
        Ok(!self.state_index_db.migrated()?)
    }

    /// Rebuild the private index from what is on disk, and seed the physical
    /// openable lower bound from the same enumeration. Runs once, behind
    /// `needs_state_index_migration`.
    ///
    /// The layer coordinates of an entry come from the engine's own snapshot
    /// registry; the merkle roots come from the commitment row of the epoch,
    /// fetched through `state_root_of_epoch`. The engine never reaches the
    /// ledger db itself, because the column and the key layout are consensus
    /// schema and the row may not even live in a rocksdb.
    ///
    /// Entries have two sources. A height inside a period takes its snapshot
    /// layer and its intermediate layer from the two snapshots below that
    /// period, and is written only while at least one of the two is still on
    /// disk, since those are the two the open path can read through. A
    /// snapshot's own epoch takes the snapshot itself as that layer, and
    /// only where the first source produced nothing for it: at the bottom of
    /// what is on disk, where both layers below have gone, and on a disk which
    /// arrived by snapshot sync, where they were never downloaded. The merged
    /// image is the whole state of that epoch either way.
    ///
    /// An epoch whose row cannot be read gets no entry at all: not an empty
    /// root and not a default padding. An empty root is a legal value that
    /// derives a legal looking key padding, so an entry carrying it would read
    /// stale data rather than fail. The height is logged and skipped, and the
    /// epoch is re-executed like any other epoch without an entry.
    ///
    /// A height is openable only if this rebuild wrote it an entry and the
    /// layers that entry names are on disk, and neither implies the other. The
    /// published bound is therefore the higher of the two lower ends: the one
    /// the snapshots give, and the lowest height an entry was written for.
    /// Entries below the published bound are written all the same; they simply
    /// have no reader.
    ///
    /// Known gap: the current delta segment above the newest snapshot is not
    /// covered. It cannot be enumerated, and the replay this startup runs
    /// re-registers those epochs anyway.
    pub fn migrate_state_index(
        &self, state_root_of_epoch: &dyn Fn(&EpochId) -> Option<StateRoot>,
    ) -> Result<()> {
        // Reaching here with the mark set is a caller error, not a state to
        // repair: a rerun would republish a bound derived from a disk that
        // garbage collection has moved on from since.
        if self.state_index_db.migrated()? {
            return Err(Error::Msg(
                "the state index was already rebuilt on an earlier boot; \
                 the rebuild is one shot and belongs behind \
                 needs_state_index_migration()"
                    .into(),
            ));
        }

        let storage_manager = &self.storage_manager;
        let snapshot_epoch_count = storage_manager.get_snapshot_epoch_count();
        let all_snapshots = storage_manager.all_snapshot_infos();

        let mut executable_snapshots = all_snapshots
            .values()
            .filter(|info| {
                info.snapshot_info_kept_to_provide_sync
                    == SnapshotKeptToProvideSyncStatus::No
            })
            .cloned()
            .collect::<Vec<_>>();
        executable_snapshots.sort_by_key(|info| info.height);

        // One half of the published bound, from the snapshots alone and
        // before a single entry is written.
        let snapshot_lower_bound = physical_openable_lower_bound_of(
            &executable_snapshots,
            snapshot_epoch_count,
        );

        // The other half, filled in by the loop below.
        let mut lowest_entry_height: Option<u64> = None;
        let mut written = 0usize;
        let mut skipped_periods = 0usize;
        let mut skipped_heights = 0usize;

        for period_snapshot in executable_snapshots.iter() {
            let period_end = period_snapshot.height;
            let period_begin = period_snapshot.parent_snapshot_height;
            if period_end == 0 || period_end <= period_begin {
                // The genesis snapshot names no period.
                continue;
            }

            let intermediate_epoch_id =
                period_snapshot.parent_snapshot_epoch_id;
            let snapshot_epoch_id =
                match all_snapshots.get(&intermediate_epoch_id) {
                    Some(one_below) => one_below.parent_snapshot_epoch_id,
                    None => {
                        warn!(
                            "state index rebuild: the period ending at \
                             height {} names {:?} one period below, which the \
                             snapshot registry no longer holds, so the \
                             snapshot layer of the period cannot be \
                             resolved. \
                             Writing no entry for heights {}..={}.",
                            period_end,
                            intermediate_epoch_id,
                            period_begin + 1,
                            period_end,
                        );
                        skipped_periods += 1;
                        continue;
                    }
                };
            // The registry's own merkle root for the snapshot layer, which the
            // entries below are checked against. A layer the registry no
            // longer holds has no data either, so those entries are opened
            // through the intermediate layer and never name that file; the
            // check is then one which cannot be made, not a reason to leave
            // the heights uncovered.
            let maybe_registered_snapshot_merkle_root = all_snapshots
                .get(&snapshot_epoch_id)
                .map(|info| info.merkle_root);
            // Naming the two layers is not having them. The open path reads
            // the snapshot layer, and falls back to the snapshot at the
            // intermediate epoch when the first one is not on disk, so a
            // period with neither cannot be opened through any entry written
            // here. Its own snapshot is on disk all the same and gets an entry
            // below, from itself.
            let layer_data_on_disk = |epoch: &EpochId| {
                all_snapshots.get(epoch).map_or(false, |info| {
                    info.snapshot_info_kept_to_provide_sync
                        != SnapshotKeptToProvideSyncStatus::InfoOnly
                })
            };
            if !layer_data_on_disk(&snapshot_epoch_id)
                && !layer_data_on_disk(&intermediate_epoch_id)
            {
                warn!(
                    "state index rebuild: the period ending at height {} \
                     sits on snapshot {:?} over intermediate {:?}, and the \
                     registry says neither is still on disk. Writing no entry \
                     for heights {}..={}.",
                    period_end,
                    snapshot_epoch_id,
                    intermediate_epoch_id,
                    period_begin + 1,
                    period_end,
                );
                skipped_periods += 1;
                continue;
            }

            let maybe_intermediate_mpt_key_padding = if intermediate_epoch_id
                == NULL_EPOCH
            {
                // The layer below is the empty snapshot registered at genesis,
                // which is also this period's snapshot layer. There is no
                // intermediate MPT between the two, and `None` is what tells
                // the open path to read through to the snapshot instead of
                // looking a key up.
                None
            } else {
                match state_root_of_epoch(&intermediate_epoch_id) {
                    Some(roots) => Some(delta_mpt_padding(
                        &roots.snapshot_root,
                        &roots.intermediate_delta_root,
                    )),
                    None => {
                        warn!(
                            "state index rebuild: no commitment row for \
                             {:?}, the epoch one period below the period \
                             ending at height {}, so the intermediate key \
                             padding of that period is unknown. Writing no \
                             entry for heights {}..={}.",
                            intermediate_epoch_id,
                            period_end,
                            period_begin + 1,
                            period_end,
                        );
                        skipped_periods += 1;
                        continue;
                    }
                }
            };

            for height in (period_begin + 1)..=period_end {
                let epoch_id =
                    match period_snapshot.get_epoch_id_at_height(height) {
                        Some(id) => *id,
                        None => {
                            warn!(
                                "state index rebuild: the registry entry of \
                                 the snapshot at height {} names no epoch at \
                                 height {}. Writing no entry for it.",
                                period_end, height,
                            );
                            skipped_heights += 1;
                            continue;
                        }
                    };
                let Some(roots) = state_root_of_epoch(&epoch_id) else {
                    warn!(
                        "state index rebuild: no commitment row for epoch \
                         {:?} at height {}, so its state roots are unknown. \
                         Writing no entry for it; consensus re-executes the \
                         height.",
                        epoch_id, height,
                    );
                    skipped_heights += 1;
                    continue;
                };
                // The registry names the snapshot layer, the row carries its
                // merkle root. Disagreement means the two halves of the entry
                // describe different states, and the key paddings derived from
                // the row would address the wrong keys of the delta MPT.
                if maybe_registered_snapshot_merkle_root
                    .is_some_and(|registered| roots.snapshot_root != registered)
                {
                    warn!(
                        "state index rebuild: the commitment row of epoch \
                         {:?} at height {} carries snapshot root {:?}, while \
                         the snapshot layer the registry names for it, {:?}, \
                         has merkle root {:?}. Writing no entry for it.",
                        epoch_id,
                        height,
                        roots.snapshot_root,
                        snapshot_epoch_id,
                        maybe_registered_snapshot_merkle_root,
                    );
                    skipped_heights += 1;
                    continue;
                }

                let entry = StateIndexEntry {
                    snapshot_epoch_id,
                    snapshot_merkle_root: roots.snapshot_root,
                    intermediate_epoch_id,
                    intermediate_delta_root: roots.intermediate_delta_root,
                    maybe_intermediate_mpt_key_padding:
                        maybe_intermediate_mpt_key_padding.clone(),
                    delta_mpt_key_padding: delta_mpt_padding(
                        &roots.snapshot_root,
                        &roots.intermediate_delta_root,
                    ),
                    delta_root: roots.delta_root,
                    maybe_height: Some(height),
                    maybe_delta_trie_height: Some(
                        StateIndex::height_to_delta_height(
                            height,
                            snapshot_epoch_count,
                        ),
                    ),
                };
                self.state_index_db.put(&epoch_id, &entry)?;
                written += 1;
                lowest_entry_height = Some(
                    lowest_entry_height
                        .map_or(height, |lowest| lowest.min(height)),
                );
            }
        }

        // A period the loop could not name leaves its own snapshot's epoch
        // without an entry, and on a disk which arrived by snapshot sync that
        // epoch is the one execution restarts from. Such a disk holds the
        // merged image and nothing below it, so the two snapshots the loop
        // walks down to were never there to be walked to.
        //
        // A snapshot holds the whole state of its own epoch, so
        // that epoch is its own snapshot layer here, and the entry is the one
        // `register_synced_snapshot_state` writes for a landing the new code
        // syncs itself. It goes out only where the loop wrote nothing: where
        // the loop did write, the layers below are on disk and the layered
        // entry is what `commit` would have left, which this shape is not.
        for period_snapshot in executable_snapshots.iter() {
            let epoch_id = *period_snapshot.get_snapshot_epoch_id();
            if period_snapshot.height == 0
                || self.state_index_db.get(&epoch_id)?.is_some()
            {
                continue;
            }
            let Some(roots) = state_root_of_epoch(&epoch_id) else {
                warn!(
                    "state index rebuild: no commitment row for epoch {:?}, \
                     whose state the snapshot at height {} holds, so its \
                     state roots are unknown. Writing no entry for it.",
                    epoch_id, period_snapshot.height,
                );
                skipped_heights += 1;
                continue;
            };
            let entry = StateIndexEntry {
                snapshot_epoch_id: epoch_id,
                snapshot_merkle_root: roots.snapshot_root,
                intermediate_epoch_id: period_snapshot.parent_snapshot_epoch_id,
                intermediate_delta_root: roots.intermediate_delta_root,
                maybe_intermediate_mpt_key_padding: None,
                delta_mpt_key_padding: delta_mpt_padding(
                    &roots.snapshot_root,
                    &roots.intermediate_delta_root,
                ),
                delta_root: roots.delta_root,
                maybe_height: Some(period_snapshot.height),
                maybe_delta_trie_height: Some(
                    StateIndex::height_to_delta_height(
                        period_snapshot.height,
                        snapshot_epoch_count,
                    ),
                ),
            };
            self.state_index_db.put(&epoch_id, &entry)?;
            written += 1;
            lowest_entry_height = Some(
                lowest_entry_height.map_or(period_snapshot.height, |lowest| {
                    lowest.min(period_snapshot.height)
                }),
            );
        }

        // With no entry at all there is no second half to take. Publishing
        // the snapshot half then claims heights no entry can open, but a
        // rebuild that wrote nothing has left every height in that state, and
        // each of them is refused at the index lookup.
        let lower_bound = match lowest_entry_height {
            Some(height) => snapshot_lower_bound.max(height),
            None => snapshot_lower_bound,
        };
        // The mark is what keeps the rebuild from running a second time, and
        // it is also the only boot on which the rebuild can still happen. A
        // run which wrote no entry at all, on a disk whose registry holds a
        // snapshot to write one for, has not rebuilt anything; marking it done
        // would spend that one boot and leave the node with an index it can
        // never fill. It stays unmarked and this startup fails instead.
        if written == 0
            && executable_snapshots.iter().any(|info| info.height > 0)
        {
            return Err(Error::Msg(format!(
                "the snapshot registry holds {} executable snapshots and not \
                 one entry could be written, {} periods and {} single heights \
                 left out; the rebuild stays unmarked so that a later boot \
                 can run it again",
                executable_snapshots.len(),
                skipped_periods,
                skipped_heights,
            )));
        }
        // Seeding only. From here on the bound is maintained by garbage
        // collection inside `StorageManager`, on the snapshot half of the
        // formula alone: every height above this bound has an entry.
        storage_manager.set_physical_openable_lower_bound(lower_bound)?;
        self.state_index_db.set_migrated()?;
        let lowest_entry_height_text = match lowest_entry_height {
            Some(height) => height.to_string(),
            None => "none, no entry was written".into(),
        };
        info!(
            "state index rebuild: {} entries written, physical openable \
             lower bound {}; {} periods and {} single heights left out. The \
             bound is the higher of the lower end the snapshots give, {}, and \
             the lowest height an entry was written for, {}.",
            written,
            lower_bound,
            skipped_periods,
            skipped_heights,
            snapshot_lower_bound,
            lowest_entry_height_text,
        );
        Ok(())
    }

    pub fn log_usage(&self) {
        self.storage_manager.log_usage();
        debug!(
            "number of nodes committed to db {}",
            self.number_committed_nodes.load(Ordering::Relaxed),
        );
    }

    pub fn get_storage_manager(&self) -> &StorageManager {
        &*self.storage_manager
    }

    pub fn get_storage_manager_arc(&self) -> &Arc<StorageManager> {
        &self.storage_manager
    }

    /// The physical coordinates of one epoch, read out of the engine's own
    /// index. Only the epoch id comes from the caller; every other field of
    /// the returned `StateIndex` is the entry's.
    ///
    /// `Ok(None)` when the epoch has no entry, which every caller treats as
    /// "this version is not available". Epochs without an entry exist by
    /// design: the first boot migration leaves the current delta segment above
    /// the newest snapshot uncovered, and a crash between a db commit and its
    /// index write leaves the same kind of hole. Both heal by re-executing
    /// that segment.
    fn resolve_coordinates(
        &self, epoch_id: &EpochId,
    ) -> Result<Option<StateIndex>> {
        let Some(entry) = self.state_index_db.get(epoch_id)? else {
            warn!(
                "resolve_coordinates, no index entry for epoch {:?}.",
                epoch_id,
            );
            return Ok(None);
        };

        Ok(Some(StateIndex {
            snapshot_epoch_id: entry.snapshot_epoch_id,
            snapshot_merkle_root: entry.snapshot_merkle_root,
            intermediate_epoch_id: entry.intermediate_epoch_id,
            intermediate_trie_root_merkle: entry.intermediate_delta_root,
            maybe_intermediate_mpt_key_padding: entry
                .maybe_intermediate_mpt_key_padding,
            epoch_id: *epoch_id,
            delta_mpt_key_padding: entry.delta_mpt_key_padding,
            maybe_delta_trie_height: entry.maybe_delta_trie_height,
            maybe_height: entry.maybe_height,
        }))
    }

    /// Snapshot H is normally produced here, by merging snapshot H-1, the
    /// previous one a period below, with the intermediate MPT of the epochs
    /// between them. An entry saying otherwise says snapshot H arrived whole
    /// instead, so epoch H has no layers of its own: one image holds the
    /// state, there is no intermediate layer, and the delta layer is empty.
    /// The values read back the same; the three layers do not, so the layered
    /// open refuses the epoch.
    ///
    /// An epoch with no entry answers false, and the open itself reports the
    /// version as unavailable.
    fn is_unlayered(&self, epoch_id: &EpochId) -> Result<bool> {
        Ok(match self.state_index_db.get(epoch_id)? {
            Some(entry) => entry.snapshot_epoch_id == *epoch_id,
            None => false,
        })
    }

    /// delta_mpt_key_padding is required. When None is passed,
    /// it's calculated for the state_trees.
    #[inline]
    pub fn get_state_trees_internal(
        snapshot_db: SnapshotDb, snapshot_epoch_id: &EpochId,
        snapshot_merkle_root: MerkleHash,
        maybe_intermediate_trie: Option<Arc<DeltaMpt>>,
        maybe_intermediate_trie_key_padding: Option<&DeltaMptKeyPadding>,
        intermediate_epoch_id: &EpochId,
        intermediate_trie_root_merkle: MerkleHash, delta_mpt: Arc<DeltaMpt>,
        maybe_delta_mpt_key_padding: Option<&DeltaMptKeyPadding>,
        epoch_id: &EpochId, delta_root: Option<NodeRefDeltaMpt>,
        maybe_height: Option<u64>, maybe_delta_trie_height: Option<u32>,
    ) -> Result<Option<StateTrees>> {
        let intermediate_trie_root = match &maybe_intermediate_trie {
            None => None,
            Some(mpt) => {
                match mpt.get_root_node_ref_by_epoch(intermediate_epoch_id)? {
                    None => {
                        warn!(
                            "get_state_trees_internal, intermediate_mpt root not found \
                             for epoch {:?}.",
                            intermediate_epoch_id,
                        );
                        return Ok(None);
                    }
                    Some(root) => root,
                }
            }
        };

        let delta_trie_key_padding = match maybe_delta_mpt_key_padding {
            Some(x) => x.clone(),
            None => {
                // TODO: maybe we can move the calculation to a central place
                // and cache the result?
                delta_mpt_padding(
                    &snapshot_merkle_root,
                    &intermediate_trie_root_merkle,
                )
            }
        };

        Ok(Some(StateTrees {
            snapshot_db,
            snapshot_merkle_root,
            snapshot_epoch_id: *snapshot_epoch_id,
            maybe_intermediate_trie,
            intermediate_trie_root,
            intermediate_trie_root_merkle,
            maybe_intermediate_trie_key_padding:
                maybe_intermediate_trie_key_padding.cloned(),
            delta_trie: delta_mpt,
            delta_trie_root: delta_root,
            delta_trie_key_padding,
            maybe_delta_trie_height,
            maybe_height,
            intermediate_epoch_id: intermediate_epoch_id.clone(),
            parent_epoch_id: epoch_id.clone(),
        }))
    }

    pub fn get_state_trees(
        &self, epoch_id: &EpochId, try_open: bool, open_mpt_snapshot: bool,
    ) -> Result<Option<StateTrees>> {
        let Some(resolved) = self.resolve_coordinates(epoch_id)? else {
            return Ok(None);
        };
        let state_index = &resolved;

        let maybe_intermediate_mpt;
        let maybe_intermediate_mpt_key_padding;
        let delta_mpt;
        let snapshot;

        match self.storage_manager.wait_for_snapshot(
            &state_index.snapshot_epoch_id,
            try_open,
            open_mpt_snapshot,
        )? {
            None => {
                // This is the special scenario when the snapshot isn't
                // available but the snapshot at the intermediate epoch exists.
                if let Some(guarded_snapshot) =
                    self.storage_manager.wait_for_snapshot(
                        &state_index.intermediate_epoch_id,
                        try_open,
                        open_mpt_snapshot,
                    )?
                {
                    snapshot = guarded_snapshot;
                    maybe_intermediate_mpt = None;
                    maybe_intermediate_mpt_key_padding = None;
                    delta_mpt = match self
                        .storage_manager
                        .get_intermediate_mpt(
                            &state_index.intermediate_epoch_id,
                        )? {
                        None => {
                            warn!(
                                    "get_state_trees, special case, \
                                    intermediate_mpt not found for epoch {:?}. StateIndex: {:?}.",
                                    state_index.intermediate_epoch_id,
                                    state_index,
                                );
                            return Ok(None);
                        }
                        Some(delta_mpt) => delta_mpt,
                    };
                } else {
                    warn!(
                        "get_state_trees, special case, \
                         snapshot not found for epoch {:?}. StateIndex: {:?}.",
                        state_index.intermediate_epoch_id, state_index,
                    );
                    return Ok(None);
                }
            }
            Some(guarded_snapshot) => {
                snapshot = guarded_snapshot;
                maybe_intermediate_mpt_key_padding =
                    state_index.maybe_intermediate_mpt_key_padding.as_ref();
                maybe_intermediate_mpt = if maybe_intermediate_mpt_key_padding
                    .is_some()
                {
                    self.storage_manager
                        .get_intermediate_mpt(&state_index.snapshot_epoch_id)?
                } else {
                    None
                };
                delta_mpt = self
                    .storage_manager
                    .get_delta_mpt(&state_index.snapshot_epoch_id)?;
            }
        }

        // An epoch which is its own snapshot layer is the base of that
        // snapshot's delta MPT, so its delta layer is empty by construction
        // and there is no root to look up. A snapshot boundary epoch has two
        // equally valid decompositions of the same state, and this is the one
        // where the snapshot layer has already been promoted and the delta
        // layer is still empty; `get_state_trees_for_next_epoch` builds the
        // very same shape for the child of a boundary epoch, where it sets
        // `new_delta_root` and skips the lookup for the same reason.
        let delta_root = if state_index.epoch_id
            == state_index.snapshot_epoch_id
        {
            None
        } else {
            match delta_mpt.get_root_node_ref_by_epoch(&state_index.epoch_id)? {
                None => {
                    debug!(
                        "get_state_trees, \
                        delta_root not found for epoch {:?}. mpt_id {}, StateIndex: {:?}.",
                        state_index.epoch_id, delta_mpt.get_mpt_id(), state_index,
                    );
                    return Ok(None);
                }
                Some(root) => root,
            }
        };

        Self::get_state_trees_internal(
            snapshot.into().1,
            &state_index.snapshot_epoch_id,
            state_index.snapshot_merkle_root,
            maybe_intermediate_mpt,
            maybe_intermediate_mpt_key_padding,
            &state_index.intermediate_epoch_id,
            state_index.intermediate_trie_root_merkle,
            delta_mpt,
            Some(&state_index.delta_mpt_key_padding),
            &state_index.epoch_id,
            delta_root,
            state_index.maybe_height,
            state_index.maybe_delta_trie_height,
        )
    }

    pub fn get_state_trees_for_next_epoch(
        &self, parent_epoch_id: &EpochId, try_open: bool,
        open_mpt_snapshot: bool,
    ) -> Result<Option<StateTrees>> {
        let Some(resolved) = self.resolve_coordinates(parent_epoch_id)? else {
            return Ok(None);
        };
        let parent_state_index = &resolved;

        let maybe_height = parent_state_index.maybe_height.map(|x| x + 1);

        let snapshot;
        let snapshot_epoch_id;
        let snapshot_merkle_root;
        let maybe_delta_trie_height;
        let maybe_intermediate_mpt;
        let maybe_intermediate_mpt_key_padding;
        let intermediate_trie_root_merkle;
        let delta_mpt;
        let maybe_delta_mpt_key_padding;
        let intermediate_epoch_id;
        let new_delta_root;

        if parent_state_index
            .maybe_delta_trie_height
            .unwrap_or_default()
            == self.storage_manager.get_snapshot_epoch_count()
        {
            // Should shift to a new snapshot
            // When the delta_height is set to None (e.g. in tests), we
            // assume that the snapshot shift check is
            // disabled.

            snapshot_epoch_id = &parent_state_index.intermediate_epoch_id;
            intermediate_epoch_id = &parent_state_index.epoch_id;
            match self.storage_manager.wait_for_snapshot(
                snapshot_epoch_id,
                try_open,
                open_mpt_snapshot,
            )? {
                None => {
                    // This is the special scenario when the snapshot isn't
                    // available but the snapshot at the intermediate epoch
                    // exists.
                    //
                    // At the synced snapshot, the intermediate_epoch_id is
                    // its parent snapshot. We need to shift again.

                    // There is no snapshot_info for the parent snapshot,
                    // how can we find out the snapshot_merkle_root?
                    // See validate_blame_states().
                    match self.storage_manager.wait_for_snapshot(
                        &intermediate_epoch_id,
                        try_open,
                        open_mpt_snapshot,
                    )? {
                        None => {
                            warn!(
                                "get_state_trees_for_next_epoch, shift snapshot, special case, \
                                snapshot not found for snapshot {:?}. StateIndex: {:?}.",
                                parent_state_index.epoch_id,
                                parent_state_index,
                            );
                            return Ok(None);
                        }
                        Some(guarded_snapshot) => {
                            let (guard, _snapshot) = guarded_snapshot.into();
                            snapshot_merkle_root =
                                match StorageManager::find_merkle_root(
                                    &guard,
                                    snapshot_epoch_id,
                                ) {
                                    None => {
                                        warn!(
                                            "get_state_trees_for_next_epoch, shift snapshot, special case, \
                                            snapshot merkel root not found for snapshot {:?}. StateIndex: {:?}.",
                                            snapshot_epoch_id,
                                            parent_state_index,
                                        );
                                        return Ok(None);
                                    }
                                    Some(merkle_root) => merkle_root,
                                };

                            snapshot = GuardedValue::new(guard, _snapshot);
                        }
                    }
                    maybe_intermediate_mpt = None;
                    maybe_intermediate_mpt_key_padding = None;
                    match self
                        .storage_manager
                        .intermediate_trie_root_merkle
                        .write()
                        .take()
                    {
                        Some(v) => {
                            intermediate_trie_root_merkle = v;
                        }
                        _ => {
                            warn!(
                                "get_state_trees_for_next_epoch, shift snapshot, special case, \
                                intermediate_trie_root_merkle not found for snapshot {:?}. StateIndex: {:?}.",
                                snapshot_epoch_id,
                                parent_state_index,
                            );
                            return Ok(None);
                        }
                    }

                    debug!("get_state_trees_for_next_epoch, snapshot_merkle_root {:?}, intermediate_trie_root_merkle {:?}", snapshot_merkle_root, intermediate_trie_root_merkle);

                    match self
                        .storage_manager
                        .get_intermediate_mpt(&parent_state_index.epoch_id)?
                    {
                        None => {
                            warn!(
                                "get_state_trees_for_next_epoch, shift snapshot, special case, \
                                intermediate_mpt not found for snapshot {:?}. StateIndex: {:?}.",
                                parent_state_index.epoch_id,
                                parent_state_index,
                            );
                            return Ok(None);
                        }
                        Some(mpt) => delta_mpt = mpt,
                    }
                }
                Some(guarded_snapshot) => {
                    let (guard, _snapshot) = guarded_snapshot.into();
                    snapshot_merkle_root =
                        match StorageManager::find_merkle_root(
                            &guard,
                            snapshot_epoch_id,
                        ) {
                            None => {
                                warn!(
                                "get_state_trees_for_next_epoch, shift snapshot, normal case, \
                                snapshot info not found for snapshot {:?}. StateIndex: {:?}.",
                                snapshot_epoch_id,
                                parent_state_index,
                            );
                                return Ok(None);
                            }
                            Some(merkle_root) => merkle_root,
                        };
                    let guarded_snapshot = GuardedValue::new(guard, _snapshot);
                    let temp_maybe_intermediate_mpt = self
                        .storage_manager
                        .get_intermediate_mpt(snapshot_epoch_id)?;
                    match temp_maybe_intermediate_mpt {
                        None => {
                            snapshot = guarded_snapshot;
                            maybe_intermediate_mpt_key_padding =
                                Some(&parent_state_index.delta_mpt_key_padding);
                            delta_mpt = self
                                .storage_manager
                                .get_delta_mpt(&snapshot_epoch_id)?;
                            intermediate_trie_root_merkle = MERKLE_NULL_NODE;
                            maybe_intermediate_mpt = None;
                        }
                        Some(mpt) => match mpt.get_merkle_root_by_epoch_id(
                            &parent_state_index.epoch_id,
                        )? {
                            Some(merkle_root) => {
                                snapshot = guarded_snapshot;
                                maybe_intermediate_mpt_key_padding = Some(
                                    &parent_state_index.delta_mpt_key_padding,
                                );
                                delta_mpt = self
                                    .storage_manager
                                    .get_delta_mpt(&snapshot_epoch_id)?;
                                intermediate_trie_root_merkle = merkle_root;
                                maybe_intermediate_mpt = Some(mpt);
                            }
                            None => {
                                warn!(
                                        "get_state_trees_for_next_epoch, shift snapshot, normal case, \
                                        intermediate_trie_root not found for epoch {:?}. StateIndex: {:?}.",
                                        parent_state_index.epoch_id,
                                        parent_state_index,
                                    );

                                // Check if we should progress with a synced
                                // state.
                                // This is a quick fix for
                                // https://github.com/Conflux-Chain/conflux-rust/issues/1543.
                                // FIXME: We should remove all the hacks about
                                // state sync.
                                match self.storage_manager.wait_for_snapshot(
                                    &parent_state_index.epoch_id,
                                    try_open,
                                    open_mpt_snapshot,
                                )? {
                                    None => {
                                        warn!(
                                            "get_state_trees_for_next_epoch, shift snapshot, special case, \
                                snapshot not found for snapshot {:?}. StateIndex: {:?}.",
                                            parent_state_index.epoch_id,
                                            parent_state_index,
                                        );
                                        return Ok(None);
                                    }
                                    Some(guarded_synced_snapshot) => {
                                        snapshot = guarded_synced_snapshot;
                                    }
                                }
                                maybe_intermediate_mpt = None;
                                maybe_intermediate_mpt_key_padding = None;
                                match self
                                    .storage_manager
                                    .intermediate_trie_root_merkle
                                    .write()
                                    .take()
                                {
                                    Some(v) => {
                                        intermediate_trie_root_merkle = v;
                                    }
                                    _ => {
                                        warn!("get_state_trees_for_next_epoch, shift snapshot, special case, \
                                        intermediate_trie_root_merkle not found for snapshot {:?}. StateIndex: {:?}.", snapshot_epoch_id, parent_state_index,);
                                        return Ok(None);
                                    }
                                }

                                match self
                                    .storage_manager
                                    .get_intermediate_mpt(
                                        &parent_state_index.epoch_id,
                                    )? {
                                    None => {
                                        warn!(
                                            "get_state_trees_for_next_epoch, shift snapshot, special case, \
                                intermediate_mpt not found for snapshot {:?}. StateIndex: {:?}.",
                                            parent_state_index.epoch_id,
                                            parent_state_index,
                                        );
                                        return Ok(None);
                                    }
                                    Some(mpt) => delta_mpt = mpt,
                                }
                            }
                        },
                    };
                }
            };
            maybe_delta_mpt_key_padding = None;
            maybe_delta_trie_height = Some(1);
            new_delta_root = true;
        } else {
            snapshot_epoch_id = &parent_state_index.snapshot_epoch_id;
            snapshot_merkle_root = parent_state_index.snapshot_merkle_root;
            intermediate_epoch_id = &parent_state_index.intermediate_epoch_id;
            intermediate_trie_root_merkle =
                parent_state_index.intermediate_trie_root_merkle;
            match self.storage_manager.wait_for_snapshot(
                snapshot_epoch_id,
                try_open,
                open_mpt_snapshot,
            )? {
                None => {
                    // This is the special scenario when the snapshot isn't
                    // available but the snapshot at the intermediate epoch
                    // exists.
                    if let Some(guarded_snapshot) =
                        self.storage_manager.wait_for_snapshot(
                            &intermediate_epoch_id,
                            try_open,
                            open_mpt_snapshot,
                        )?
                    {
                        snapshot = guarded_snapshot;
                        maybe_intermediate_mpt = None;
                        maybe_intermediate_mpt_key_padding = None;
                        delta_mpt = match self
                            .storage_manager
                            .get_intermediate_mpt(intermediate_epoch_id)?
                        {
                            None => {
                                return {
                                    warn!(
                                    "get_state_trees_for_next_epoch, special case, \
                                    intermediate_mpt not found for epoch {:?}. StateIndex: {:?}.",
                                    intermediate_epoch_id,
                                    parent_state_index,
                                );
                                    Ok(None)
                                }
                            }
                            Some(delta_mpt) => delta_mpt,
                        };
                    } else {
                        warn!(
                            "get_state_trees_for_next_epoch, special case, \
                            snapshot not found for epoch {:?}. StateIndex: {:?}.",
                            intermediate_epoch_id,
                            parent_state_index,
                        );
                        return Ok(None);
                    }
                }
                Some(guarded_snapshot) => {
                    snapshot = guarded_snapshot;
                    maybe_intermediate_mpt_key_padding = parent_state_index
                        .maybe_intermediate_mpt_key_padding
                        .as_ref();
                    maybe_intermediate_mpt =
                        if maybe_intermediate_mpt_key_padding.is_some() {
                            self.storage_manager
                                .get_intermediate_mpt(snapshot_epoch_id)?
                        } else {
                            None
                        };
                    delta_mpt = self
                        .storage_manager
                        .get_delta_mpt(snapshot_epoch_id)?;
                }
            };
            maybe_delta_trie_height =
                parent_state_index.maybe_delta_trie_height.map(|x| x + 1);
            maybe_delta_mpt_key_padding =
                Some(&parent_state_index.delta_mpt_key_padding);
            new_delta_root = false;
        };

        let delta_root = if new_delta_root {
            None
        } else {
            match delta_mpt
                .get_root_node_ref_by_epoch(&parent_state_index.epoch_id)?
            {
                None => {
                    warn!(
                        "get_state_trees_for_next_epoch, not shifting, \
                         delta_root not found for epoch {:?}. mpt_id {}, StateIndex: {:?}.",
                        parent_state_index.epoch_id,
                        delta_mpt.get_mpt_id(), parent_state_index
                    );
                    return Ok(None);
                }
                Some(root_node) => root_node,
            }
        };
        Self::get_state_trees_internal(
            snapshot.into().1,
            snapshot_epoch_id,
            snapshot_merkle_root,
            maybe_intermediate_mpt,
            maybe_intermediate_mpt_key_padding,
            intermediate_epoch_id,
            intermediate_trie_root_merkle,
            delta_mpt,
            maybe_delta_mpt_key_padding,
            &parent_state_index.epoch_id,
            delta_root,
            maybe_height,
            maybe_delta_trie_height,
        )
    }

    /// Check if we can make a new snapshot, and if so, make it in background.
    pub fn check_make_snapshot(
        &self, maybe_intermediate_trie: Option<Arc<DeltaMpt>>,
        intermediate_trie_root: Option<NodeRefDeltaMpt>,
        intermediate_epoch_id: &EpochId, new_height: u64,
        recover_mpt_during_construct_pivot_state: bool,
    ) -> Result<()> {
        StorageManager::check_make_register_snapshot_background(
            self.storage_manager.clone(),
            intermediate_epoch_id.clone(),
            new_height,
            maybe_intermediate_trie.map(|intermediate_trie| DeltaMptIterator {
                mpt: intermediate_trie,
                maybe_root_node: intermediate_trie_root,
            }),
            recover_mpt_during_construct_pivot_state,
        )
    }

    /// Open one version of the layered state: `open_state` without the single
    /// MPT half. The proof paths and the engine specific queries need the
    /// concrete `State`, which a `SingleMptState` cannot stand in for.
    ///
    /// `open_mpt_snapshot` opens the snapshot's MPT table alongside its key
    /// value table. Only the callers of this entry point ever ask for it,
    /// because only proofs and per layer node merkle values read that table,
    /// so it is a parameter here instead of a field of `OpenOptions`.
    pub fn open_layered_state(
        self: &Arc<Self>, epoch_id: &EpochId, opts: OpenOptions,
        open_mpt_snapshot: bool,
    ) -> Result<Option<State>> {
        // The read only half of this entry point refuses an unlayered epoch.
        // The concrete state object answers per layer, and an epoch which
        // arrived as one merged snapshot image does not decompose the way its
        // block header commits to. Opening it as the execution base of its
        // child epoch is a different question and is not refused: the child
        // reads values, which the merged image answers correctly.
        if matches!(opts.mode, OpenMode::ReadOnly)
            && self.is_unlayered(epoch_id)?
        {
            return Ok(None);
        }
        let (maybe_state_trees, recover_mpt_during_construct_pivot_state) =
            match opts.mode {
                OpenMode::ReadOnly => (
                    self.get_state_trees(
                        epoch_id,
                        opts.try_open,
                        open_mpt_snapshot,
                    )?,
                    false,
                ),
                OpenMode::NextEpochBase {
                    recover_mpt_during_construct_pivot_state,
                } => (
                    self.get_state_trees_for_next_epoch(
                        epoch_id,
                        opts.try_open,
                        open_mpt_snapshot,
                    )?,
                    recover_mpt_during_construct_pivot_state,
                ),
            };
        match maybe_state_trees {
            None => Ok(None),
            Some(state_trees) => Ok(Some(State::new(
                self.clone(),
                state_trees,
                recover_mpt_during_construct_pivot_state,
            ))),
        }
    }

    fn get_state_for_genesis_write_inner(self: &Arc<Self>) -> State {
        State::new(
            self.clone(),
            StateTrees {
                snapshot_db: self
                    .storage_manager
                    .wait_for_snapshot(
                        &NULL_EPOCH,
                        /* try_open = */ false,
                        true,
                    )
                    .unwrap()
                    .unwrap()
                    .into()
                    .1,
                snapshot_epoch_id: NULL_EPOCH,
                snapshot_merkle_root: MERKLE_NULL_NODE,
                maybe_intermediate_trie: None,
                intermediate_trie_root: None,
                intermediate_trie_root_merkle: MERKLE_NULL_NODE,
                maybe_intermediate_trie_key_padding: None,
                delta_trie: self
                    .storage_manager
                    .get_delta_mpt(&NULL_EPOCH)
                    .unwrap(),
                delta_trie_root: None,
                delta_trie_key_padding: GENESIS_DELTA_MPT_KEY_PADDING.clone(),
                maybe_delta_trie_height: Some(1),
                maybe_height: Some(1),
                intermediate_epoch_id: NULL_EPOCH,
                parent_epoch_id: NULL_EPOCH,
            },
            false,
        )
    }

    /// Tell the engine that the chain has confirmed the state at
    /// `view.confirmed_height()`, its cue to reclaim the states and snapshots
    /// it no longer needs. The call is synchronous, so an open or a commit
    /// issued after it sees a state consistent with the reclamation.
    ///
    /// `view` is read only and is not kept past this call. It carries the
    /// confirmed height, the retention parameters the engine's own rules are
    /// stated in, and the lookup from a height to its pivot chain epoch, which
    /// is the one thing the engine cannot answer for itself.
    ///
    /// Returns the physical openable lower bound left behind, which consensus
    /// needs in order to move its own bound and drop the prefix of the pivot
    /// hash mirror indexed from it.
    ///
    /// An error on the way is logged rather than returned; it surfaces at a
    /// later open or commit, and the next confirmation retries the round. The
    /// bound is still reported, straight off the record, because a failed
    /// round leaves the record describing the disk just as a successful one
    /// does.
    pub fn notify_state_confirmed(&self, view: &dyn StateConfirmedView) -> u64 {
        match self.storage_manager.maintain_state_confirmed(view) {
            Ok(physical_openable_lower_bound) => physical_openable_lower_bound,
            Err(e) => {
                warn!(
                    "state confirmed at height {}: maintenance failed, {:?}. \
                     The engine will retry at the next confirmation.",
                    view.confirmed_height(),
                    e
                );
                self.storage_manager.physical_openable_lower_bound()
            }
        }
    }

    pub fn notify_genesis_hash(&self, genesis_hash: EpochId) {
        if let Some(single_mpt_manager) = &self.single_mpt_storage_manager {
            *single_mpt_manager.genesis_hash.lock() = genesis_hash;
        }
    }

    pub fn config(&self) -> &StorageConfiguration {
        &self.storage_manager.storage_conf
    }
}

/// The physical openable lower bound implied by a set of snapshots: the lowest
/// height whose state can still be opened.
///
/// `executable_snapshots_by_height` must be sorted by ascending height and
/// must already have the archive snapshots kept only to serve sync removed.
/// Those sit far below the retention window with nothing between them and it,
/// so counting them would put the bound below a stretch of heights that cannot
/// be opened at all.
///
/// The run is walked down from the newest snapshot and stops at the first gap,
/// a pair of neighbours more than one period apart. The bound is the height of
/// the lowest snapshot in that run.
///
/// That height is included rather than skipped, because the snapshot holds
/// the whole state of its own epoch and the rebuild writes that epoch an
/// entry saying so. Garbage collection publishes the height above the
/// snapshot it keeps instead, and is right to: the entry `commit` left for
/// that epoch names layers two periods further down, which collection has
/// removed. Both are lower bounds, so the one that includes a height an open
/// may still refuse costs a refusal at the open and nothing else.
fn physical_openable_lower_bound_of(
    executable_snapshots_by_height: &[SnapshotInfo], snapshot_epoch_count: u32,
) -> u64 {
    let mut continuous_from = 0usize;
    for i in (1..executable_snapshots_by_height.len()).rev() {
        if executable_snapshots_by_height[i - 1].height
            + snapshot_epoch_count as u64
            == executable_snapshots_by_height[i].height
        {
            continuous_from = i - 1;
        } else {
            continuous_from = i;
            break;
        }
    }
    let lowest_continuous_snapshot_height = executable_snapshots_by_height
        .get(continuous_from)
        .map(|info| info.height)
        .unwrap_or(0);
    lowest_continuous_snapshot_height
}

/// What the read only open picked for one epoch. The two variants are the two
/// live state shapes: the layered state tree, and the single MPT replica used
/// when the layered tree cannot answer for the epoch and the node keeps a
/// single MPT covering the requested space.
enum ReadOnlyState {
    Layered(State),
    SingleMpt(SingleMptState),
}

impl StateManager {
    /// Open one version of the state, keyed by its epoch hash alone. The
    /// coordinates of that version come from the engine's own index; nothing
    /// about the layout of the state comes from the caller.
    ///
    /// `opts.mode` picks between reading that epoch and using it as the
    /// execution base of its child epoch.
    ///
    /// `Ok(None)` means the version is not available. A matchable "version not
    /// found" error variant arrives with the `StorageEngine` trait; until then
    /// this method keeps the `Option` shape its callers already branch on.
    pub fn open_state(
        self: &Arc<Self>, epoch_hash: &EpochId, opts: OpenOptions,
    ) -> Result<Option<Box<dyn StateTrait>>> {
        match opts.mode {
            OpenMode::ReadOnly => self.open_read_only(epoch_hash, opts),
            OpenMode::NextEpochBase { .. } => {
                self.open_next_epoch_base(epoch_hash, opts)
            }
        }
    }

    /// Can the epoch `base` identifies serve as the execution base of its
    /// child epoch? A per epoch existence question, asked when the
    /// sync path skips epochs it cannot execute and when the restart path
    /// locates the recompute start.
    ///
    /// Answered by the read only open, not by the base open: the two are not
    /// known to agree on availability in the snapshot sync special case.
    ///
    /// An engine error answers `false`. Every caller reacts to `false` by
    /// skipping the epoch or by re-executing it, and both are the safe move
    /// when the truth cannot be read off the disk.
    pub fn usable_as_base(&self, base: &EpochId) -> bool {
        match self.get_state_trees(
            base, /* try_open = */ false,
            /* open_mpt_snapshot = */ false,
        ) {
            Ok(maybe_state_trees) => maybe_state_trees.is_some(),
            Err(e) => {
                warn!(
                    "usable_as_base({:?}): the engine could not tell, {:?}. \
                     Answering not usable.",
                    base, e
                );
                false
            }
        }
    }

    /// Open one version of the state for an operational export. The export
    /// needs the callback traversal, which `StorageView` does not offer, so it
    /// gets a concrete export object rather than going through
    /// `open_state`.
    pub fn open_state_for_export(
        self: &Arc<Self>, epoch_hash: &EpochId, opts: OpenOptions,
    ) -> Result<Option<StateExport>> {
        Ok(self.open_read_only_state(epoch_hash, opts)?.map(
            |state| match state {
                ReadOnlyState::Layered(state) => StateExport::layered(state),
                ReadOnlyState::SingleMpt(state) => {
                    StateExport::single_mpt(state)
                }
            },
        ))
    }

    /// At the boundary of snapshot, getting a state for new epoch will switch
    /// to new Delta MPT, but it's unnecessary getting a no-commit state.
    ///
    /// With try_open == true, the call fails immediately when the max number of
    /// snapshot open is reached.
    ///
    /// If `space` is `None`, we need data from all spaces.
    fn open_read_only(
        self: &Arc<Self>, epoch_hash: &EpochId, opts: OpenOptions,
    ) -> Result<Option<Box<dyn StateTrait>>> {
        Ok(self.open_read_only_state(epoch_hash, opts)?.map(
            |state| match state {
                ReadOnlyState::Layered(state) => {
                    Box::new(state) as Box<dyn StateTrait>
                }
                ReadOnlyState::SingleMpt(state) => {
                    Box::new(state) as Box<dyn StateTrait>
                }
            },
        ))
    }

    /// The body of the read only open, answering with the concrete state it
    /// picked instead of a boxed trait object.
    fn open_read_only_state(
        self: &Arc<Self>, epoch_hash: &EpochId, opts: OpenOptions,
    ) -> Result<Option<ReadOnlyState>> {
        let try_open = opts.try_open;
        let space = opts.space;
        let epoch_id = *epoch_hash;
        let maybe_state_trees =
            self.get_state_trees(&epoch_id, try_open, false);
        // If there is an error, we will continue to search for an available
        // single_mpt.
        let maybe_state_err = match maybe_state_trees {
            Ok(Some(state_trees)) => {
                return Ok(Some(ReadOnlyState::Layered(State::new(
                    self.clone(),
                    state_trees,
                    false,
                ))));
            }
            Err(e) => Err(e),
            Ok(None) => Ok(None),
        };
        if self.single_mpt_storage_manager.is_none() {
            return maybe_state_err;
        }
        let single_mpt_storage_manager =
            self.single_mpt_storage_manager.as_ref().unwrap();
        if !single_mpt_storage_manager.contains_space(&space) {
            return maybe_state_err;
        }
        debug!("read state from single mpt state: epoch={}", epoch_id);
        let single_mpt_state =
            single_mpt_storage_manager.get_state_by_epoch(epoch_id)?;
        if single_mpt_state.is_none() {
            warn!("single mpt state missing: epoch={:?}", epoch_id);
            return maybe_state_err;
        } else {
            Ok(Some(ReadOnlyState::SingleMpt(single_mpt_state.unwrap())))
        }
    }

    /// The `NextEpochBase` half of `open_state`, the shape the deleted
    /// `get_state_for_next_epoch` had. The one field it used to read off
    /// the caller's `StateIndex` besides the epoch id is the parent height,
    /// which decides whether the single MPT covers the parent epoch; it now
    /// comes from the parent's index entry, which holds the engine's own copy
    /// of the very number the caller passed, see `resolve_coordinates`.
    fn open_next_epoch_base(
        self: &Arc<Self>, parent_epoch_hash: &EpochId, opts: OpenOptions,
    ) -> Result<Option<Box<dyn StateTrait>>> {
        let mut parent_epoch = *parent_epoch_hash;
        let state = self.open_layered_state(parent_epoch_hash, opts, false)?;
        if state.is_none() {
            return Ok(None);
        }
        if self.single_mpt_storage_manager.is_none() {
            return Ok(Some(Box::new(state.unwrap())));
        }
        let single_mpt_storage_manager =
            self.single_mpt_storage_manager.as_ref().unwrap();
        // TODO: the entry was already read once by `resolve_coordinates` on
        // the way in. Reading it twice is a wasted point lookup, not a
        // correctness problem.
        let parent_height = self
            .state_index_db
            .get(parent_epoch_hash)?
            .and_then(|entry| entry.maybe_height);
        if let Some(parent_height) = parent_height {
            trace!(
                "open_next_epoch_base: parent={}, available={}",
                parent_height,
                single_mpt_storage_manager.available_height
            );
            if single_mpt_storage_manager.available_height > parent_height {
                return Ok(Some(Box::new(state.unwrap())));
            } else if single_mpt_storage_manager.available_height
                == parent_height
            {
                // For the first available single_mpt state, we read the genesis
                // block state as the parent state to continue execution.
                // This is only needed for tests because there is no eSpace
                // state entries for Conflux Mainnet.
                parent_epoch = *single_mpt_storage_manager.genesis_hash.lock();
            }
        }
        let single_mpt_state =
            single_mpt_storage_manager.get_state_by_epoch(parent_epoch)?;
        if single_mpt_state.is_none() {
            error!("open_next_epoch_base: single_mpt_state is required but is not found!");
            return Ok(None);
        }
        Ok(Some(Box::new(ReplicatedState::new(
            state.unwrap(),
            single_mpt_state.unwrap(),
            single_mpt_storage_manager.get_state_filter(),
        ))))
    }

    /// Commit a whole changeset on top of `parent`. The caller names the
    /// parent version; opening the three layers of the next epoch happens
    /// here, inside the engine.
    ///
    /// `meta.height` is not read. The height the engine acts on comes off the
    /// state object, where the open path put it, and holds the same value.
    ///
    /// TODO: `recover_mpt_during_construct_pivot_state` is worked out by the
    /// engine itself once the recovery handshake lands.
    pub fn commit_changeset(
        self: &Arc<Self>, parent: StorageVersion,
        recover_mpt_during_construct_pivot_state: bool, changeset: Changeset,
        meta: CommitMeta,
    ) -> Result<StateRoot> {
        let mut state = match parent {
            StorageVersion::Empty => self.get_state_for_genesis_write(),
            StorageVersion::Epoch(parent) => self
                .open_state(
                    &parent,
                    OpenOptions::next_epoch_base(
                        recover_mpt_during_construct_pivot_state,
                    ),
                )?
                .ok_or_else(|| {
                    Error::Msg(format!(
                        "commit: the parent version {:?} is not available",
                        parent
                    ))
                })?,
        };
        state.commit_changeset(changeset, meta)
    }

    /// Compute the state root the genesis changeset would produce on the empty
    /// base, without persisting any of it.
    ///
    /// Only genesis needs a root before its epoch has an id. Execution is
    /// deferred: the state root a block header carries belongs to the epoch
    /// `DEFERRED_STATE_EPOCH_COUNT` heights below it, an epoch which already
    /// has a block hash of its own, so a commit always has an epoch id to
    /// persist under. Nothing sits that far below the earliest headers, so
    /// they carry the genesis root instead — the genesis header among them,
    /// and the hash of that header is in turn the epoch id the genesis state
    /// is committed under.
    ///
    /// The `commit_changeset` that follows applies the same changeset a second
    /// time, on a state object opened afresh. The preview runs on the plain
    /// delta MPT state, never on the replicated wrapper, so the single MPT
    /// replica sees each write exactly once, in the commit.
    pub fn preview_genesis_root(
        self: &Arc<Self>, changeset: &Changeset,
    ) -> Result<StateRoot> {
        self.get_state_for_genesis_write_inner()
            .preview_root(changeset)
    }

    /// The empty base, i.e. the version the genesis epoch is committed on top
    /// of. It is spelled as the null epoch, which the genesis state object
    /// already carries as its parent epoch id and which no real epoch can
    /// collide with.
    /// A read only handle on the empty base, which the genesis execution reads
    /// through. Every key is answered out of the null snapshot and an empty
    /// delta trie.
    pub fn open_empty_base(self: &Arc<Self>) -> Box<dyn StorageView> {
        Box::new(self.get_state_for_genesis_write_inner())
    }

    pub fn get_state_for_genesis_write(
        self: &Arc<Self>,
    ) -> Box<dyn StateTrait> {
        let state = self.get_state_for_genesis_write_inner();
        if self.single_mpt_storage_manager.is_none() {
            return Box::new(state);
        }
        let single_mpt_storage_manager =
            self.single_mpt_storage_manager.as_ref().unwrap();
        let single_mpt_state = single_mpt_storage_manager
            .get_state_for_genesis()
            .expect("single_mpt genesis initialize error");
        Box::new(ReplicatedState::new(
            state,
            single_mpt_state,
            single_mpt_storage_manager.get_state_filter(),
        ))
    }
}

use crate::{
    delta_mpt_key::{
        delta_mpt_padding, DeltaMptKeyPadding, GENESIS_DELTA_MPT_KEY_PADDING,
    },
    impls::{
        delta_mpt::*,
        errors::*,
        replicated_state::ReplicatedState,
        single_mpt_state::SingleMptState,
        state_export::StateExport,
        state_index_db::{StateIndexDb, StateIndexEntry},
        storage_db::{
            delta_db_manager_rocksdb::DeltaDbManagerRocksdb,
            snapshot_db_manager_sqlite::SnapshotDbManagerSqlite,
        },
        storage_manager::{
            single_mpt_storage_manager::SingleMptStorageManager,
            storage_manager::StorageManager,
        },
    },
    state::*,
    state_manager::*,
    storage_db::*,
    utils::guarded_value::GuardedValue,
    StorageConfiguration,
};
use malloc_size_of_derive::MallocSizeOf as MallocSizeOfDerive;
use primitives::{
    EpochId, MerkleHash, StateRoot, MERKLE_NULL_NODE, NULL_EPOCH,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
