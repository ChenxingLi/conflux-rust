// Copyright 2019 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

use crate::{
    impls::{
        errors::*, merkle_patricia_trie::MptKeyValue,
        single_mpt_state::SingleMptState, state::State,
    },
    state::StorageView,
};
use primitives::StorageKeyWithSpace;

/// One version of the state, opened for an operational export: a callback
/// driven full traversal plus the reads of `StorageView`.
pub struct StateExport {
    state: ExportedState,
}

/// The two state shapes an export can run on; dropping a variant silently
/// drops an export path.
enum ExportedState {
    Layered(State),
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
