use std::{collections::BTreeMap, ops::Deref};

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

#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord,
)]
pub struct VoteID(usize);
impl VoteID {
    pub fn new(value: usize) -> Self {
        assert!(value > 0);
        VoteID(value)
    }

    pub fn to_identifier(&self) -> Identifier {
        Identifier::new(Scalar::from(self.0 as u32)).unwrap()
    }

    pub fn as_usize(&self) -> usize { self.0 as usize }
}

pub struct VoteGroup {
    inner: BTreeMap<NodeID, Vec<VoteID>>,
    total_votes: usize,
}

impl Deref for VoteGroup {
    type Target = BTreeMap<NodeID, Vec<VoteID>>;

    fn deref(&self) -> &Self::Target { &self.inner }
}

impl VoteGroup {
    pub fn new(inner: BTreeMap<NodeID, Vec<VoteID>>) -> Self {
        let total_votes: usize =
            inner.iter().map(|(_id, votes)| votes.len()).sum();
        Self { inner, total_votes }
    }

    pub fn total_votes(&self) -> usize { self.total_votes }
}
