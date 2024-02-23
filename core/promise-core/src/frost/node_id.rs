use serde::{Deserialize, Serialize};

use crate::crypto_types::{Identifier, Scalar};

#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord,
)]
pub struct NodeID(u16);

impl NodeID {
    pub fn to_identifier(&self) -> Identifier {
        Identifier::new(Scalar::from(self.0 as u32)).unwrap()
    }

    pub fn as_usize(&self) -> usize { self.0 as usize }
}
