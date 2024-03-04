use super::{error::FrostError, FrostPubKeyContext, FrostSignTask, NodeID};

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use frost_core::{compute_lagrange_coefficient, VartimeMultiscalarMul};
use frost_secp256k1::Secp256K1Sha256;

use crate::crypto_types::{
    Element, Identifier, PublicKeyPackage, Scalar, SigningCommitments,
    SigningPackage, VerifyingShare,
};

/// Represents a group of Frost signers within a specific epoch.
/// This structure holds cached information about signers participating in the
/// Frost signing process. Ideally, this type remain unchanged throughout the
/// epoch. However, a signer may be considered invalid if they submit erroneous
/// transactions or fail to respond in time, which changes the inside data.
pub struct FrostSignerGroup {
    /// The public key context for the epoch.
    pub context: Arc<FrostPubKeyContext>,

    /// Nodes that considered valid. (Submitted a pre-commit nonce before the
    /// epoch started and have not been deemed invalid.)
    valid_nodes: BTreeSet<NodeID>,

    /// Caches the aggregated Lagrange interpolation of shares held by the
    /// node. Updated when the set of valid nodes changes.
    aggregated_verifying_shares: BTreeMap<Identifier, VerifyingShare>,

    /// Lagrange Coefficients in aggregating shares.
    lagrange_coefficients: BTreeMap<NodeID, Vec<Scalar>>,

    /// A cached total count of signable shares. Allows for quick error
    /// responses when there are insufficient total shares held by valid
    /// nodes to proceed with signing requests.
    cached_total_shares: Option<usize>,
}

impl FrostSignerGroup {
    pub fn contains(&self, id: &NodeID) -> bool {
        self.valid_nodes.contains(id)
    }

    /// Check if the remain valid nodes hold enough signing shares.
    pub fn check_enough_shares(&self) -> Result<(), FrostError> {
        if let Some(shares) = self.cached_total_shares {
            if shares < self.context.num_signing_shares {
                return Err(FrostError::NotEnoughSigningShares);
            }
        }
        Ok(())
    }

    #[inline]
    /// Insert node at the beginning of an epoch.
    pub(crate) fn insert_node(
        &mut self, id: &NodeID,
    ) -> Result<(), FrostError> {
        if !self.context.identifier_groups.contains_key(id) {
            return Err(FrostError::UnknownNodeID);
        }
        self.valid_nodes.insert(*id);

        Ok(())

        // For insert operation, aggregated_verifying_shares is lazily updated
        // by `FrostEpochState`
    }

    #[inline]
    /// Mark some nodes as invalid.
    pub fn remove_nodes(
        &mut self, node_list: &[NodeID],
    ) -> Result<(), FrostError> {
        let mut changed = false;
        for id in node_list {
            changed |= self.valid_nodes.remove(&id);
        }

        if !changed {
            return Ok(());
        }

        self.update_aggregated_verifying_shares()?;
        Ok(())
    }

    pub fn valid_nodes(&self) -> impl Iterator<Item = NodeID> + '_ {
        self.valid_nodes.iter().cloned()
    }

    /// Calculates the exact groups of shares needed from each node for a Frost
    /// multisignature. The total shares across all nodes
    /// may exceed the exact required number of signers. This function
    /// determines which shares from each node are needed for the signature.
    ///
    /// # Arguments
    /// * `num_identifiers` - The exact number of signers (identifiers) required
    ///   for the signature.
    ///
    /// # Returns
    /// * `Ok(BTreeMap<NodeID, Vec<Identifier>>)` - A map from node IDs to the
    ///   identifiers of the shares that need to participate in the signature.
    /// * `Err(usize)` - The gap between the available shares and the minimum
    ///   required shares.
    fn get_exact_size_identifier_groups(
        &self, num_identifiers: usize,
    ) -> Result<BTreeMap<NodeID, Vec<Identifier>>, usize> {
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
                return Ok(identifier_groups);
            }
        }
        Err(rest_votes)
    }

    pub fn update_aggregated_verifying_shares(
        &mut self,
    ) -> Result<(), FrostError> {
        let mut aggregated_verifying_shares = BTreeMap::new();
        let mut lagrange_coefficients = BTreeMap::new();

        let identifier_groups = match self
            .get_exact_size_identifier_groups(self.context.num_signing_shares)
        {
            Ok(res) => res,
            Err(deficit_shares) => {
                self.cached_total_shares =
                    Some(self.context.num_signing_shares - deficit_shares);
                return Err(FrostError::NotEnoughSigningShares);
            }
        };

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

            lagrange_coefficients.insert(node_id, lambdas.clone());

            let aggregated_verifying_share: Element = VartimeMultiscalarMul::<
                Secp256K1Sha256,
            >::vartime_multiscalar_mul(
                lambdas.clone(),
                origin_verifying_shares,
            );

            aggregated_verifying_shares.insert(
                node_id.to_identifier(),
                VerifyingShare::new(aggregated_verifying_share),
            );
        }

        self.aggregated_verifying_shares = aggregated_verifying_shares;
        self.lagrange_coefficients = lagrange_coefficients;
        Ok(())
    }

    pub(crate) fn make_sign_task(
        &self, nonce_index: usize,
        nonce_commitments: &BTreeMap<NodeID, SigningCommitments>,
        message: Vec<u8>,
    ) -> FrostSignTask {
        // The caller should guarantee that the aggregated_verifying_shares is
        // ready.

        let signing_package = {
            let mut signing_commitments = BTreeMap::new();
            for node_id in self.valid_nodes.iter().filter(|x| {
                self.aggregated_verifying_shares
                    .contains_key(&x.to_identifier())
            }) {
                let signing_commitment =
                    nonce_commitments.get(&node_id).unwrap().clone();
                signing_commitments
                    .insert(node_id.to_identifier(), signing_commitment);
            }
            SigningPackage::new(signing_commitments, &message)
        };
        let pubkey_package = PublicKeyPackage::new(
            self.aggregated_verifying_shares.clone(),
            self.context.verifying_key().clone(),
        );

        FrostSignTask::new(
            signing_package,
            pubkey_package,
            self.lagrange_coefficients.clone(),
            nonce_index,
        )
    }
}
