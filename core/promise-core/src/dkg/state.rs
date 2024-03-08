use std::collections::BTreeSet;

use cfx_types::H256;

use crate::{
    crypto::{add_commitment, AffinePolynomialCommitment as AffinePC},
    crypto_types::{Element, PolynomialCommitment as PC},
};

pub struct DkgState {
    num_nodes: usize,
    num_votes: usize,
    commitment: PC,
    commitment_hashes: BTreeSet<H256>,
}

impl DkgState {
    pub fn new() -> Self {
        Self {
            num_nodes: 0,
            num_votes: 0,
            commitment: PC::new(vec![]),
            commitment_hashes: BTreeSet::new(),
        }
    }

    pub fn receive_new_commitment(
        &mut self, node_votes: usize, commitments: AffinePC,
    ) {
        let hash = commitments.hash();
        if self.commitment_hashes.contains(&hash) {
            return;
        } else {
            self.commitment_hashes.insert(hash);
        }

        self.num_nodes += 1;
        self.num_votes += node_votes;

        self.commitment = add_commitment(&self.commitment, &commitments.into());
    }

    pub fn commit_secret(&self) -> Element {
        self.commitment
            .coefficients()
            .first()
            .map_or(Element::IDENTITY, |x| x.value())
    }

    pub fn has_enough_votes(&self, votes: usize) -> bool {
        self.num_votes >= votes
    }

    pub fn commitment(&self) -> &PC { &self.commitment }

    pub fn commitment_hashes(&self) -> &BTreeSet<H256> {
        &self.commitment_hashes
    }
}
