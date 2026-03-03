// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

// Copyright 2021 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

use crate::on_chain_config::{OnChainConfig, ValidatorSet};
use anyhow::Result;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{collections::btree_map::BTreeMap, fmt};

#[derive(Default, Deserialize, PartialEq, Serialize)]
pub struct AccountState(BTreeMap<Vec<u8>, Vec<u8>>);

impl AccountState {
    pub fn get_validator_set(&self) -> Result<Option<ValidatorSet>> {
        self.get_config::<ValidatorSet>()
    }

    pub fn insert(&mut self, key: Vec<u8>, value: Vec<u8>) -> Option<Vec<u8>> {
        self.0.insert(key, value)
    }

    pub fn remove(&mut self, key: &[u8]) -> Option<Vec<u8>> {
        self.0.remove(key)
    }

    fn get_resource_impl<T: DeserializeOwned>(
        &self, key: &[u8],
    ) -> Result<Option<T>> {
        self.0
            .get(key)
            .map(|bytes| bcs::from_bytes(bytes))
            .transpose()
            .map_err(Into::into)
    }

    fn get_config<T: OnChainConfig>(&self) -> Result<Option<T>> {
        self.get_resource_impl(&T::CONFIG_ID.access_path().path)
    }
}

impl fmt::Debug for AccountState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let validator_set_str = self
            .get_validator_set()
            .map(|validator_set_opt| format!("{:#?}", validator_set_opt))
            .unwrap_or_else(|e| format!("parse error: {:#?}", e));

        write!(
            f,
            "{{ \n \
             ValidatorSet {{ {} }} \n \
             }}",
            validator_set_str,
        )
    }
}
