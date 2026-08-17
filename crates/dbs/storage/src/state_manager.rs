// Copyright 2019 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

// StateManager is the single entry-point to access State for any epoch.
// StateManager manages internal mutability and is thread-safe.
pub use super::impls::state_manager::StateManager;

/// Which version of the state is opened, or committed against. `Empty` is
/// the empty base: the state before any epoch exists. It is a concept of its
/// own, not an epoch id that happens to be absent, and how an engine stands
/// for the empty base on disk is that engine's own business.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StorageVersion {
    Empty,
    Epoch(EpochId),
}

/// The interface of a versioned state engine, for callers that read, write
/// and maintain state without depending on a concrete engine.
///
/// `open_state`, `commit` and `preview_genesis_root` take `self: Arc<Self>`:
/// the state objects they build store an owned `Arc` back to the engine,
/// which cannot be recovered from a `&self` receiver. The other methods need
/// only a borrow and take the ordinary `&self`.
///
/// `open_state` answers with `Box<dyn StorageView>`, which is read only. A
/// caller that has to write names a parent version and hands the engine a
/// changeset; the writable state object never leaves the engine.
pub trait StorageEngine: Send + Sync {
    /// Open one version of the state for reading. `Ok(None)` means the
    /// version is not available on this node, which the caller tells apart
    /// from an engine failure by the `Result` around it.
    fn open_state(
        self: Arc<Self>, version: StorageVersion, opts: OpenOptions,
    ) -> Result<Option<Box<dyn StorageView>>>;

    /// Apply a whole changeset on top of `parent` and persist it under
    /// `meta.epoch_id`, answering with the consensus commitment.
    fn commit(
        self: Arc<Self>, parent: StorageVersion, changeset: Changeset,
        meta: CommitMeta,
    ) -> Result<StateRoot>;

    /// The state root the genesis changeset would produce on the empty base,
    /// computed without persisting anything.
    ///
    /// Only genesis needs a root before its epoch has an id. Execution is
    /// deferred: the state root a block header carries belongs to the epoch
    /// `DEFERRED_STATE_EPOCH_COUNT` heights below it, an epoch which already
    /// has a block hash of its own, so a commit always has an epoch id to
    /// persist under. Nothing sits that far below the earliest headers, so
    /// they carry the genesis root instead — the genesis header among them,
    /// and the hash of that header is in turn the epoch id the genesis state
    /// is committed under.
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

/// The chain level facts the engine pulls while it plans a restart recovery.
/// Every method answers a question in consensus vocabulary; what to rebuild,
/// and from which epoch execution should restart, is the engine's own answer,
/// given as a `RecoveryPlan`.
///
/// The handle is read only and lives only for the duration of `plan_recovery`.
///
/// Not interchangeable with `StateConfirmedView`, although three of its
/// methods have a counterpart of the same name there. That handle must answer
/// without the consensus graph, because the maintenance trigger points hold it
/// exclusively; this one is asked about heights up to the replay window end,
/// which is close to the pivot tip, and about era boundaries which can sit
/// below the current era genesis, so it needs the consensus graph plus its own
/// database fallback.
pub trait ConsensusRecoveryView {
    /// The stable checkpoint height of the current era. The engine treats it
    /// as the floor of everything it may rebuild from: the snapshot at this
    /// height is the one snapshot recovery may assume exists.
    fn stable_checkpoint_height(&self) -> u64;

    /// The number of epochs in an era. The engine rounds down to an era
    /// aligned height to pick the anchor it rebuilds the snapshot pipeline
    /// from.
    fn era_epoch_count(&self) -> u64;

    /// Which epoch sits at `height` on the pivot chain. The engine names
    /// snapshots by epoch id, so it needs this to turn the heights it computes
    /// into the era anchor snapshot, the snapshot one period before it, and
    /// the snapshot candidates it scans for.
    fn pivot_hash_at_height(&self, height: u64) -> Option<EpochId>;

    /// The first height consensus is prepared to replay from, i.e. the bottom
    /// of the replay window. The engine scans the snapshot period boundaries
    /// inside this window looking for snapshots it removed at startup.
    fn replay_window_start_height(&self) -> u64;

    /// One past the last height consensus will replay in this window. The
    /// engine reports "no epoch in this window needs its MPT rebuilt" by
    /// answering with this height, and it is also where the engine clears its
    /// recovery mode: the commit at the height below this one is the last
    /// commit of the replay, which the engine cannot recognize on its own.
    fn replay_window_end_height(&self) -> u64;

    /// The height consensus would restart execution from if the engine had no
    /// say: the outcome of its own existence probe and its force recompute
    /// rules. The engine may only move it down, never up.
    fn proposed_recompute_start_height(&self) -> u64;
}

/// The engine's answer to `plan_recovery`. The snapshot work itself is already
/// done when this is returned; what is left is what consensus needs in order to
/// drive the replay.
pub struct RecoveryPlan {
    /// The height execution should restart from. It is at most the height
    /// consensus proposed. The engine does not support resuming from an
    /// arbitrary height: when the latest MPT snapshot has to be rebuilt, this
    /// is re-anchored to an era aligned height plus two snapshot periods,
    /// which is usually far below the chain tip.
    pub recompute_start_height: u64,
}

/// Which of the two layered opens the engine performs. Not part of
/// `OpenOptions` and not exported: `open_state` only reads, and opening a
/// version as the execution base of its child epoch produces a writable
/// state, which stays inside the engine.
///
/// At a snapshot boundary the same epoch has two valid three-layer layouts:
/// its own (what the block header commits to) and its child's (snapshot
/// shifts up, delta starts empty). ReadOnly assembles the epoch's own layout;
/// NextEpochBase assembles the child's.
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
