// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

// Copyright 2021 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

use anyhow::Result;
use diem_state_view::{StateView, StateViewId};
use diem_types::{
    access_path::AccessPath,
    account_address::{AccountAddress, HashAccountAddress},
    account_state::AccountState,
    account_state_blob::AccountStateBlob,
    term_state::PosState,
};
use parking_lot::RwLock;
use scratchpad::{AccountStatus, SparseMerkleTree};
use std::{
    collections::{hash_map::Entry, HashMap},
    convert::TryInto,
};

/// `VerifiedStateView` is like a snapshot of the global state comprised of
/// state view at two levels, persistent storage and memory.
pub struct VerifiedStateView<'a> {
    /// For logging and debugging purpose, identifies what this view is for.
    id: StateViewId,

    /// The in-memory version of sparse Merkle tree of which the states haven't
    /// been committed.
    speculative_state: &'a SparseMerkleTree<AccountStateBlob>,

    /// The cache of verified account states from `speculative_state`,
    /// represented by a hashmap with an account address as key and an
    /// AccountState as value.
    account_to_state_cache: RwLock<HashMap<AccountAddress, AccountState>>,

    pos_state: PosState,
}

impl<'a> VerifiedStateView<'a> {
    /// Constructs a [`VerifiedStateView`] with the in-memory speculative
    /// state.
    pub fn new(
        id: StateViewId,
        speculative_state: &'a SparseMerkleTree<AccountStateBlob>,
        pos_state: PosState,
    ) -> Self {
        Self {
            id,
            speculative_state,
            account_to_state_cache: RwLock::new(HashMap::new()),
            pos_state,
        }
    }
}

impl<'a> From<VerifiedStateView<'a>> for HashMap<AccountAddress, AccountState> {
    fn from(view: VerifiedStateView<'a>) -> Self {
        view.account_to_state_cache.into_inner()
    }
}

impl<'a> StateView for VerifiedStateView<'a> {
    fn id(&self) -> StateViewId { self.id }

    fn get(&self, access_path: &AccessPath) -> Result<Option<Vec<u8>>> {
        let address = access_path.address;
        let path = &access_path.path;

        // Lock for read first:
        if let Some(contents) = self.account_to_state_cache.read().get(&address)
        {
            return Ok(contents.get(path).cloned());
        }

        // Do most of the work outside the write lock.
        let address_hash = address.hash();
        let account_blob_option = match self.speculative_state.get(address_hash)
        {
            AccountStatus::ExistsInScratchPad(blob) => Some(blob),
            AccountStatus::DoesNotExist
            | AccountStatus::ExistsInDB
            | AccountStatus::Unknown => None,
        };

        // Now enter the locked region, and write if still empty.
        let new_account_blob = account_blob_option
            .as_ref()
            .map(TryInto::try_into)
            .transpose()?
            .unwrap_or_default();

        match self.account_to_state_cache.write().entry(address) {
            Entry::Occupied(occupied) => Ok(occupied.get().get(path).cloned()),
            Entry::Vacant(vacant) => {
                Ok(vacant.insert(new_account_blob).get(path).cloned())
            }
        }
    }

    fn multi_get(
        &self, _access_paths: &[AccessPath],
    ) -> Result<Vec<Option<Vec<u8>>>> {
        unimplemented!();
    }

    fn is_genesis(&self) -> bool { false }

    fn pos_state(&self) -> &PosState { &self.pos_state }
}
