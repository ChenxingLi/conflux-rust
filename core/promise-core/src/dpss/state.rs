use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    usize::MIN,
};

use cfx_types::H256;
use frost_core::{keys::PublicKeyPackage, VerifyingKey};
use frost_secp256k1::Identifier;

use crate::{
    cfg_into_iter,
    converted_id::{num_to_identifier, NodeID, VoteGroup, VoteID},
    crypto::{evaluate_commitment_points, ElementMatrix},
    crypto_types::{
        AffinePolynomialCommitment as AffinePC, Element,
        PolynomialCommitment as PC, VerifyingShare,
    },
    dkg::{DkgState, VerifiableSecretShares},
    frost::FrostPubKeyContext,
    FROST_SIGN_VOTES, PROACTIVE_DKG_VOTES, PROACTIVE_RESHARE_VOTES,
};

#[cfg(feature = "parallel")]
use rayon::prelude::*;

pub enum DpssError {
    InvalidReshareCommitment,
    DkgStageNotComplete,
    DkgStageHasFinished,
    EnoughReshareSubmit,
}

pub struct ReshareState {
    // It is heavy to recover polynomial commitment from the commitment point
    // list, we store the commitment point list instead.
    commitment_points: Vec<Element>,
    accepted_commitments: BTreeSet<H256>,
    valid_submissions: BTreeMap<VoteID, PC>,
    target_votes: usize,
}

impl ReshareState {
    pub fn new(
        commitment_points: Vec<Element>, accepted_commitments: BTreeSet<H256>,
        target_votes: usize,
    ) -> Self {
        Self {
            commitment_points,
            accepted_commitments,
            valid_submissions: BTreeMap::new(),
            target_votes,
        }
    }

    pub fn from_dkg_stage(
        state: &DkgState, last_matrix: &ElementMatrix,
    ) -> Self {
        let element_list = last_matrix.get_col_add(0, state.commitment());

        ReshareState::new(
            element_list,
            state.commitment_hashes().clone(),
            PROACTIVE_RESHARE_VOTES,
        )
    }

    pub fn receive_reshare_message(
        &mut self, vote_id: VoteID, commitment: PC,
    ) -> Result<bool, DpssError> {
        let expected = self.commitment_points.get(vote_id.as_usize()).unwrap();
        let actual = commitment.coefficients().first().unwrap().value();
        if expected != &actual {
            return Err(DpssError::InvalidReshareCommitment);
        }

        self.valid_submissions.insert(vote_id, commitment);
        Ok(self.valid_submissions.len() == self.target_votes)
    }

    pub fn make_new_matrix(&self, empty_matrix: &mut ElementMatrix) {
        empty_matrix.set_col(0, &self.commitment_points);
        let mut filled_rows = BTreeSet::new();

        for (vote_id, commitment) in self.valid_submissions.iter() {
            let row_idx = vote_id.as_usize();
            empty_matrix.evaluate_row(row_idx, &commitment);
            filled_rows.insert(row_idx);
        }
        for col_idx in 1..empty_matrix.size().0 {
            empty_matrix.interpolate_col(col_idx, filled_rows.clone());
        }
    }
}

pub enum DpssStage {
    DkgStage(DkgState, bool),
    ReshareStage(ReshareState),
    Complete(ElementMatrix),
}

pub struct DpssEpochState {
    epoch: u64,

    stage: DpssStage,

    last_matrix: ElementMatrix,

    vote_groups: Arc<VoteGroup>,
}

impl DpssEpochState {
    pub fn receive_reshare_message(
        &mut self, vote_id: VoteID, commitment: PC,
    ) -> Result<(), DpssError> {
        match self.stage {
            DpssStage::DkgStage(_, false) => {
                return Err(DpssError::DkgStageNotComplete);
            }
            DpssStage::DkgStage(ref state, true) => {
                if !state.has_enough_votes(PROACTIVE_DKG_VOTES) {
                    return Err(DpssError::DkgStageNotComplete);
                }

                let reshare_state =
                    ReshareState::from_dkg_stage(state, &self.last_matrix);
                self.stage = DpssStage::ReshareStage(reshare_state);
            }
            DpssStage::ReshareStage(_) => {}
            DpssStage::Complete(_) => {
                return Err(DpssError::EnoughReshareSubmit);
            }
        };

        let reshare_state =
            if let DpssStage::ReshareStage(ref mut x) = self.stage {
                x
            } else {
                unreachable!()
            };

        let complete =
            reshare_state.receive_reshare_message(vote_id, commitment)?;

        if complete {
            let mut new_matrix = self.last_matrix.create_empty();
            reshare_state.make_new_matrix(&mut new_matrix);
            self.stage = DpssStage::Complete(new_matrix);
        }

        Ok(())
    }

    pub fn make_frost_context(&self) -> FrostPubKeyContext {
        let element_points = self.last_matrix.get_col(0);
        let verifying_key = VerifyingKey::new(element_points[0]);
        let verifying_shares = element_points
            .into_iter()
            .enumerate()
            .skip(1)
            .map(|(idx, elem)| {
                (num_to_identifier(idx), VerifyingShare::new(elem))
            })
            .collect();
        let pubkey_package =
            PublicKeyPackage::new(verifying_shares, verifying_key);

        let identifier_groups = self
            .vote_groups
            .iter()
            .map(|(node_id, votes)| {
                let identifiers =
                    votes.into_iter().map(VoteID::to_identifier).collect();
                (*node_id, identifiers)
            })
            .collect();
        
        FrostPubKeyContext {
            epoch: self.epoch,
            pubkey_package,
            identifier_groups,
            num_signing_shares: FROST_SIGN_VOTES,
        }
    }

    pub fn receive_dkg_participate(
        &mut self, node_id: NodeID, commitment: AffinePC,
    ) -> Result<(), DpssError> {
        let dkg_state = match self.stage {
            DpssStage::DkgStage(ref mut state, _) => state,
            _ => {
                return Err(DpssError::DkgStageHasFinished);
            }
        };

        dkg_state.receive_new_commitment(
            self.vote_groups.node_votes(node_id),
            commitment,
        );

        Ok(())
    }

    pub fn try_finish_dkg_stage(&mut self) -> Result<bool, DpssError> {
        if let DpssStage::DkgStage(ref state, ref mut can_finish) = self.stage {
            if state.has_enough_votes(PROACTIVE_DKG_VOTES) {
                let reshare_state =
                    ReshareState::from_dkg_stage(state, &self.last_matrix);
                self.stage = DpssStage::ReshareStage(reshare_state);
                Ok(true)
            } else {
                *can_finish = true;
                Ok(false)
            }
        } else {
            Err(DpssError::DkgStageHasFinished)
        }
    }
}
