use std::collections::BTreeMap;

use crate::{
    crypto_types::PolynomialCommitment,
    node_id::{NodeID, VoteID},
};

pub struct DpssEpochState {
    epoch: u64,

    two_dim_commitments: Vec<PolynomialCommitment>,

    identifier_groups: BTreeMap<NodeID, Vec<VoteID>>,
}
