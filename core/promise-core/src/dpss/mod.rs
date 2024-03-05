mod state;

pub type DpssID = usize;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Copy)]
pub enum VssType {
    DistributedKeyGeneration,
    SecretShareRotation,
}
