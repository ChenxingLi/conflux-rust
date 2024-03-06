use std::{collections::BTreeMap, ops::Deref};

use serde::{Deserialize, Serialize};

use crate::crypto_types::{Identifier, Scalar};

#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord,
)]
pub struct NodeID(u16);

impl NodeID {
    pub fn to_identifier(&self) -> Identifier {
        Identifier::new(Scalar::from(self.0 as u64)).unwrap()
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
        Identifier::new(Scalar::from(self.0 as u64)).unwrap()
    }

    pub fn as_usize(&self) -> usize { self.0 as usize }
}

pub fn num_to_identifier(value: usize) -> Identifier {
    Identifier::new(Scalar::from(value as u64)).unwrap()
}

// #[derive(
//     Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd,
// Ord, )]
// pub struct CellIdx(usize);
// impl CellIdx {
//     pub fn new(value: usize) -> Self { CellIdx(value) }

//     pub fn to_identifier(&self) -> Identifier {
//         Identifier::new(Scalar::from(self.0 as u64 + 1)).unwrap()
//     }
// }

// impl Deref for CellIdx {
//     type Target = usize;

//     fn deref(&self) -> &Self::Target { &self.0 }
// }

// impl From<VoteID> for CellIdx {
//     fn from(value: VoteID) -> Self { Self(value.0 - 1) }
// }

// impl From<CellIdx> for VoteID {
//     fn from(value: CellIdx) -> Self { Self(value.0 + 1) }
// }

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

    pub fn node_votes(&self, node_id: NodeID) -> usize {
        self.inner.get(&node_id).map_or(0, Vec::len)
    }
}
