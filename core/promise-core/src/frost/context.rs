use super::node_id::NodeID;

use std::collections::BTreeMap;

use crate::crypto_types::{
    Identifier, PublicKeyPackage, VerifyingKey, VerifyingShare,
};

/// Warning, since FrostContext contains private keys and must be maintained in
/// proper, it cannot be made publicity.
pub struct FrostPubKeyContext {
    pub epoch: u64,
    pub pubkey_package: PublicKeyPackage,
    pub identifier_groups: BTreeMap<NodeID, Vec<Identifier>>,
    pub num_signing_shares: usize,
}

impl FrostPubKeyContext {
    pub fn verifying_shares(&self) -> &BTreeMap<Identifier, VerifyingShare> {
        self.pubkey_package.verifying_shares()
    }

    pub fn verifying_key(&self) -> &VerifyingKey {
        self.pubkey_package.verifying_key()
    }
}
