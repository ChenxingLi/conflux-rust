use crate::crypto_types::NonceCommitment;
use cfx_types::H256;
use serde::{Deserialize, Serialize};
use tiny_keccak::{Hasher, Keccak};

#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Copy)]
struct PreCommitPayloadID {
    epoch: u64,
    sender: u8,
}

#[derive(Serialize, Deserialize, Clone, Copy)]
struct PreCommitPayload {
    id: PreCommitPayloadID,
    commitment_hash: H256,
    commitment_size: u32,
}

enum PreCommitPayloadError {
    InconsistentCommitmentHash,
    InconsistentLength,
}

#[derive(Serialize, Deserialize, Clone)]
struct PreCommitWitness {
    commitments: Vec<[NonceCommitment; 2]>,
}

impl PreCommitPayload {
    fn validate_witness(
        &self, witness: &PreCommitWitness,
    ) -> Result<(), PreCommitPayloadError> {
        if witness.commitments.len() as u32 != self.commitment_size {
            return Err(PreCommitPayloadError::InconsistentLength);
        }
        if self.commitment_hash
            != precommit_commitment_hash(&witness.commitments)
        {
            return Err(PreCommitPayloadError::InconsistentCommitmentHash);
        }
        Ok(())
    }
}

fn precommit_commitment_hash(commitments: &[[NonceCommitment; 2]]) -> H256 {
    let mut hasher = Keccak::v256();
    for ec_point in commitments.iter() {
        // TODO: Performance optimization for batch affine point inversion
        hasher.update(&ec_point[0].serialize());
        hasher.update(&ec_point[1].serialize());
    }
    let mut digest = H256::zero();
    hasher.finalize(&mut digest.0);
    digest
}
