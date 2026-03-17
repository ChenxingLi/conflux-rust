// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

// Copyright 2021 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

use crate::{
    local_client::LocalClient,
    persistent_safety_storage::PersistentSafetyStorage, SafetyRules,
    TSafetyRules,
};
use diem_config::config::SafetyRulesConfig;
use diem_infallible::RwLock;
use diem_secure_storage::{KVStorage, Storage};
use diem_types::{
    account_address::AccountAddress, validator_config::ConsensusVRFPrivateKey,
};
use std::{convert::TryInto, sync::Arc};

pub fn storage(config: &SafetyRulesConfig) -> PersistentSafetyStorage {
    let backend = &config.backend;
    let internal_storage: Storage =
        backend.try_into().expect("Unable to initialize storage");
    if let Err(error) = internal_storage.available() {
        panic!("Storage is not available: {:?}", error);
    }

    if let Some(test_config) = &config.test {
        let author = test_config.author;
        let consensus_private_key = test_config
            .consensus_key
            .as_ref()
            .expect("Missing consensus key in test config")
            .private_key();

        PersistentSafetyStorage::initialize(
            internal_storage,
            author,
            consensus_private_key,
            config.enable_cached_safety_data,
        )
    } else {
        panic!("Remote consensus key storage not supported!")
    }
}

pub struct SafetyRulesManager {
    internal_safety_rules: Arc<RwLock<SafetyRules>>,
}

impl SafetyRulesManager {
    pub fn new(config: &SafetyRulesConfig) -> Self {
        let storage = storage(config);
        let verify_vote_proposal_signature =
            config.verify_vote_proposal_signature;
        let export_consensus_key = config.export_consensus_key;
        let author = config.test.as_ref().map(|c| c.author).unwrap_or_default();

        Self::new_local(
            storage,
            verify_vote_proposal_signature,
            export_consensus_key,
            config.vrf_private_key.as_ref().map(|key| key.private_key()),
            author,
        )
    }

    pub fn new_local(
        storage: PersistentSafetyStorage, verify_vote_proposal_signature: bool,
        export_consensus_key: bool,
        vrf_private_key: Option<ConsensusVRFPrivateKey>,
        author: AccountAddress,
    ) -> Self {
        let safety_rules = SafetyRules::new(
            storage,
            verify_vote_proposal_signature,
            export_consensus_key,
            vrf_private_key,
            author,
        );
        Self {
            internal_safety_rules: Arc::new(RwLock::new(safety_rules)),
        }
    }

    pub fn client(&self) -> Box<dyn TSafetyRules + Send + Sync> {
        Box::new(LocalClient::new(self.internal_safety_rules.clone()))
    }
}
