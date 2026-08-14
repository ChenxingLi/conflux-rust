// Copyright 2019 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

/// The width of one placeholder slot of `StateRootAuxInfo`, in bytes.
///
/// It is frozen at what the old decoder demands and must not follow the
/// storage engine's own key prefix algorithm: the two epoch id slots were
/// `H256` and the two key prefix slots were `DeltaMptKeyPadding`, all four
/// 32 bytes wide. Should the storage engine ever change the width of a key
/// prefix, this constant stays where it is, otherwise rows written here stop
/// being readable by an older node.
pub const AUX_INFO_PLACEHOLDER_BYTES: usize = 32;

/// One placeholder slot of `StateRootAuxInfo`: a run of bytes with no meaning.
///
/// Nothing interprets the content. It is written as zeroes and, when a row
/// written by an older node is decoded, whatever bytes that row carried are
/// kept as raw bytes and handed to nobody.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuxInfoPlaceholder([u8; AUX_INFO_PLACEHOLDER_BYTES]);

impl Encodable for AuxInfoPlaceholder {
    fn rlp_append(&self, s: &mut RlpStream) { s.append_internal(&&self.0[..]); }
}

impl Decodable for AuxInfoPlaceholder {
    fn decode(rlp: &Rlp) -> Result<Self, DecoderError> {
        let bytes = rlp.as_val::<Vec<u8>>()?;
        if bytes.len() != AUX_INFO_PLACEHOLDER_BYTES {
            // The width is the one the old decoder froze. A slot of any other
            // width is not a row this decoder is compatible with.
            return Err(DecoderError::RlpInconsistentLengthAndData);
        }
        let mut placeholder = Self::default();
        placeholder.0.copy_from_slice(&bytes);
        Ok(placeholder)
    }
}

/// What the commitment row carries next to the state root triplet.
///
/// Four of the five fields used to be the coordinates of the state version:
/// the snapshot epoch id, the intermediate epoch id and the two delta MPT key
/// prefixes. The engine holds the coordinates in its own index now, so those
/// four fields carry nothing and are written as zeroes; they survive only as
/// byte slots, because the row shape is frozen by the decoder of an older node.
/// The slot numbers below are RLP item positions and are the only thing left of
/// the old field order.
///
/// The fifth field is not a coordinate and keeps its meaning: it is the merged
/// hash of the state root triplet in the same row, and consensus both writes
/// and reads it, for the blame vector and the block reward computation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateRootAuxInfo {
    /// Slot 0, which used to hold the snapshot epoch id.
    pub placeholder_slot_0: AuxInfoPlaceholder,
    /// Slot 1, which used to hold the intermediate epoch id.
    pub placeholder_slot_1: AuxInfoPlaceholder,
    /// Slot 2, which used to hold the optional intermediate key prefix. The
    /// `Option` is part of the frozen shape, not of any remaining meaning: RLP
    /// encodes it as a list of zero or one item, and both forms are what an
    /// old decoder of this slot accepts. It is written as `Some`, so that the
    /// slot keeps the width the old row gave it.
    pub placeholder_slot_2: Option<AuxInfoPlaceholder>,
    /// Slot 3, which used to hold the delta key prefix.
    pub placeholder_slot_3: AuxInfoPlaceholder,

    /// The merged hash of the state root triplet in the same row.
    pub state_root_hash: MerkleHash,
}

impl StateRootAuxInfo {
    /// The only way to build one. The four coordinate slots are zero filled;
    /// there is no longer anything a caller could put in them.
    pub fn new(state_root_hash: MerkleHash) -> Self {
        Self {
            placeholder_slot_0: Default::default(),
            placeholder_slot_1: Default::default(),
            placeholder_slot_2: Some(Default::default()),
            placeholder_slot_3: Default::default(),
            state_root_hash,
        }
    }

    pub fn genesis_state_root_aux_info(
        genesis_state_root: &MerkleHash,
    ) -> Self {
        Self::new(genesis_state_root.clone())
    }
}

/// This struct is stored as state execution result and is used to compute state
/// of children blocks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateRootWithAuxInfo {
    pub state_root: StateRoot,
    pub aux_info: StateRootAuxInfo,
}

impl StateRootWithAuxInfo {
    /// The commitment row of an epoch: the triplet the commit answered with,
    /// and the merged hash which is a pure function of it.
    pub fn from_state_root(state_root: StateRoot) -> Self {
        let state_root_hash = state_root.compute_state_root_hash();
        Self {
            state_root,
            aux_info: StateRootAuxInfo::new(state_root_hash),
        }
    }

    pub fn genesis(genesis_root: &MerkleHash) -> Self {
        Self::from_state_root(StateRoot::genesis(genesis_root))
    }
}

impl From<(&StateRoot, &StateRootAuxInfo)> for StateRootWithAuxInfo {
    fn from(x: (&StateRoot, &StateRootAuxInfo)) -> Self {
        Self {
            state_root: x.0.clone(),
            aux_info: x.1.clone(),
        }
    }
}

impl Encodable for StateRootAuxInfo {
    fn rlp_append(&self, s: &mut RlpStream) {
        s.begin_list(5)
            .append(&self.placeholder_slot_0)
            .append(&self.placeholder_slot_1)
            .append(&self.placeholder_slot_2)
            .append(&self.placeholder_slot_3)
            .append(&self.state_root_hash);
    }
}

impl Decodable for StateRootAuxInfo {
    fn decode(rlp: &Rlp) -> Result<Self, DecoderError> {
        Ok(Self {
            placeholder_slot_0: rlp.val_at(0)?,
            placeholder_slot_1: rlp.val_at(1)?,
            placeholder_slot_2: rlp.val_at(2)?,
            placeholder_slot_3: rlp.val_at(3)?,
            state_root_hash: rlp.val_at(4)?,
        })
    }
}

impl Encodable for StateRootWithAuxInfo {
    fn rlp_append(&self, s: &mut RlpStream) {
        s.begin_list(2)
            .append(&self.state_root)
            .append(&self.aux_info);
    }
}

impl Decodable for StateRootWithAuxInfo {
    fn decode(rlp: &Rlp) -> Result<Self, DecoderError> {
        Ok(Self {
            state_root: rlp.val_at(0)?,
            aux_info: rlp.val_at(1)?,
        })
    }
}

use primitives::{MerkleHash, StateRoot};
use rlp::{Decodable, DecoderError, Encodable, Rlp, RlpStream};
use serde_derive::{Deserialize, Serialize};
