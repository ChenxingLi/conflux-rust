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

/// The chain level facts the engine pulls while it reclaims what a state
/// confirmation has made unnecessary. Every method answers a question in
/// consensus vocabulary; which states and snapshots to remove is the engine's
/// own answer.
///
/// The handle is read only and lives only for the duration of
/// `StateManager::notify_state_confirmed`.
///
/// The implementation must not need the consensus graph: the engine calls this
/// from inside the notification, and the trigger points are on the consensus
/// side, holding the graph. An implementation over the persisted epoch set
/// table meets that requirement, because that table is written for every
/// height up to a fixed distance behind the pivot tip, and the heights asked
/// about here are derived from the confirmed height and stay well below it.
pub trait StateConfirmedView {
    /// The height whose state the chain has confirmed. Every height the engine
    /// looks up follows from this one by its own retention rules.
    fn confirmed_height(&self) -> u64;

    /// The stable checkpoint height of the current era. A node configured to
    /// serve snapshot sync keeps the snapshots which sit an era apart from
    /// this height, and the engine never removes the states above it.
    fn stable_checkpoint_height(&self) -> u64;

    /// The number of epochs in an era, the step of the era aligned heights
    /// counted off `stable_checkpoint_height`.
    fn era_epoch_count(&self) -> u64;

    /// Which epoch sits at `height` on the pivot chain. The engine names
    /// snapshots by epoch id, so it needs this to turn the height it retains
    /// down to into an epoch id.
    ///
    /// The error text says why the lookup found nothing; it reaches the node
    /// log, because the engine logs a failed round rather than returning it.
    fn pivot_hash_at_height(
        &self, height: u64,
    ) -> std::result::Result<EpochId, String>;
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
