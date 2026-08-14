// Copyright 2024 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

//! The engine's own persistent state index.
//!
//! It maps an epoch id to the physical coordinates of the state version at
//! that epoch, plus the state root triplet and the height. The open path
//! resolves its coordinates from here. The commitment row still carries the
//! same coordinates in its `aux_info`; nothing resolves from that copy.
//!
//! One field cannot be recomputed and therefore must be stored: the
//! intermediate layer key padding. It is carried along the chain -- on a
//! snapshot shift it becomes the parent version's delta key padding, otherwise
//! it is inherited unchanged -- so it is not a function of the epoch's own
//! merkle roots. Everything else is either a chain fact or derivable, and is
//! stored anyway to keep the entry self contained.

use crate::{
    impls::{errors::*, storage_db::kvdb_rocksdb::KvdbRocksdb},
    storage_db::{KeyValueDbTrait, KeyValueDbTraitRead},
};
use kvdb_rocksdb::{CompactionProfile, Database, DatabaseConfig};
use primitives::EpochId;
use rlp::{Decodable, Encodable, Rlp};
use std::{path::Path, sync::Arc};

/// The entry lives in a module of its own so that the RLP derive expands
/// against `std::result::Result`; this file's `Result` is the engine's own
/// one argument alias.
mod entry {
    use primitives::{DeltaMptKeyPadding, EpochId, MerkleHash, StateRoot};
    use rlp_derive::{RlpDecodable, RlpEncodable};

    /// The value stored for one epoch. The key is the epoch id itself.
    #[derive(Clone, Debug, RlpEncodable, RlpDecodable)]
    pub struct StateIndexEntry {
        /// Snapshot layer: which snapshot the version sits on.
        pub snapshot_epoch_id: EpochId,
        pub snapshot_merkle_root: MerkleHash,
        /// Intermediate layer: which epoch's delta root freezes as the
        /// intermediate root for this version.
        pub intermediate_epoch_id: EpochId,
        pub intermediate_delta_root: MerkleHash,
        /// The intermediate layer key prefix. This is THE field which forces
        /// the index to exist: it is chained, not derivable from this
        /// epoch's roots. `None` means the special case where the
        /// snapshot db in use is the one of `intermediate_epoch_id`,
        /// i.e. there is no intermediate layer to look up.
        pub maybe_intermediate_mpt_key_padding: Option<DeltaMptKeyPadding>,
        /// The delta layer key prefix. Derivable from the two merkle roots
        /// above, stored to avoid recomputing a keccak on every open.
        pub delta_mpt_key_padding: DeltaMptKeyPadding,
        /// This epoch's own delta root, the third element of the triplet.
        pub delta_root: MerkleHash,
        /// The epoch height. `None` only on the test only paths which open a
        /// state without a height.
        pub maybe_height: Option<u64>,
        /// The number of epochs already in the delta MPT including this one.
        /// It decides whether the next epoch shifts to a new snapshot.
        /// It is a pure function of `maybe_height` and the snapshot
        /// period, but is stored so that the shift decision does not
        /// depend on that equivalence.
        pub maybe_delta_trie_height: Option<u32>,
    }

    impl StateIndexEntry {
        #[allow(dead_code)]
        pub fn state_root(&self) -> StateRoot {
            StateRoot {
                snapshot_root: self.snapshot_merkle_root,
                intermediate_delta_root: self.intermediate_delta_root,
                delta_root: self.delta_root,
            }
        }
    }
}

pub use entry::StateIndexEntry;

/// The index is a private key space of the engine. It is a rocksdb of its own
/// under the storage dir, so no existing table is touched. Point reads only,
/// one per state open, so it is on the hot read path and is left without a
/// lock of its own: rocksdb serves concurrent readers itself.
pub struct StateIndexDb {
    db: KvdbRocksdb,
}

