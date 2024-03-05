mod error;
mod share_aggregator;
mod share_sender;
mod state;

use std::collections::BTreeMap;

pub use error::DkgError;
pub use share_aggregator::ShareAggregator;
pub use share_sender::ShareSender;
pub use state::DkgState;

use crate::{
    converted_id::VoteID,
    crypto_types::{PolynomialCommitment, Scalar},
};

pub struct VerifiableSecretShares {
    pub commitment: PolynomialCommitment,
    pub shares: BTreeMap<VoteID, Scalar>,
}
