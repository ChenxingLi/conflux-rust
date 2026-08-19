// Copyright 2019 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

// StateManager is the single entry-point to access State for any epoch.
// StateManager manages internal mutability and is thread-safe.
pub use super::impls::state_manager::StateManager;

/// Which version of the state is opened, or committed against. `Empty` is the
/// empty base: the state before any epoch exists. It is a concept of its own,
/// not an epoch id that happens to be absent, and how an engine stands for the
/// empty base on disk is that engine's own business.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageVersion {
    Empty,
    Epoch(EpochId),
}

/// The interface of a versioned state engine, for callers that read, write
/// and maintain state without depending on a concrete engine.
///
/// Reads and writes split at this surface: `open_state` answers a read-only
/// view, and a caller that writes names a parent version and hands the
/// engine a changeset — a writable state never leaves the engine.
///
/// `open_state`, `commit` and `preview_genesis_root` take `self: Arc<Self>`:
/// the state objects they build store an owned `Arc` back to the engine,
/// which cannot be recovered from a `&self` receiver. The other methods
/// need only a borrow and take the ordinary `&self`.
pub trait StorageEngine: Send + Sync {
    /// Open one version of the state for reading. `Ok(None)` means the
    /// version is not available on this node, which the caller tells apart
    /// from an engine failure by the `Result` around it.
    /// `StorageVersion::Empty` is always available and ignores the options:
    /// the empty base has nothing to filter by space, nothing to contend
    /// for, and no snapshot to open.
    fn open_state(
        self: Arc<Self>, version: StorageVersion, opts: OpenOptions,
    ) -> Result<Option<Box<dyn StorageView>>>;

    /// Apply a whole changeset on top of `parent` and persist it under
    /// `meta.epoch_id`, answering with the state root triplet.
    fn commit(
        self: Arc<Self>, parent: StorageVersion, changeset: Changeset,
        meta: CommitMeta,
    ) -> Result<StateRoot>;

    /// The state root the genesis changeset would produce on the empty base,
    /// computed without persisting anything.
    ///
    /// Genesis needs its root before it has an epoch id (the id is the header
    /// hash, which depends on the root). Every other epoch already has a
    /// block hash by the time it commits, thanks to deferred execution.
    fn preview_genesis_root(
        self: Arc<Self>, changeset: &Changeset,
    ) -> Result<StateRoot>;

    /// Whether the epoch `base` identifies can serve as the execution base
    /// of its child epoch. An engine error answers `false`; every caller
    /// reacts to `false` by skipping or re-executing the epoch, and both
    /// are the safe move.
    fn usable_as_base(&self, base: &EpochId) -> bool;

    /// The restart handshake: get the engine ready for the replay consensus
    /// is about to drive, and answer where it should start. The returned
    /// height is never above the one consensus proposed, so consensus can
    /// keep applying its own force recompute rules by taking the lower of
    /// the two.
    fn plan_recovery(&self, view: &dyn ConsensusRecoveryView) -> RecoveryPlan;

    /// Tell the engine that the chain has confirmed the state at
    /// `view.confirmed_height()`, its cue to reclaim what it no longer
    /// needs. The call is synchronous: an open or commit issued after it
    /// sees a state consistent with the reclamation. Returns the physical
    /// openable lower bound the reclamation leaves behind. An error on the
    /// way is logged rather than returned; the next confirmation retries
    /// the round.
    fn notify_state_confirmed(&self, view: &dyn StateConfirmedView) -> u64;
}

/// The chain facts the engine reads while it reclaims what a state
/// confirmation has made unnecessary.
///
/// The implementation must not need the consensus graph: the engine calls
/// this from inside the notification, and both trigger points hold the graph
/// exclusively. An implementation over the persisted epoch set table meets
/// that requirement.
pub trait StateConfirmedView {
    fn confirmed_height(&self) -> u64;

    /// The engine never removes states above this height; a node serving
    /// snapshot sync also keeps the snapshots one era apart from it.
    fn stable_checkpoint_height(&self) -> u64;

    fn era_epoch_count(&self) -> u64;

    /// The error text reaches the node log (the engine logs a failed round
    /// rather than returning it).
    fn pivot_hash_at_height(
        &self, height: u64,
    ) -> std::result::Result<EpochId, String>;
}

/// The chain facts the engine reads while it plans a restart recovery.
///
/// Not interchangeable with `StateConfirmedView`: that handle must work
/// without the consensus graph; this one queries heights near the pivot tip
/// and era boundaries below the current era genesis, so it needs the graph
/// plus a database fallback.
pub trait ConsensusRecoveryView {
    /// The engine treats this height as the floor of everything it may
    /// rebuild from.
    fn stable_checkpoint_height(&self) -> u64;

    fn era_epoch_count(&self) -> u64;

    fn pivot_hash_at_height(&self, height: u64) -> Option<EpochId>;

    fn replay_window_start_height(&self) -> u64;

    /// One past the last height consensus replays in this window. The commit
    /// at the height below is the last replay commit, which the engine
    /// cannot recognize on its own, so this is also where the engine clears
    /// its recovery mode.
    fn replay_window_end_height(&self) -> u64;

    /// The height consensus would restart execution from on its own. The
    /// engine may only move it down, never up.
    fn proposed_recompute_start_height(&self) -> u64;
}

/// The engine's answer to `plan_recovery`: what consensus needs in order to
/// drive the replay.
pub struct RecoveryPlan {
    /// The height execution should restart from, at most the height
    /// consensus proposed. When the latest MPT snapshot must be rebuilt, the
    /// engine re-anchors it to an era-aligned height plus two snapshot
    /// periods.
    pub recompute_start_height: u64,
}

/// Which of the two layered opens the engine performs.
///
/// At a snapshot boundary the same epoch has two valid three-layer layouts:
/// its own (what the block header commits to) and its child's (snapshot
/// shifts up, delta starts empty). ReadOnly assembles the epoch's own
/// layout; NextEpochBase assembles the child's.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum OpenMode {
    /// Read the state of that epoch itself.
    ReadOnly,
    /// Open the state of that epoch as the execution base of its child
    /// epoch. The engine shifts to a new delta MPT here when that epoch sits
    /// on a snapshot boundary.
    NextEpochBase,
}

/// Everything `StateManager::open_state` needs besides the epoch hash.
#[derive(Clone, Copy, Debug)]
pub struct OpenOptions {
    /// `None` means data from all address spaces are required. Only consulted
    /// on the read only open, which is the only one with a single MPT
    /// fallback.
    pub space: Option<Space>,
    /// Fail immediately instead of waiting when the engine has reached its
    /// limit of concurrently opened snapshots.
    pub try_open: bool,
}

impl OpenOptions {
    pub fn new() -> Self {
        Self {
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
    pub intermediate_delta_root: MerkleHash,
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

use crate::{
    delta_mpt_key::DeltaMptKeyPadding,
    impls::errors::Result,
    state::{Changeset, CommitMeta, StorageView},
};
use cfx_types::Space;
use primitives::{EpochId, MerkleHash, StateRoot};
use std::sync::Arc;
