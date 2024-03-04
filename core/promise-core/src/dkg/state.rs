use std::collections::BTreeMap;

use cfx_types::H256;
use k256::elliptic_curve::BatchNormalize;

use crate::{
    crypto_types::{
        Affine, AffinePolynomialCommitment as AffinePC, CoefficientCommitment,
        Element, PolynomialCommitment as PC, Projective, Scalar,
    },
    node_id::VoteID,
};

pub struct DkgState {
    num_nodes: usize,
    num_votes: usize,
    current_commitments: PC,
}

pub fn add_commitment(a: &PC, b: &PC) -> PC {
    let a_coeff = a.coefficients();
    let b_coeff = b.coefficients();
    let length = std::cmp::max(a.coefficients().len(), b.coefficients().len());

    let mut answer = vec![];
    for i in 0..length {
        let a_i = a_coeff.get(i).map_or(Element::IDENTITY, |x| x.value());
        let b_i = b_coeff.get(i).map_or(Element::IDENTITY, |x| x.value());
        let c_i = CoefficientCommitment::new(a_i + b_i);
        answer.push(c_i)
    }
    PC::new(answer)
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
        let x = Scalar::ONE;
        self.current_commitments
            .coefficients()
            .first()
            .map_or(Element::IDENTITY, |x| x.value())
    }
}

pub struct DkgManager {
    vote_ids: Vec<VoteID>,
    accepted_shares: Vec<Scalar>,
    secret_shares: BTreeMap<H256, (PC, Vec<Scalar>)>,
}

pub enum DkgError {
    InconsistentSecretShare,
    IncorrectLength,
}

impl DkgManager {
    pub fn receive_secret_share(
        &mut self, commitments: AffinePC, secret_shares: Vec<Scalar>,
    ) -> Result<(), DkgError> {
        let hash = commitments.hash();
        let commitments: PC = commitments.into();

        if self.vote_ids.len() != self.secret_shares.len() {
            return Err(DkgError::IncorrectLength);
        }

        let expected_points: Vec<Projective> = self
            .vote_ids
            .iter()
            .map(|id| {
                frost_core::keys::evaluate_vss(id.to_identifier(), &commitments)
            })
            .collect();
        let expected_points: Vec<Affine> =
            Projective::batch_normalize(&expected_points[..]);

        let actual_points: Vec<Projective> = secret_shares
            .iter()
            .map(|x| Affine::GENERATOR * x)
            .collect();
        let actual_points: Vec<Affine> =
            Projective::batch_normalize(&actual_points[..]);

        for (expected, actual) in
            expected_points.into_iter().zip(actual_points.into_iter())
        {
            if expected != actual {
                return Err(DkgError::InconsistentSecretShare);
            }
        }

        self.secret_shares
            .insert(hash, (commitments, secret_shares));

        Ok(())
    }
}
