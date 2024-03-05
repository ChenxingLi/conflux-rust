use std::{collections::BTreeMap, sync::Arc};

use cfx_types::H256;
use rand_core::CryptoRngCore;
use serde::{Deserialize, Serialize};

use crate::{
    converted_id::{NodeID, VoteGroup, VoteID},
    crypto::generate_polynomial_commitments,
    crypto_types::{AffinePolynomialCommitment as AffinePC, Element, Scalar},
    TOTAL_VOTES,
};

pub type ShareSignature = ();

pub struct ShareSender {
    vote_groups: Arc<VoteGroup>,

    commitment: AffinePC,
    #[allow(unused)]
    commitment_hash: H256,
    shares: BTreeMap<VoteID, (Scalar, Element)>,

    signature: BTreeMap<NodeID, ShareSignature>,
}

#[derive(Serialize, Deserialize)]
pub struct RawShareMessage {
    commitment: AffinePC,
    shares: Vec<Scalar>,
}

impl ShareSender {
    pub fn generate<R: CryptoRngCore>(
        rng: &mut R, vote_groups: Arc<VoteGroup>, shared_input: Option<Scalar>,
    ) -> Self {
        let degree = TOTAL_VOTES / 3 + 1;
        let all_vote_id = vote_groups
            .values()
            .map(|x| x.iter())
            .flatten()
            .cloned()
            .collect();

        let (commitment, shares) = generate_polynomial_commitments(
            rng,
            degree - 1,
            all_vote_id,
            shared_input,
        );
        let commitment = AffinePC::from(commitment);
        let commitment_hash = commitment.hash();
        Self {
            vote_groups,
            commitment,
            commitment_hash,
            signature: BTreeMap::new(),
            shares,
        }
    }

    pub fn make_share_messages(&self) -> BTreeMap<NodeID, RawShareMessage> {
        let mut answer = BTreeMap::new();
        for (&node_id, votes) in self.vote_groups.iter() {
            let shares = votes
                .iter()
                .map(|id| self.shares.get(id).unwrap().0)
                .collect();
            answer.insert(
                node_id,
                RawShareMessage {
                    commitment: self.commitment.clone(),
                    shares,
                },
            );
        }
        answer
    }

    pub fn receive_share_ack(
        &mut self, node_id: NodeID, signature: ShareSignature,
    ) {
        if !self.vote_groups.contains_key(&node_id) {
            return;
        }
        self.signature.insert(node_id, signature);
    }

    pub fn generate_proof(&self) { todo!() }

    pub fn total_ack_votes(&self) -> usize {
        self.signature
            .keys()
            .map(|node_id| self.vote_groups.get(node_id).map_or(0, Vec::len))
            .sum()
    }
}
