use std::collections::BTreeMap;

use crate::{
    converted_id::{NodeID, VoteID},
    crypto_types::PolynomialCommitment,
};

pub struct DpssEpochState {
    epoch: u64,

    commitment_matrix: Vec<PolynomialCommitment>,

    identifier_groups: BTreeMap<NodeID, Vec<VoteID>>,
}
