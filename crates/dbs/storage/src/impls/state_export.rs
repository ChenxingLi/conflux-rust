// Copyright 2019 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

//! The operational export entry point.
//!
//! `state_dump` walks the whole state with a callback and an "account keys
//! only" filter. The data set cannot be materialized in memory and the filter
//! cannot be expressed as a key prefix, so neither method of `StorageView`
//! covers it, and it is not a concern of the execution engine, so it does not
//! belong on `StorageView` either. The engine hands out a concrete export
//! object instead.
//!
//! Which of the two state shapes that object wraps is decided here, on the
//! same dispatch the read only open uses: the layered state when the state
//! tree can answer for the requested epoch, the single MPT replica when it
//! cannot and the node keeps one covering the requested space.

use crate::{
    impls::{
        errors::*, merkle_patricia_trie::MptKeyValue,
        single_mpt_state::SingleMptState, state::State,
    },
    state::StorageView,
};
use primitives::StorageKeyWithSpace;

/// One version of the state, opened for an operational export. Besides the
/// callback traversal it is a `StorageView`, so the same handle serves the
/// point reads and prefix lookups the export tool does between traversals.
pub struct StateExport {
    state: ExportedState,
}

/// The two state shapes an export can run on. Both implement the traversal;
/// dropping either one would silently drop an export path.
enum ExportedState {
    /// The layered state: delta, intermediate and snapshot.
    Layered(State),
    /// The single MPT replica of one address space.
    SingleMpt(SingleMptState),
}

impl StateExport {
    pub(crate) fn layered(state: State) -> Self {
        StateExport {
            state: ExportedState::Layered(state),
        }
    }

    pub(crate) fn single_mpt(state: SingleMptState) -> Self {
        StateExport {
            state: ExportedState::SingleMpt(state),
        }
    }

    /// Walk every key/value pair under `access_key_prefix`, handing each one
    /// to `callback`. With `only_account_key` the traversal stops descending
    /// into everything that is not an account key.
    pub fn read_all_with_callback(
        &mut self, access_key_prefix: StorageKeyWithSpace,
        callback: &mut dyn FnMut(MptKeyValue), only_account_key: bool,
    ) -> Result<()> {
        match &mut self.state {
            ExportedState::Layered(state) => state.read_all_with_callback_impl(
                access_key_prefix,
                callback,
                only_account_key,
            ),
            ExportedState::SingleMpt(state) => state
                .read_all_with_callback_impl(
                    access_key_prefix,
                    callback,
                    only_account_key,
                ),
        }
    }
}

impl StorageView for StateExport {
    fn get(
        &self, access_key: StorageKeyWithSpace,
    ) -> Result<Option<Box<[u8]>>> {
        match &self.state {
            ExportedState::Layered(state) => state.get(access_key),
            ExportedState::SingleMpt(state) => state.get(access_key),
        }
    }

    fn iter_prefix(
        &self, access_key_prefix: StorageKeyWithSpace,
    ) -> Result<Box<dyn Iterator<Item = MptKeyValue>>> {
        match &self.state {
            ExportedState::Layered(state) => {
                state.iter_prefix(access_key_prefix)
            }
            ExportedState::SingleMpt(state) => {
                state.iter_prefix(access_key_prefix)
            }
        }
    }
}