impl StateIndexDb {
    /// The directory is created here if it is not there, but its parent, the
    /// storage dir, must exist already. The only caller is
    /// `StorageManager::new_arc`, right after it creates that dir, so the
    /// ordering holds by construction.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        // One row per epoch of a few hundred bytes, written once and read
        // back by point lookup, so the file budget is small and the size
        // triggers stay near rocksdb's own defaults. The write ahead log
        // stays on: the completion mark and the lower bound have to survive
        // a crash.
        const CONFIG: DatabaseConfig = DatabaseConfig {
            max_open_files: 64,
            memory_budget: None,
            compaction: CompactionProfile {
                initial_file_size: 64 * 1048576,
                block_size: 16 * 1024,
                write_rate_limit: None,
            },
            columns: 1,
            disable_wal: false,
        };
        let db = Database::open(
            &CONFIG,
            path.as_ref().to_str().ok_or_else(|| {
                Error::Msg(format!(
                    "the state index path {:?} is not valid unicode",
                    path.as_ref(),
                ))
            })?,
        )?;
        Ok(Self {
            db: KvdbRocksdb {
                kvdb: Arc::new(db),
                col: 0,
            },
        })
    }

    pub fn put(&self, epoch: &EpochId, entry: &StateIndexEntry) -> Result<()> {
        self.db.put(epoch.as_ref(), &entry.rlp_bytes())?;
        Ok(())
    }

    pub fn get(&self, epoch: &EpochId) -> Result<Option<StateIndexEntry>> {
        match self.db.get(epoch.as_ref())? {
            None => Ok(None),
            Some(bytes) => {
                Ok(Some(StateIndexEntry::decode(&Rlp::new(&bytes))?))
            }
        }
    }

    #[allow(dead_code)]
    pub fn remove(&self, epoch: &EpochId) -> Result<()> {
        self.db.delete(epoch.as_ref())?;
        Ok(())
    }

    /// The first boot migration runs once and leaves a mark. There is no lazy
    /// backfill: after the mark is set the index is maintained by the write
    /// ports only.
    pub fn migrated(&self) -> Result<bool> {
        Ok(self.db.get(MIGRATION_MARK_KEY)?.is_some())
    }

    pub fn set_migrated(&self) -> Result<()> {
        self.db.put(MIGRATION_MARK_KEY, &[1u8])?;
        Ok(())
    }

    /// The physical openable lower bound: the lowest height from which the
    /// disk content is continuously executable. Produced by garbage
    /// collection, so it has to survive a restart instead of being re-derived
    /// every time.
    ///
    /// `None` means no record, and a record whose length is not the eight
    /// bytes `store_lower_bound` writes reports the same thing: any other
    /// length is disk corruption or a format this build does not know, and
    /// neither can be turned into a height. The caller starts from zero in
    /// that case, which costs one round of redundant collection work and is
    /// undone by the next collection that writes the bound again, so the
    /// case is logged rather than made fatal.
    pub fn load_lower_bound(&self) -> Result<Option<u64>> {
        match self.db.get(LOWER_BOUND_KEY)? {
            None => Ok(None),
            Some(bytes) => {
                let mut buf = [0u8; 8];
                if bytes.len() != buf.len() {
                    warn!(
                        "Corrupted physical openable lower bound record: \
                         {} bytes instead of {}. Treating it as absent and \
                         starting from height 0; the next garbage collection \
                         writes the bound again.",
                        bytes.len(),
                        buf.len(),
                    );
                    return Ok(None);
                }
                buf.copy_from_slice(&bytes);
                Ok(Some(u64::from_be_bytes(buf)))
            }
        }
    }

    pub fn store_lower_bound(&self, height: u64) -> Result<()> {
        self.db.put(LOWER_BOUND_KEY, &height.to_be_bytes())?;
        Ok(())
    }
}

/// Marks the index as migrated. Its length differs from the 32 bytes of an
/// epoch id, so it cannot collide with an entry.
const MIGRATION_MARK_KEY: &[u8] = b"__state_index_migrated__";

const LOWER_BOUND_KEY: &[u8] = b"__physical_openable_lower_bound__";

#[cfg(test)]
mod tests {
    use super::*;
    use primitives::{DeltaMptKeyPadding, MerkleHash, MERKLE_NULL_NODE};
    use rand::random;
    use std::fs;

    /// An index on a directory of its own, removed when the test ends. The
    /// parent directory has to exist before the index is opened, the same
    /// ordering `StorageManager::new_arc` establishes.
    struct TempIndex {
        dir: std::path::PathBuf,
        index: Option<StateIndexDb>,
    }

    impl TempIndex {
        fn new() -> Self {
            let dir = std::env::temp_dir()
                .join(format!("cfx_state_index_test_{}", random::<u64>()));
            fs::create_dir_all(&dir).unwrap();
            let index = StateIndexDb::open(dir.join("state_index_db")).unwrap();
            Self {
                dir,
                index: Some(index),
            }
        }

        /// Closes the handle and opens the same path again, which is what a
        /// restart does to the index.
        fn reopen(&mut self) {
            self.index = None;
            self.index = Some(
                StateIndexDb::open(self.dir.join("state_index_db")).unwrap(),
            );
        }

        fn index(&self) -> &StateIndexDb { self.index.as_ref().unwrap() }
    }

    impl Drop for TempIndex {
        fn drop(&mut self) {
            self.index.take();
            fs::remove_dir_all(&self.dir).ok();
        }
    }

    fn padding(byte: u8) -> DeltaMptKeyPadding {
        let mut p = DeltaMptKeyPadding::default();
        p[..].copy_from_slice(&[byte; 32]);
        p
    }

