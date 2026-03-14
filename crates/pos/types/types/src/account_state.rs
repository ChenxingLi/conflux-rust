// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

// Copyright 2021 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

use crate::{
    access_path::Path,
    diem_timestamp::DiemTimestampResource,
    on_chain_config::{ConfigurationResource, OnChainConfig, ValidatorSet},
};
use anyhow::Result;
use move_core_types::move_resource::MoveResource;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{collections::btree_map::BTreeMap, convert::TryFrom, fmt};

#[derive(Default, Deserialize, PartialEq, Serialize)]
pub struct AccountState(BTreeMap<Vec<u8>, Vec<u8>>);

impl AccountState {
    pub fn get_configuration_resource(
        &self,
    ) -> Result<Option<ConfigurationResource>> {
        self.get_resource::<ConfigurationResource>()
    }

    pub fn get_diem_timestamp_resource(
        &self,
    ) -> Result<Option<DiemTimestampResource>> {
        self.get_resource::<DiemTimestampResource>()
    }

    pub fn get_validator_set(&self) -> Result<Option<ValidatorSet>> {
        self.get_config::<ValidatorSet>()
    }

    pub fn get(&self, key: &[u8]) -> Option<&Vec<u8>> { self.0.get(key) }

    pub fn get_resource_impl<T: DeserializeOwned>(
        &self, key: &[u8],
    ) -> Result<Option<T>> {
        self.0
            .get(key)
            .map(|bytes| bcs::from_bytes(bytes))
            .transpose()
            .map_err(Into::into)
    }

    pub fn insert(&mut self, key: Vec<u8>, value: Vec<u8>) -> Option<Vec<u8>> {
        self.0.insert(key, value)
    }

    pub fn remove(&mut self, key: &[u8]) -> Option<Vec<u8>> {
        self.0.remove(key)
    }

    pub fn iter(
        &self,
    ) -> impl std::iter::Iterator<Item = (&Vec<u8>, &Vec<u8>)> {
        self.0.iter()
    }

    pub fn get_config<T: OnChainConfig>(&self) -> Result<Option<T>> {
        self.get_resource_impl(&T::CONFIG_ID.access_path().path)
    }

    pub fn get_resource<T: MoveResource + DeserializeOwned>(
        &self,
    ) -> Result<Option<T>> {
        self.get_resource_impl(&T::struct_tag().access_vector())
    }

    /// Return an iterator over the module values stored under this account
    pub fn get_modules(&self) -> impl Iterator<Item = &Vec<u8>> {
        self.0.iter().filter_map(|(k, v)| {
            match Path::try_from(k).expect("Invalid access path") {
                Path::Code(_) => Some(v),
                Path::Resource(_) => None,
            }
        })
    }
}

impl fmt::Debug for AccountState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let diem_timestamp_str = self
            .get_diem_timestamp_resource()
            .map(|diem_timestamp_opt| format!("{:#?}", diem_timestamp_opt))
            .unwrap_or_else(|e| format!("parse: {:#?}", e));

        let validator_set_str = self
            .get_validator_set()
            .map(|validator_set_opt| format!("{:#?}", validator_set_opt))
            .unwrap_or_else(|e| format!("parse error: {:#?}", e));

        write!(
            f,
            "{{ \n \
             DiemTimestamp {{ {} }} \n \
             ValidatorSet {{ {} }} \n \
             }}",
            diem_timestamp_str, validator_set_str,
        )
    }
}
