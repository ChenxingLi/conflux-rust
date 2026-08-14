// Copyright 2019 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

// StateManager is the single entry-point to access State for any epoch.
// StateManager manages internal mutability and is thread-safe.
pub use super::impls::state_manager::StateManager;

pub type SharedStateManager = Arc<StateManager>;

/// Which version of the state is opened, or committed against. `Empty` is
/// the empty base: the state before any epoch exists. It is a concept of its
/// own, not an epoch id that happens to be absent, and how an engine stands
/// for the empty base on disk is that engine's own business.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageVersion {
    Empty,
    Epoch(EpochId),
}

/// What the caller intends to do with the version it opens: read that
/// epoch, or use it as the execution base of its child epoch. One entry point
/// serves both.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpenMode {
    /// Read the state of that epoch itself.
    ReadOnly,
    /// Open the state of that epoch as the execution base of its child
    /// epoch. The engine shifts to a new delta MPT here when that epoch sits
    /// on a snapshot boundary.
    NextEpochBase {
        /// Whether the snapshot generated while this epoch is committed has
        /// to rebuild its MPT from scratch, which is the case during the
        /// restart replay window. The flag moves into the engine later; until
        /// then it stays an input.
        recover_mpt_during_construct_pivot_state: bool,
    },
}

/// Everything `StateManager::open_state` needs besides the epoch hash.
#[derive(Clone, Copy, Debug)]
pub struct OpenOptions {
    pub mode: OpenMode,
    /// `None` means data from all address spaces are required. Only consulted
    /// in `ReadOnly` mode, which is the only mode with a single MPT fallback.
    pub space: Option<Space>,
    /// Fail immediately instead of waiting when the engine has reached its
    /// limit of concurrently opened snapshots.
    pub try_open: bool,
}

impl OpenOptions {
    pub fn read_only() -> Self {
        Self {
            mode: OpenMode::ReadOnly,
            space: None,
            try_open: false,
        }
    }

    pub fn next_epoch_base(
        recover_mpt_during_construct_pivot_state: bool,
    ) -> Self {
        Self {
            mode: OpenMode::NextEpochBase {
                recover_mpt_during_construct_pivot_state,
            },
            space: None,
            try_open: false,
        }
    }

    pub fn with_space(mut self, space: Option<Space>) -> Self {
        self.space = space;
        self
    }

    pub fn with_try_open(mut self, try_open: bool) -> Self {
        self.try_open = try_open;
        self
    }
}

#[derive(Debug)]
pub struct StateIndex {
    pub snapshot_epoch_id: EpochId,
    pub snapshot_merkle_root: MerkleHash,
    pub intermediate_epoch_id: EpochId,
    pub intermediate_trie_root_merkle: MerkleHash,
    pub maybe_intermediate_mpt_key_padding: Option<DeltaMptKeyPadding>,
    pub epoch_id: EpochId,
    pub delta_mpt_key_padding: DeltaMptKeyPadding,
    pub maybe_delta_trie_height: Option<u32>,
    pub maybe_height: Option<u64>,
}

impl StateIndex {
    pub fn height_to_delta_height(
        height: u64, snapshot_epoch_count: u32,
    ) -> u32 {
        if height == 0 {
            0
        } else {
            ((height - 1) % (snapshot_epoch_count as u64)) as u32 + 1
        }
    }
}

use crate::delta_mpt_key::DeltaMptKeyPadding;
use cfx_types::Space;
use primitives::{EpochId, MerkleHash};
use std::sync::Arc;
