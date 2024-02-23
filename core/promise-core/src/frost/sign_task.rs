use super::FrostError;
use crate::crypto_types::{
    BindingFactorList, Challenge, GroupCommitment, Identifier,
    PublicKeyPackage, Signature, SignatureShare, SigningPackage,
};
use frost_core::{
    challenge, compute_binding_factor_list, compute_group_commitment,
    compute_lagrange_coefficient,
};
use std::collections::{BTreeMap, BTreeSet};

pub struct FrostSignTask {
    received_shares: BTreeMap<Identifier, SignatureShare>,

    binding_factor_list: BindingFactorList,
    group_commitment: GroupCommitment,
    signing_package: SigningPackage,
    pubkey_package: PublicKeyPackage,
    challenge: Challenge,
    message: Vec<u8>,
    all_identifiers: BTreeSet<Identifier>,
}

impl FrostSignTask {
    pub fn new(
        signing_package: SigningPackage, pubkey_package: PublicKeyPackage,
        message: Vec<u8>,
    ) -> Self {
        const BINDING_FACTOR_PREFIX: &'static [u8] = b"conflux-promise";

        let verifying_key = pubkey_package.verifying_key();

        let binding_factor_list = compute_binding_factor_list(
            &signing_package,
            verifying_key,
            BINDING_FACTOR_PREFIX,
        );
        // TODO: Remember check validity of input: identifier != hiding / biding
        let group_commitment =
            compute_group_commitment(&signing_package, &binding_factor_list)
                .unwrap();
        let challenge = challenge(
            &group_commitment.clone().to_element(),
            &verifying_key,
            &message,
        );

        let all_identifiers =
            pubkey_package.verifying_shares().keys().cloned().collect();

        FrostSignTask {
            binding_factor_list,
            group_commitment,
            challenge,
            signing_package,
            pubkey_package,
            all_identifiers,
            message,
            received_shares: Default::default(),
        }
    }

    pub fn message(&self) -> &[u8] { &self.message }

    pub fn all_shares_ready(&self) -> bool {
        self.received_shares.len() == self.all_identifiers.len()
    }

    pub fn unsigned_nodes(&self) -> BTreeSet<Identifier> {
        let mut unsigned_nodes = self.all_identifiers.clone();
        for signed_nodes in self.received_shares.keys() {
            unsigned_nodes.remove(signed_nodes);
        }
        unsigned_nodes
    }

    pub fn insert_signature_share(
        &mut self, identifier: &Identifier, signature_share: SignatureShare,
    ) -> Result<(), FrostError> {
        if self.received_shares.contains_key(identifier) {
            return Err(FrostError::DuplicatedSignatureShare);
        }

        self.verify_signature_share(identifier, &signature_share)?;
        self.received_shares
            .insert(identifier.clone(), signature_share);

        Ok(())
    }

    pub fn try_aggregate_signature_share(&self) -> Option<Signature> {
        if !self.all_shares_ready() {
            return None;
        }
        // unwrap safety: FrostSignTask should reject unknown identifiers
        Some(
            frost_core::aggregate(
                &self.signing_package,
                &self.received_shares,
                &self.pubkey_package,
            )
            .unwrap(),
        )
    }

    pub fn verify_signature_share(
        &self, identifier: &Identifier, signature_share: &SignatureShare,
    ) -> Result<(), FrostError> {
        let challenge = &self.challenge;

        // Look up the public key for this signer, where `signer_pubkey` =
        // _G.ScalarBaseMult(s[i])_, and where s[i] is a secret share of
        // the constant term of _f_, the secret polynomial.
        let signer_pubkey = self
            .pubkey_package
            .verifying_shares()
            .get(identifier)
            .ok_or(FrostError::UnknownSigner)?;

        // Compute Lagrange coefficient.
        let lambda_i = compute_lagrange_coefficient(
            &self.all_identifiers,
            None,
            *identifier,
        )
        .unwrap();

        let binding_factor = self
            .binding_factor_list
            .get(identifier)
            .ok_or(FrostError::UnknownSigner)?;

        // Compute the commitment share.
        let r_share = self
            .signing_package
            .signing_commitment(identifier)
            .ok_or(FrostError::UnknownSigner)?
            .to_group_commitment_share(binding_factor);

        // Compute relation values to verify this signature share.
        signature_share
            .verify(*identifier, &r_share, signer_pubkey, lambda_i, &challenge)
            .map_err(|_| FrostError::InvalidSignatureShare)?;

        Ok(())
    }
}
