use super::{FrostPubKeyContext, FrostSignTask, NodeID};

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

    emulated_verifying_shares: BTreeMap<Identifier, VerifyingShare>,
}

impl FrostSignerGroup {
    pub fn contains(&self, id: &NodeID) -> bool {
        self.valid_nodes.contains(id)
    }

    #[inline]
    pub fn remove_nodes(&mut self, node_list: &[NodeID]) {
        let mut changed = false;
        for id in node_list {
            changed != self.valid_nodes.remove(&id);
        }

        if !changed {
            return;
        }

        self.update_emulated_verifying_shares();
    }

    #[inline]
    pub fn insert_node(&mut self, id: &NodeID) {
        self.valid_nodes.insert(*id);

        // For insert operation, emulated_verifying_shares is lazily updated by
        // `FrostState`
    }

    pub fn valid_nodes(&self) -> impl Iterator<Item = NodeID> + '_ {
        self.valid_nodes.iter().cloned()
    }

    pub fn update_emulated_verifying_shares(&mut self) {
        self.emulated_verifying_shares = self.make_emulated_verifying_shares();
    }

    fn make_emulated_verifying_shares(
        &self,
    ) -> BTreeMap<Identifier, VerifyingShare> {
        let mut answer = BTreeMap::new();

        let all_identifiers = self
            .valid_nodes
            .iter()
            .map(|id| self.context.identifier_groups.get(id).unwrap().iter())
            .flatten()
            .cloned()
            .collect();
        let all_emulated_identifiers = self
            .valid_nodes
            .iter()
            .map(|id| id.to_identifier())
            .collect();

        for node_id in self.valid_nodes.iter() {
            let mut scalars = vec![];
            let mut elements = vec![];

            for node_identifier in
                self.context.identifier_groups.get(&node_id).unwrap()
            {
                elements.push(
                    self.context
                        .verifying_shares()
                        .get(node_identifier)
                        .unwrap()
                        .to_element(),
                );
                scalars.push(
                    compute_lagrange_coefficient(
                        &all_identifiers,
                        None,
                        node_identifier.clone(),
                    )
                    .unwrap(),
                )
            }

            let mut emulated_verifying_share: Element = VartimeMultiscalarMul::<
                Secp256K1Sha256,
            >::vartime_multiscalar_mul(
                scalars, elements
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
