use cfx_types::H256;
use serde::{Deserialize, Serialize};

use crate::crypto_types::SignatureShare;
use tiny_keccak::{Hasher, Keccak};

#[derive(Serialize, Deserialize, Clone)]
struct SignaturePayload {
    epoch: u64,
    sender: u64,
    proposal_index_list: Vec<u32>,
    signature_hash: H256,
}

#[derive(Serialize, Deserialize, Clone)]
struct SignatureWitness {
    signature_shards: Vec<SignatureShare>,
}

enum SignaturePayloadError {
    InconsistentSignatureHash,
}

impl SignaturePayload {
    fn validate_witness(
        &self, witness: &SignatureWitness,
    ) -> Result<(), SignaturePayloadError> {
        if self.signature_hash != signature_hash(&witness.signature_shards) {
            return Err(SignaturePayloadError::InconsistentSignatureHash);
        }
        Ok(())
    }
}

fn signature_hash(signature_shards: &[SignatureShare]) -> H256 {
    let mut hasher = Keccak::v256();
    for share in signature_shards.iter() {
        // TODO: Performance optimization for batch affine point inversion
        hasher.update(&share.serialize());
    }
    let mut digest = H256::zero();
    hasher.finalize(&mut digest.0);
    digest
}
