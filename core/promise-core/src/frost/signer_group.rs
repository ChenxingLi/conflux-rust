use super::{error::FrostError, FrostPubKeyContext, FrostSignTask, NodeID};

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use frost_core::{compute_lagrange_coefficient, VartimeMultiscalarMul};
use frost_secp256k1::Secp256K1Sha256;

use crate::crypto_types::{
    Element, Identifier, NonceCommitment, PublicKeyPackage, SigningCommitments,
    SigningPackage, VerifyingShare,
};

pub struct FrostSignerGroup {
    context: Arc<FrostPubKeyContext>,

    valid_nodes: BTreeSet<NodeID>,

    total_votes: usize,
    emulated_verifying_shares: BTreeMap<Identifier, VerifyingShare>,
}

impl FrostSignerGroup {
    pub fn contains(&self, id: &NodeID) -> bool {
        self.valid_nodes.contains(id)
    }

    #[inline]
    pub(crate) fn insert_node(
        &mut self, id: &NodeID,
    ) -> Result<(), FrostError> {
        let changed = self.valid_nodes.insert(*id);
        if changed {
            self.total_votes += self
                .context
                .identifier_groups
                .get(id)
                .ok_or(FrostError::UnknownNodeID)?
                .len();
        }

        Ok(())

        // For insert operation, emulated_verifying_shares is lazily updated by
        // `FrostEpochState`
    }

    #[inline]
    pub fn remove_nodes(&mut self, node_list: &[NodeID]) {
        let mut changed = false;
        for id in node_list {
            changed |= self.valid_nodes.remove(&id);
        }

        if !changed {
            return;
        }

        self.update_emulated_verifying_shares();
    }

    pub fn valid_nodes(&self) -> impl Iterator<Item = NodeID> + '_ {
        self.valid_nodes.iter().cloned()
    }

    pub fn update_emulated_verifying_shares(&mut self) {
        self.emulated_verifying_shares = self.make_emulated_verifying_shares();
    }

    fn get_exact_size_identifier_groups(
        &self, num_identifiers: usize,
    ) -> Option<BTreeMap<NodeID, Vec<Identifier>>> {
        let mut identifier_groups = BTreeMap::new();
        let mut rest_votes = num_identifiers;
        for node_id in &self.valid_nodes {
            let node_identifiers =
                self.context.identifier_groups.get(node_id).unwrap();

            let picked_identifiers = if rest_votes <= node_identifiers.len() {
                &node_identifiers[..rest_votes]
            } else {
                &node_identifiers[..]
            };

            identifier_groups.insert(*node_id, picked_identifiers.to_vec());
            rest_votes -= picked_identifiers.len();
            if rest_votes == 0 {
                return Some(identifier_groups);
            }
        }
        None
    }

    fn make_emulated_verifying_shares(
        &self,
    ) -> BTreeMap<Identifier, VerifyingShare> {
        let mut answer = BTreeMap::new();

        let identifier_groups =
            self.get_exact_size_identifier_groups(self.context.min_votes).expect("The signer group should guarantee enough votes when making emulated shares");
        let all_emulated_identifiers = identifier_groups
            .keys()
            .map(|node_id| node_id.to_identifier())
            .collect();
        let all_origin_identifiers = identifier_groups
            .values()
            .map(|x| x.iter().copied())
            .flatten()
            .collect();

        for (node_id, origin_identifier_list) in identifier_groups {
            let mut lambdas = vec![];
            let mut origin_verifying_shares = vec![];

            for origin_identifier in origin_identifier_list {
                origin_verifying_shares.push(
                    self.context
                        .verifying_shares()
                        .get(&origin_identifier)
                        .unwrap()
                        .to_element(),
                );
                lambdas.push(
                    compute_lagrange_coefficient(
                        &all_origin_identifiers,
                        None,
                        origin_identifier.clone(),
                    )
                    .unwrap(),
                )
            }

            let mut emulated_verifying_share: Element = VartimeMultiscalarMul::<
                Secp256K1Sha256,
            >::vartime_multiscalar_mul(
                lambdas,
                origin_verifying_shares,
            );

            let emulated_identifier = node_id.to_identifier();

            let emulated_lambda_i = compute_lagrange_coefficient(
                &all_emulated_identifiers,
                None,
                emulated_identifier,
            )
            .unwrap();

            let inv_emulated_lambda_i = emulated_lambda_i.invert().unwrap();

            emulated_verifying_share *= inv_emulated_lambda_i;

            answer.insert(
                emulated_identifier,
                VerifyingShare::new(emulated_verifying_share),
            );
        }
        answer
    }

    pub fn make_sign_task(
        &self, nonce_commitments: &BTreeMap<NodeID, [NonceCommitment; 2]>,
        message: Vec<u8>,
    ) -> FrostSignTask {
        let signing_package = {
            let mut signing_commitments = BTreeMap::new();
            for &node_id in &self.valid_nodes {
                let [hiding, binding] =
                    nonce_commitments.get(&node_id).unwrap();
                signing_commitments.insert(
                    node_id.to_identifier(),
                    SigningCommitments::new(*hiding, *binding),
                );
            }
            SigningPackage::new(signing_commitments, &message)
        };
        let pubkey_package = PublicKeyPackage::new(
            self.emulated_verifying_shares.clone(),
            self.context.verifying_key().clone(),
        );

        FrostSignTask::new(signing_package, pubkey_package, message)
    }
}
