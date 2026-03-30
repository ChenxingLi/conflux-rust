// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

// Copyright 2021 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

use anyhow::Result;
use diem_state_view::{StateView, StateViewId};
use diem_types::{access_path::AccessPath, term_state::PosState};

/// `VerifiedStateView` is a snapshot of the global state for PoS execution.
///
/// In Conflux PoS, the VM (`PosVM`) reads state exclusively through
/// `pos_state()` and never calls `get()`.
pub struct VerifiedStateView {
    /// For logging and debugging purpose, identifies what this view is for.
    id: StateViewId,

    pos_state: PosState,
}

impl VerifiedStateView {
    pub fn new(id: StateViewId, pos_state: PosState) -> Self {
        Self { id, pos_state }
    }
}

impl StateView for VerifiedStateView {
    fn id(&self) -> StateViewId { self.id }

    fn get(&self, _access_path: &AccessPath) -> Result<Option<Vec<u8>>> {
        // SAFETY INVARIANT: PosVM must not read account state via
        // StateView::get(). It exclusively uses pos_state(). If this
        // panic is hit, a code change has violated this invariant and
        // the scratchpad/Jellyfish Merkle state tree removal is no
        // longer safe.
        panic!(
            "PosVM must not read account state via StateView::get(). \
             Use pos_state() instead. If you need account state reads, \
             the scratchpad state tree must be re-introduced."
        );
    }

    fn multi_get(
        &self, _access_paths: &[AccessPath],
    ) -> Result<Vec<Option<Vec<u8>>>> {
        unimplemented!();
    }

    fn is_genesis(&self) -> bool { false }

    fn pos_state(&self) -> &PosState { &self.pos_state }
}
