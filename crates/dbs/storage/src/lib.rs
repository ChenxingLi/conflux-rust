// Copyright 2019 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

// TODO: check them again and reason about the safety of each usage.
#![allow(clippy::mut_from_ref)]

#[macro_use]
extern crate cfx_util_macros;
#[macro_use]
extern crate lazy_static;
#[macro_use]
extern crate log;

#[macro_use]
pub mod utils;

mod delta_mpt_key;
pub(self) mod snapshot_manager;
mod state;
mod state_manager;
#[macro_use]
pub(crate) mod storage_db;

pub mod tests;

mod impls;

pub mod storage_dir {
    use std::path::PathBuf;
    lazy_static! {
        pub static ref DELTA_MPTS_DIR: PathBuf =
            ["storage_db", "delta_mpts"].iter().collect::<PathBuf>();
        pub static ref STATE_INDEX_DB_PATH: PathBuf = "state_index_db".into();
        pub static ref SNAPSHOT_DIR: PathBuf =
            ["storage_db", "snapshot"].iter().collect::<PathBuf>();
        pub static ref SNAPSHOT_INFO_DB_NAME: &'static str = "snapshot_info";
        pub static ref SNAPSHOT_INFO_DB_PATH: PathBuf =
            ["storage_db", "snapshot_info_db"]
                .iter()
                .collect::<PathBuf>();
        pub static ref STORAGE_DIR: PathBuf = "storage_db".into();
    }
}

/// Consensus parameter is only configurable in test mode.
#[derive(Debug, Clone)]
pub struct ConsensusParam {
    // The current delta switching rule is simply split by height at
    // multiple of SNAPSHOT_EPOCHS_CAPACITY.
    //
    // Only if we see problem dealing with attacks, consider rules like the
    // size of delta trie.
    pub snapshot_epoch_count: u32,

    pub era_epoch_count: u64,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ProvideExtraSnapshotSyncConfig {
    /// Keep the snapshot at the same epoch as the checkpoint.
    /// TODO:
    ///  This config will be removed when there is a more reasonable
    ///  snapshot sync point.
    StableCheckpoint,
    EpochNearestMultipleOf(u32),
}

impl ProvideExtraSnapshotSyncConfig {
    pub fn from_str(config: &str) -> Option<Self> {
        const MULTIPLE_OF_PREFIX: &'static str = "multiple_of_";
        if config == "checkpoint" {
            Some(Self::StableCheckpoint)
        } else if config.starts_with(MULTIPLE_OF_PREFIX) {
            let number_str = &config[MULTIPLE_OF_PREFIX.len()..];
            match number_str.parse::<u32>() {
                Err(_) => None,
                Ok(num) => Some(Self::EpochNearestMultipleOf(num)),
            }
        } else {
            None
        }
    }

    pub fn parse_config_list(
        config: &str,
    ) -> std::result::Result<Vec<Self>, String> {
        let mut list = vec![];
        for item in config.split(",") {
            if item.len() > 0 {
                list.push(Self::from_str(item).ok_or_else(|| {
                    format!(
                        "{} is not a valid ProvideExtraSnapshotSyncConfig",
                        item
                    )
                })?);
            }
        }
        Ok(list)
    }
}

#[derive(Debug, Clone)]
pub struct StorageConfiguration {
    pub additional_maintained_snapshot_count: u32,
    pub consensus_param: ConsensusParam,
    pub debug_snapshot_checker_threads: u16,
    pub delta_mpts_cache_recent_lfu_factor: f64,
    pub delta_mpts_cache_start_size: u32,
    pub delta_mpts_cache_size: u32,
    pub delta_mpts_node_map_vec_size: u32,
    pub delta_mpts_slab_idle_size: u32,
    pub single_mpt_cache_start_size: u32,
    pub single_mpt_cache_size: u32,
    pub single_mpt_slab_idle_size: u32,
    pub max_open_snapshots: u16,
    pub path_delta_mpts_dir: PathBuf,
    pub path_storage_dir: PathBuf,
    pub path_snapshot_dir: PathBuf,
    pub path_snapshot_info_db: PathBuf,
    pub provide_more_snapshot_for_sync: Vec<ProvideExtraSnapshotSyncConfig>,
    pub max_open_mpt_count: u32,
    pub enable_single_mpt_storage: bool,
    pub single_mpt_space: Option<Space>,
    pub cip90a: u64,
    pub keep_snapshot_before_stable_checkpoint: bool,
    pub use_isolated_db_for_mpt_table: bool,
    pub use_isolated_db_for_mpt_table_height: Option<u64>,
    /// Force the restart recovery to rebuild the latest MPT snapshot even when
    /// the engine's own self check finds it intact. An operator override of an
    /// engine decision, hence part of the engine's own configuration.
    pub recovery_latest_mpt_snapshot: bool,
    pub keep_era_genesis_snapshot: bool,
    pub backup_mpt_snapshot: bool,
}

impl StorageConfiguration {
    pub fn new_default(
        conflux_data_dir: &str, snapshot_epoch_count: u32, era_epoch_count: u64,
    ) -> Self {
        let conflux_data_path = Path::new(conflux_data_dir);
        StorageConfiguration {
            additional_maintained_snapshot_count: 0,
            consensus_param: ConsensusParam {
                snapshot_epoch_count,
                era_epoch_count,
            },
            debug_snapshot_checker_threads:
                defaults::DEFAULT_DEBUG_SNAPSHOT_CHECKER_THREADS,
            delta_mpts_cache_recent_lfu_factor:
                defaults::DEFAULT_DELTA_MPTS_CACHE_RECENT_LFU_FACTOR,
            delta_mpts_cache_size: defaults::DEFAULT_DELTA_MPTS_CACHE_SIZE,
            delta_mpts_cache_start_size:
                defaults::DEFAULT_DELTA_MPTS_CACHE_START_SIZE,
            delta_mpts_node_map_vec_size: defaults::DEFAULT_NODE_MAP_SIZE,
            delta_mpts_slab_idle_size:
                defaults::DEFAULT_DELTA_MPTS_SLAB_IDLE_SIZE,
            single_mpt_cache_start_size:
                defaults::DEFAULT_DELTA_MPTS_CACHE_START_SIZE * 2,
            single_mpt_cache_size: defaults::DEFAULT_DELTA_MPTS_CACHE_SIZE * 2,
            single_mpt_slab_idle_size:
                defaults::DEFAULT_DELTA_MPTS_SLAB_IDLE_SIZE * 2,
            max_open_snapshots: defaults::DEFAULT_MAX_OPEN_SNAPSHOTS,
            path_delta_mpts_dir: conflux_data_path
                .join(&*storage_dir::DELTA_MPTS_DIR),
            path_snapshot_dir: conflux_data_path
                .join(&*storage_dir::SNAPSHOT_DIR),
            path_snapshot_info_db: conflux_data_path
                .join(&*storage_dir::SNAPSHOT_INFO_DB_PATH),
            path_storage_dir: conflux_data_path
                .join(&*storage_dir::STORAGE_DIR),
            provide_more_snapshot_for_sync: vec![
                ProvideExtraSnapshotSyncConfig::StableCheckpoint,
            ],
            max_open_mpt_count: defaults::DEFAULT_MAX_OPEN_MPT,
            enable_single_mpt_storage: false,
            single_mpt_space: None,
            cip90a: 0,
            keep_snapshot_before_stable_checkpoint: true,
            use_isolated_db_for_mpt_table: false,
            use_isolated_db_for_mpt_table_height: None,
            recovery_latest_mpt_snapshot: false,
            keep_era_genesis_snapshot: false,
            backup_mpt_snapshot: true,
        }
    }