    fn epoch(byte: u8) -> EpochId { EpochId::from([byte; 32]) }

    /// An entry with every optional field present, i.e. what a commit in the
    /// middle of a snapshot period writes.
    fn full_entry() -> StateIndexEntry {
        StateIndexEntry {
            snapshot_epoch_id: epoch(0x11),
            snapshot_merkle_root: MerkleHash::from([0x21; 32]),
            intermediate_epoch_id: epoch(0x12),
            intermediate_delta_root: MerkleHash::from([0x22; 32]),
            maybe_intermediate_mpt_key_padding: Some(padding(0x31)),
            delta_mpt_key_padding: padding(0x32),
            delta_root: MerkleHash::from([0x23; 32]),
            maybe_height: Some(4_200_000_000_000u64),
            maybe_delta_trie_height: Some(7),
        }
    }

    fn assert_same_entry(left: &StateIndexEntry, right: &StateIndexEntry) {
        assert_eq!(left.snapshot_epoch_id, right.snapshot_epoch_id);
        assert_eq!(left.snapshot_merkle_root, right.snapshot_merkle_root);
        assert_eq!(left.intermediate_epoch_id, right.intermediate_epoch_id);
        assert_eq!(left.intermediate_delta_root, right.intermediate_delta_root);
        assert_eq!(
            left.maybe_intermediate_mpt_key_padding,
            right.maybe_intermediate_mpt_key_padding
        );
        assert_eq!(left.delta_mpt_key_padding, right.delta_mpt_key_padding);
        assert_eq!(left.delta_root, right.delta_root);
        assert_eq!(left.maybe_height, right.maybe_height);
        assert_eq!(left.maybe_delta_trie_height, right.maybe_delta_trie_height);
    }

    // ---- the entry encoding -------------------------------------------

    #[test]
    fn entry_round_trips_with_every_field_present() {
        let entry = full_entry();
        let bytes = entry.rlp_bytes();
        let decoded = StateIndexEntry::decode(&Rlp::new(&bytes)).unwrap();
        assert_same_entry(&decoded, &entry);
        // The triplet the entry answers with is the one it stores.
        assert_eq!(
            decoded.state_root().snapshot_root,
            entry.snapshot_merkle_root
        );
        assert_eq!(
            decoded.state_root().intermediate_delta_root,
            entry.intermediate_delta_root
        );
        assert_eq!(decoded.state_root().delta_root, entry.delta_root);
    }

    /// The lowest surviving period has no previous period to take the
    /// intermediate padding from, and the test only open paths carry no
    /// height, so both `None` shapes are shapes the index really stores.
    #[test]
    fn entry_round_trips_with_the_empty_shapes() {
        let entry = StateIndexEntry {
            maybe_intermediate_mpt_key_padding: None,
            maybe_height: None,
            maybe_delta_trie_height: None,
            intermediate_delta_root: MERKLE_NULL_NODE,
            ..full_entry()
        };
        let bytes = entry.rlp_bytes();
        let decoded = StateIndexEntry::decode(&Rlp::new(&bytes)).unwrap();
        assert_same_entry(&decoded, &entry);
        assert!(decoded.maybe_intermediate_mpt_key_padding.is_none());
        assert!(decoded.maybe_height.is_none());
        assert!(decoded.maybe_delta_trie_height.is_none());
    }

    #[test]
    fn entry_has_nine_items_and_the_option_shapes_rlp_expects() {
        let with_some = full_entry().rlp_bytes();
        let rlp = Rlp::new(&with_some);
        assert_eq!(rlp.item_count().unwrap(), 9);
        // Item 4 is the only chained field: a one item list when present.
        let slot = rlp.at(4).unwrap();
        assert!(slot.is_list());
        assert_eq!(slot.item_count().unwrap(), 1);
        assert_eq!(slot.at(0).unwrap().data().unwrap().len(), 32);
        // Item 7, the height, is present as a one item list too.
        assert_eq!(rlp.at(7).unwrap().item_count().unwrap(), 1);

        let entry = StateIndexEntry {
            maybe_intermediate_mpt_key_padding: None,
            maybe_height: None,
            ..full_entry()
        };
        let with_none = entry.rlp_bytes();
        let rlp = Rlp::new(&with_none);
        assert_eq!(rlp.item_count().unwrap(), 9);
        assert_eq!(rlp.at(4).unwrap().item_count().unwrap(), 0);
        assert_eq!(rlp.at(4).unwrap().as_raw(), &[0xc0u8]);
        assert_eq!(rlp.at(7).unwrap().item_count().unwrap(), 0);
    }

    // ---- the index as a store ------------------------------------------

