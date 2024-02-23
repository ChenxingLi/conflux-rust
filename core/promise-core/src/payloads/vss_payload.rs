#![allow(dead_code)]

use cfx_types::H256;
use group::GroupEncoding;
use serde::{Deserialize, Serialize};
use tiny_keccak::{Hasher, Keccak};

use crate::crypto_types::{Element, PolynomialCommitment};

#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Copy)]
enum VSSPayloadType {
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
    type_tag: VSSPayloadType,
    id: VSSPayloadID,
    commitment_hash: H256,

    commitment: PolynomialCommitment,
    proof: VSSDummyProof,
}

enum VSSPayloadError {
    InconsistentCommitmentHash,
    NotZeroHole,
}

impl VSSPayload {
    pub fn validate_witness(&self) -> Result<(), VSSPayloadError> {
        if self.commitment_hash != polynomial_commitment_hash(&self.commitment)
        {
            return Err(VSSPayloadError::InconsistentCommitmentHash);
        }

        // TODO: secret share validation

        if self.type_tag == VSSPayloadType::SecretShareRotation
            && self.commitment.coefficients()[0].value() != Element::IDENTITY
        {
            return Err(VSSPayloadError::NotZeroHole);
        }
        Ok(())
    }
}

fn polynomial_commitment_hash(commitment: &PolynomialCommitment) -> H256 {
    let mut hasher = Keccak::v256();
    for ec_point in commitment.coefficients().iter() {
        // TODO: Performance optimization for batch affine point inversion
        hasher.update(ec_point.value().to_bytes().as_slice());
    }
    let mut digest = H256::zero();
    hasher.finalize(&mut digest.0);
    digest
}
