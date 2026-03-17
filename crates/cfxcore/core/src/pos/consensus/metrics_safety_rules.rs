// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

// Copyright 2021 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

use super::persistent_liveness_storage::PersistentLivenessStorage;
use consensus_types::{
    block::Block, block_data::BlockData, timeout::Timeout, vote::Vote,
    vote_proposal::MaybeSignedVoteProposal,
};
use diem_infallible::RwLock;
use diem_metrics::monitor;
use diem_types::{
    epoch_state::EpochState, validator_config::ConsensusSignature,
};
#[cfg(test)]
use safety_rules::ConsensusState;
use safety_rules::{Error, SafetyRules};
use std::sync::Arc;

/// Wrap safety rules with counters.
pub struct MetricsSafetyRules {
    inner: Arc<RwLock<SafetyRules>>,
    storage: Arc<dyn PersistentLivenessStorage>,
}

impl MetricsSafetyRules {
    pub fn new(
        inner: Arc<RwLock<SafetyRules>>,
        storage: Arc<dyn PersistentLivenessStorage>,
    ) -> Self {
        Self { inner, storage }
    }

    pub fn perform_initialize(&mut self) -> Result<(), Error> {
        let db = self.storage.pos_ledger_db();
        // Determine the latest epoch from the DB, then load the
        // epoch-ending LI that transitions into it.
        let latest_li = db.get_latest_ledger_info().map_err(|e| {
            Error::InternalError(format!(
                "Unable to retrieve latest ledger info: {}",
                e
            ))
        })?;
        let target_epoch = latest_li.ledger_info().next_block_epoch();
        let proof = db
            .get_epoch_ending_ledger_infos(
                target_epoch.saturating_sub(1),
                target_epoch,
            )
            .map_err(|e| {
                Error::InternalError(format!(
                    "Unable to retrieve epoch ending LI: {}",
                    e
                ))
            })?;
        let li = proof.ledger_info_with_sigs.last().ok_or_else(|| {
            Error::InternalError("No epoch ending LI found".into())
        })?;
        let epoch_state =
            li.ledger_info().next_epoch_state().ok_or_else(|| {
                Error::InternalError(
                    "Epoch ending LI has no next_epoch_state".into(),
                )
            })?;
        self.initialize(epoch_state)
    }

    #[cfg(test)]
    pub fn consensus_state(&mut self) -> Result<ConsensusState, Error> {
        monitor!("safety_rules", self.inner.write().consensus_state())
    }

    pub fn initialize(
        &mut self, epoch_state: &EpochState,
    ) -> Result<(), Error> {
        monitor!("safety_rules", self.inner.write().initialize(epoch_state))
    }

    pub fn construct_and_sign_vote(
        &mut self, vote_proposal: &MaybeSignedVoteProposal,
    ) -> Result<Vote, Error> {
        let mut result = monitor!(
            "safety_rules",
            self.inner.write().construct_and_sign_vote(vote_proposal)
        );

        if let Err(Error::NotInitialized(_res)) = result {
            self.perform_initialize()?;
            result = monitor!(
                "safety_rules",
                self.inner.write().construct_and_sign_vote(vote_proposal)
            );
        }
        result
    }

    pub fn sign_proposal(
        &mut self, block_data: BlockData,
    ) -> Result<Block, Error> {
        let mut result = monitor!(
            "safety_rules",
            self.inner.write().sign_proposal(block_data.clone())
        );
        if let Err(Error::NotInitialized(_res)) = result {
            self.perform_initialize()?;
            result = monitor!(
                "safety_rules",
                self.inner.write().sign_proposal(block_data)
            );
        }
        result
    }

    pub fn sign_timeout(
        &mut self, timeout: &Timeout,
    ) -> Result<ConsensusSignature, Error> {
        let mut result =
            monitor!("safety_rules", self.inner.write().sign_timeout(timeout));
        if let Err(Error::NotInitialized(_res)) = result {
            self.perform_initialize()?;
            result = monitor!(
                "safety_rules",
                self.inner.write().sign_timeout(timeout)
            );
        }
        result
    }

    pub fn start_voting(&mut self, initialize: bool) -> Result<(), Error> {
        monitor!("safety_rules", self.inner.write().start_voting(initialize))
    }

    pub fn stop_voting(&mut self) -> Result<(), Error> {
        monitor!("safety_rules", self.inner.write().stop_voting())
    }
}
