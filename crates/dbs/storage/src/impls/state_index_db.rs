// Copyright 2024 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

//! The engine's own persistent state index.
//!
//! It maps an epoch id to the physical coordinates of the state version at
//! that epoch, plus the state root triplet and the height. The open path does
//! not resolve from it: it takes those coordinates out of the `aux_info` field
//! of the commitment row, which consensus owns. So `commit` is the only thing
//! that fills this table and nothing yet depends on what is in it.
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

    #[allow(dead_code)]
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
}