    #[test]
    fn entry_survives_a_put_get_and_a_reopen() {
        let mut temp = TempIndex::new();
        let id = epoch(0xa1);
        assert!(temp.index().get(&id).unwrap().is_none());

        let entry = full_entry();
        temp.index().put(&id, &entry).unwrap();
        assert_same_entry(&temp.index().get(&id).unwrap().unwrap(), &entry);

        temp.reopen();
        assert_same_entry(&temp.index().get(&id).unwrap().unwrap(), &entry);

        temp.index().remove(&id).unwrap();
        assert!(temp.index().get(&id).unwrap().is_none());
    }

    // ---- the physical openable lower bound -------------------------------

    #[test]
    fn lower_bound_round_trips_and_survives_a_reopen() {
        let mut temp = TempIndex::new();
        // No record on a fresh index: this is the one boot which derives the
        // bound from the registry instead of reading it.
        assert_eq!(temp.index().load_lower_bound().unwrap(), None);

        temp.index().store_lower_bound(0).unwrap();
        assert_eq!(temp.index().load_lower_bound().unwrap(), Some(0));

        temp.index().store_lower_bound(123_456).unwrap();
        assert_eq!(temp.index().load_lower_bound().unwrap(), Some(123_456));

        temp.reopen();
        assert_eq!(temp.index().load_lower_bound().unwrap(), Some(123_456));

        // The store keeps no monotonicity of its own; the caller decides it.
        temp.index().store_lower_bound(7).unwrap();
        assert_eq!(temp.index().load_lower_bound().unwrap(), Some(7));

        temp.index().store_lower_bound(u64::MAX).unwrap();
        assert_eq!(temp.index().load_lower_bound().unwrap(), Some(u64::MAX));
    }

    #[test]
    fn lower_bound_and_entries_do_not_collide() {
        let temp = TempIndex::new();
        let id = epoch(0xb2);
        temp.index().put(&id, &full_entry()).unwrap();
        temp.index().store_lower_bound(99).unwrap();
        temp.index().set_migrated().unwrap();

        assert_eq!(temp.index().load_lower_bound().unwrap(), Some(99));
        assert!(temp.index().get(&id).unwrap().is_some());
        assert!(temp.index().migrated().unwrap());
    }

    /// The degradation branch of `load_lower_bound`: a record whose byte
    /// length is not 8 is reported as "no record at all".
    #[test]
    fn a_lower_bound_record_of_the_wrong_length_reads_as_no_record() {
        let temp = TempIndex::new();
        temp.index().set_migrated().unwrap();

        for corrupt in [vec![], vec![0u8; 7], vec![0u8; 9]] {
            temp.index().db.put(LOWER_BOUND_KEY, &corrupt).unwrap();
            assert_eq!(
                temp.index().load_lower_bound().unwrap(),
                None,
                "a {} byte record did not read as absent",
                corrupt.len()
            );
        }

        // A corrupt entry value, in contrast, is an error rather than an
        // absence: the two records of this index disagree on how to treat a
        // record they cannot make sense of.
        let id = epoch(0xc3);
        temp.index()
            .db
            .put(id.as_ref(), &[0xff, 0xff, 0xff])
            .unwrap();
        assert!(temp.index().get(&id).is_err());
    }

    // ---- the first boot migration mark -----------------------------------

    #[test]
    fn migration_mark_is_absent_until_set_and_then_survives_a_reopen() {
        let mut temp = TempIndex::new();
        // Nothing set: the boot which runs the migration.
        assert!(!temp.index().migrated().unwrap());

        temp.index().set_migrated().unwrap();
        assert!(temp.index().migrated().unwrap());

        // Once the record exists the derivation must not run again, and a
        // restart is exactly where that would happen.
        temp.reopen();
        assert!(temp.index().migrated().unwrap());

        // Setting it twice is the same state, so a migration which reruns
        // before crashing leaves nothing inconsistent behind.
        temp.index().set_migrated().unwrap();
        assert!(temp.index().migrated().unwrap());
    }

    /// The mark is keyed on something an epoch id cannot be, so no entry can
    /// set it and no entry is shadowed by it.
    #[test]
    fn migration_mark_key_cannot_be_an_epoch_id() {
        assert_ne!(MIGRATION_MARK_KEY.len(), EpochId::len_bytes());
        assert_ne!(LOWER_BOUND_KEY.len(), EpochId::len_bytes());
        assert_ne!(MIGRATION_MARK_KEY, LOWER_BOUND_KEY);

        // And the mark is not readable as an entry, nor an entry as the mark.
        let temp = TempIndex::new();
        temp.index().put(&epoch(0xd4), &full_entry()).unwrap();
        assert!(!temp.index().migrated().unwrap());
        assert_eq!(temp.index().load_lower_bound().unwrap(), None);
    }
}