    pub fn full_state_start_height(&self) -> Option<u64> {
        if self.enable_single_mpt_storage {
            let height = if self.single_mpt_space == Some(Space::Ethereum) {
                // The eSpace state is only available after cip90 is
                // enabled.
                self.cip90a
            } else {
                0
            };
            Some(height)
        } else {
            None
        }
    }
}

// The export surface. The modules above are private; everything this crate
// exports is named one by one in the `pub use` items below.
//
// The public entry point, the state handle and the commit vocabulary:
pub use self::{
    impls::errors::{Error, Result},
    state::{Changeset, CommitMeta, StorageView},
    state_manager::{
        ConsensusRecoveryView, OpenOptions, RecoveryPlan, StateConfirmedView,
        StorageEngine, StorageVersion,
    },
};
// Construction configuration:
pub use self::impls::defaults;

// Proof data types, needed by the light protocol and by the RPC layer.
pub use self::impls::{
    merkle_patricia_trie::{trie_proof::TrieProofNode, TrieProof},
    node_merkle_proof::{NodeMerkleProof, StorageRootProof},
    state_proof::StateProof,
};

// The simple_mpt algorithm family, used outside the state engine.
pub use self::impls::merkle_patricia_trie::simple_mpt::*;

// The concrete engine type and the entry points its adapter modules use.
pub use self::{
    impls::{
        delta_mpt::DeltaMptIterator, snapshot_sync::FullSyncVerifier,
        state_export::StateExport,
        storage_db::snapshot_db_manager_sqlite::SnapshotDbManagerSqlite,
    },
    state::State as StorageState,
    state_manager::StateManager as StorageManager,
    storage_db::{SnapshotInfo, SnapshotKeptToProvideSyncStatus},
};

// The delta MPT key prefix type and its derivation, recomputed by the light
// protocol when it verifies a proof locally.
pub use self::delta_mpt_key::{delta_mpt_padding, DeltaMptKeyPadding};

// The engine's own error type is exported as `Error` and `Result` above.

// Legacy exports, tracked by `dev-support/check_storage_exports.py`:
pub use self::{
    impls::{
        merkle_patricia_trie::{
            CompressedPathRaw, MptKeyValue, VanillaChildrenTable,
        },
        storage_db::{
            kvdb_rocksdb::KvdbRocksdb,
            kvdb_sqlite::{KvdbSqlite, KvdbSqliteStatements},
        },
    },
    storage_db::{KeyValueDbTrait, KeyValueDbTraitRead},
};

// Reachable inside the engine under `crate::`, not exported. Some entries
// are reached only from this crate's own tests, so the plain build sees them
// as unused.
#[allow(unused_imports)]
pub(crate) use self::{
    impls::{
        delta_mpt::*, merkle_patricia_trie::KVInserter,
        proof_merger::StateProofMerger, recording_storage::RecordingStorage,
        storage_db::sqlite::SqliteConnection, writable_state::WritableState,
    },
    state_manager::StateIndex,
};

#[cfg(any(test, feature = "testonly_code"))]
pub use self::tests::new_state_manager_for_unit_test as new_storage_manager_for_testing;
use cfx_types::Space;
use std::path::{Path, PathBuf};
