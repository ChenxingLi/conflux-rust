use crate::{
    crypto::{add_commitment, AffinePolynomialCommitment as AffinePC},
    crypto_types::{Element, PolynomialCommitment as PC},
};

pub struct DkgState {
    num_nodes: usize,
    num_votes: usize,
    current_commitments: PC,
}

impl DkgState {
    pub fn receive_new_commitment(
        &mut self, node_votes: usize, commitments: AffinePC,
    ) {
        self.num_nodes += 1;
        self.num_votes += node_votes;

        self.current_commitments =
            add_commitment(&self.current_commitments, &commitments.into());
    }

    pub fn commit_secret(&self) -> Element {
        self.current_commitments
            .coefficients()
            .first()
            .map_or(Element::IDENTITY, |x| x.value())
    }
}
