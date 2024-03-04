use std::collections::{btree_map::Entry, BTreeMap};

use frost_secp256k1::round1::NonceCommitment;

use crate::crypto_types::{Element, Identifier, Scalar};

use super::{
    error::FrostError,
    node_id::NodeID,
    signer_group::{self, FrostSignerGroup},
};

pub struct EpochNonceCommitments {
    commitments: BTreeMap<NodeID, Vec<[NonceCommitment; 2]>>,
    min_votes: usize,
    next_unused_index: usize,
}

impl EpochNonceCommitments {
    pub fn pull_next_commitments(
        &mut self, signer_group: &mut FrostSignerGroup,
        identifier_groups: &BTreeMap<NodeID, Vec<Identifier>>,
    ) -> Result<BTreeMap<NodeID, [NonceCommitment; 2]>, FrostError> {
        let mut nonce_commitments = BTreeMap::default();
        let mut to_remove_nodes = vec![];
        for (node_id, commitments) in &self.commitments {
            if !signer_group.contains(node_id) {
                continue;
            }

            if let Some(commitment) = commitments.get(self.next_unused_index) {
                nonce_commitments.insert(*node_id, commitment.clone());
            } else {
                to_remove_nodes.push(*node_id);
            }
        }

        if nonce_commitments.len() <= 1 {
            return Err(FrostError::NotEnoughUnusedPreCommit);
        }

        signer_group.remove_nodes(&to_remove_nodes)?;

        signer_group.check_enough_shares()?;
        self.next_unused_index += 1;

        Ok(nonce_commitments)
    }

    pub fn insert_commitments(
        &mut self, node_id: NodeID, signer_group: &FrostSignerGroup,
        nonce_commitments: Vec<[NonceCommitment; 2]>, accept_new_node: bool,
    ) -> Result<(), FrostError> {
        let node_identifier = node_id.to_identifier();
        for nonce_commitment in &nonce_commitments {
            if nonce_commitment[0].element() == &Element::IDENTITY
                || nonce_commitment[1].element() == &Element::IDENTITY
            {
                return Err(FrostError::IdentityNonceCommitment);
            }
        }

        if !accept_new_node {
            if !self.commitments.contains_key(&node_id) {
                return Err(FrostError::TooLatePreCommit);
            }
            if !signer_group.contains(&node_id) {
                return Err(FrostError::EjectedNode);
            }
        }

        let id = Identifier::new(1u64.into()).unwrap();
        let scalar = Scalar::from(1u64);
        let el = Element::GENERATOR * scalar;

        match self.commitments.entry(node_id) {
            Entry::Vacant(e) => {
                e.insert(nonce_commitments);
            }
            Entry::Occupied(e) => {
                e.into_mut().extend_from_slice(&nonce_commitments);
            }
        }

        Ok(())
    }
}
