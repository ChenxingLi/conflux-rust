// Copyright 2020 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

#![allow(dead_code)]

// `RecordingStorage` is a wrapper around other storage implementations that
// tracks all read accesses. It can then be turned into a `StateProof` that is
// able to prove all key-value accesses.

pub struct RecordingStorage<Storage> {
    storage: Storage,

    // note: we need interior mutability so that we can record accesses and we
    // need to use Mutex for this as State implementations need to be Send and
    // Sync. However, the current execution logic is single-threaded.
    proof_merger: Mutex<StateProofMerger>,
}

impl<Storage> RecordingStorage<Storage> {
    pub fn new(storage: Storage) -> Self {
        Self {
            storage,
            proof_merger: Default::default(),
        }
    }

    pub fn try_into_proof(self) -> Result<StateProof> {
        self.proof_merger.into_inner().finish()
    }
}

impl RecordingStorage<State> {
    fn record_kvs(&self, kvs: &Vec<MptKeyValue>) -> Result<()> {
        let mut proof_merger = self.proof_merger.lock();

        for (k, _) in kvs {
            let access_key =
                StorageKeyWithSpace::from_key_bytes::<CheckInput>(k)?;
            let (_, proof) = self.storage.get_with_proof(access_key)?;
            proof_merger.merge(proof);
        }

        Ok(())
    }
}

impl StorageView for RecordingStorage<State> {
    // we need to record `get` operations
    fn get(
        &self, access_key: StorageKeyWithSpace,
    ) -> Result<Option<Box<[u8]>>> {
        let (val, proof) = self.storage.get_with_proof(access_key)?;
        self.proof_merger.lock().merge(proof);
        Ok(val)
    }

    fn iter_prefix(
        &self, access_key_prefix: StorageKeyWithSpace,
    ) -> Result<Box<dyn Iterator<Item = MptKeyValue>>> {
        let kvs: Vec<MptKeyValue> =
            self.storage.iter_prefix(access_key_prefix)?.collect();
        self.record_kvs(&kvs)?;
        Ok(Box::new(kvs.into_iter()))
    }
}

use crate::{
    impls::{
        errors::*, merkle_patricia_trie::MptKeyValue, state_proof::StateProof,
    },
    state::*,
    StateProofMerger,
};
use parking_lot::Mutex;
use primitives::{CheckInput, StorageKeyWithSpace};
