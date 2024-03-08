use std::{
    collections::{BTreeMap, BTreeSet},
    iter::zip,
};

use cfx_types::H256;

use crate::{
    converted_id::VoteID,
    crypto::{
        add_commitment, validate_verifiable_secret_share,
        AffinePolynomialCommitment as AffinePC,
    },
    crypto_types::{PolynomialCommitment as PC, Scalar},
    dkg::VerifiableSecretShares,
};

use super::DkgError;

pub struct ShareAggregator {
    vote_ids: BTreeSet<VoteID>,
    secret_shares: BTreeMap<H256, (PC, Vec<Scalar>)>,
    aggregated_shares: Vec<Scalar>,
    aggregated_commitment: PC,
    accepted_commitments: BTreeSet<H256>,
}

impl ShareAggregator {
    pub fn receive_secret_share(
        &mut self, commitment: AffinePC, secret_shares: Vec<Scalar>,
    ) -> Result<(), DkgError> {
        let hash = commitment.hash();
        let commitment: PC = commitment.into();

        if self.vote_ids.len() != self.aggregated_shares.len() {
            return Err(DkgError::IncorrectLength);
        }

        let shares = zip(&self.vote_ids, &secret_shares)
            .map(|(&x, &y)| (x, y))
            .collect();

        if !validate_verifiable_secret_share(&commitment, &shares) {
            return Err(DkgError::InconsistentSecretShare);
        }

        self.secret_shares.insert(hash, (commitment, secret_shares));

        Ok(())
    }

    pub fn accept_polynomial_commitment(
        &mut self, hash: H256,
    ) -> Result<(), DkgError> {
        let (pc, pc_shares) = self
            .secret_shares
            .get(&hash)
            .ok_or(DkgError::UnknownCommitment)?;
        let changed = self.accepted_commitments.insert(hash);
        if !changed {
            return Ok(());
        }

        self.aggregated_commitment =
            add_commitment(&self.aggregated_commitment, pc);

        for (aggregated, share) in zip(&mut self.aggregated_shares, pc_shares) {
            *aggregated += share;
        }

        Ok(())
    }

    pub fn finalize(self) -> Result<VerifiableSecretShares, DkgError> {
        assert_eq!(self.vote_ids.len(), self.secret_shares.len());
        let shares = zip(self.vote_ids, self.aggregated_shares).collect();

        Ok(VerifiableSecretShares {
            commitment: self.aggregated_commitment,
            shares,
        })
    }
}
