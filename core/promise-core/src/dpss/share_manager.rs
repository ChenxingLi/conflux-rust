use frost_secp256k1::Identifier;
use k256::{elliptic_curve::ops::MulByGenerator, ProjectivePoint, Scalar};
use std::{
    collections::{BTreeMap, BTreeSet},
    iter::zip,
};

use crate::{
    converted_id::{NodeID, VoteID},
    crypto::{interpolate_and_evaluate_share, ElementMatrix},
    PROACTIVE_COL_VOTES, PROACTIVE_ROW_VOTES,
};

use super::DpssError;

pub struct ShareManager {
    epoch: usize,
}

type SenderID = VoteID;

pub struct HandoffManager {
    vote_id: BTreeSet<VoteID>,
    received_shares: BTreeMap<SenderID, Vec<Scalar>>,
}

impl HandoffManager {
    pub fn receive_share(
        &mut self, last_matrix: &ElementMatrix, sender_id: SenderID,
        shares: Vec<Scalar>,
    ) -> Result<bool, DpssError> {
        if self.received_shares.contains_key(&sender_id) {
            return Err(DpssError::DuplicatedHandoffShare);
        }

        if shares.len() != self.vote_id.len() {
            return Err(DpssError::IncorrectHandoffLength);
        }

        if self.vote_id.contains(&sender_id) {
            return Err(DpssError::IncorrectHandoffSender);
        }

        for (vote_id, share) in zip(&self.vote_id, &shares) {
            let expected =
                &last_matrix[(sender_id.as_usize(), vote_id.as_usize())];
            let actual = ProjectivePoint::mul_by_generator(share);
            if *expected != actual {
                return Err(DpssError::IncorrectHandoffShare);
            }
        }

        self.received_shares.insert(sender_id, shares);

        Ok(self.received_shares.len() >= PROACTIVE_ROW_VOTES)
    }

    pub fn construct_col_share(&self) -> BTreeMap<VoteID, Scalar> {
        let mut to_recover_id: BTreeMap<VoteID, BTreeMap<usize, Scalar>> = self
            .vote_id
            .iter()
            .map(|id| (*id, Default::default()))
            .collect();
        for (sender_id, shares) in self.received_shares.iter() {
            for (vote_id, share) in zip(&self.vote_id, shares) {
                to_recover_id
                    .get_mut(vote_id)
                    .unwrap()
                    .insert(sender_id.as_usize(), share.clone());
            }
        }
        to_recover_id
            .into_iter()
            .map(|(id, shares)| (id, interpolate_and_evaluate_share(shares)))
            .collect()
    }
}
