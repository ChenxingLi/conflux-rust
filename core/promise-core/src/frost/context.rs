use super::node_id::NodeID;

use std::collections::BTreeMap;

use crate::crypto_types::{
    Identifier, PublicKeyPackage, SigningShare, VerifyingKey, VerifyingShare,
};

/// Warning, since FrostContext contains private keys and must be maintained in
/// proper, it cannot be made publicity.
pub struct FrostPubKeyContext {
    pub epoch: u64,
    pub pubkey_package: PublicKeyPackage,
    pub identifier_groups: BTreeMap<NodeID, Vec<Identifier>>,
    pub min_votes: usize,
}

impl FrostPubKeyContext {
    pub fn verifying_shares(&self) -> &BTreeMap<Identifier, VerifyingShare> {
        self.pubkey_package.verifying_shares()
    }

    pub fn verifying_key(&self) -> &VerifyingKey {
        self.pubkey_package.verifying_key()
    }
}

pub struct FrostKey {
    /// DANGER: private key.
    signing_shares: BTreeMap<Identifier, SigningShare>,
}
