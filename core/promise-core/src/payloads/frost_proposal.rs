use frost_secp256k1::Secp256K1Sha256;
use serde::{Deserialize, Serialize};

type Challenge = frost_core::Challenge<Secp256K1Sha256>;

#[derive(Serialize, Deserialize, Clone, Copy)]
struct ProposalPayloadID {
    epoch: u64,
    pre_commit_index: u32,
}

#[derive(Serialize, Deserialize, Clone)]
struct ProposalPayload {
    id: ProposalPayloadID,
    raw_message: Vec<u8>,
}
