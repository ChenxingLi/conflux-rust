#![allow(dead_code)]

use cfx_types::H256;
use group::{prime::PrimeCurveAffine, GroupEncoding};
use serde::{Deserialize, Serialize};
use tiny_keccak::{Hasher, Keccak};

use crate::crypto_types::{
    Affine, AffinePolynomialCommitment, Element, PolynomialCommitment,
};

#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Copy)]
pub enum VssType {
    DistributedKeyGeneration,
    SecretShareRotation,
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Copy)]
struct VSSPayloadID {
    epoch: u64,
    sender: u64,
}

#[derive(Serialize, Deserialize)]
struct VSSDummyProof;

#[derive(Serialize, Deserialize)]
struct VSSPayload {
    type_tag: VssType,
    id: VSSPayloadID,
    commitment_hash: H256,

    commitment: AffinePolynomialCommitment,
    proof: VSSDummyProof,
}

enum VSSPayloadError {
    InconsistentCommitmentHash,
    TooSmallDegree,
    NotZeroHole,
}

impl VSSPayload {
    pub fn validate_witness(&self) -> Result<(), VSSPayloadError> {
        if self.commitment.len() < 100 {
            return Err(VSSPayloadError::TooSmallDegree);
        }

        if self.commitment_hash != self.commitment.hash() {
            return Err(VSSPayloadError::InconsistentCommitmentHash);
        }

        // TODO: secret share validation

        if self.type_tag == VssType::SecretShareRotation
            && !bool::from(self.commitment[0].is_identity())
        {
            return Err(VSSPayloadError::NotZeroHole);
        }
        Ok(())
    }
}
