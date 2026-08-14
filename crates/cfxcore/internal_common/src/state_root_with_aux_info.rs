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

/// The on-disk compatibility contract for the commitment row: a row written by
/// an old node must decode here, a row written here must decode on an old node,
/// and the width of a placeholder slot is the one the old row gave it.
///
/// The old row shape, which every test below takes as the reference, is
/// (from the version of this file on `master`):
///
/// ```text
/// list(2)[ StateRoot,
///          list(5)[ H256,                       // snapshot epoch id
///                   H256,                       // intermediate epoch id
///                   list(0) | list(1)[32 bytes], // Option<key prefix>
///                   32 bytes,                   // delta key prefix
///                   H256 ] ]                    // state root hash
/// ```
#[cfg(test)]
mod tests {
    use super::*;

    /// The row `master` writes for the genesis epoch, byte for byte, with a
    /// genesis root of 32 `0x77` bytes. Produced by running the encoder of
    /// `master` (`StateRootWithAuxInfo::genesis`), not by the encoder under
    /// test. Its `Option` slot is empty, and its slot 3 carries the real
    /// `GENESIS_DELTA_MPT_KEY_PADDING`, i.e. a genuine key prefix and not a
    /// value invented for the test.
    const MASTER_GENESIS_ROW: &str = "f8ec\
        f863\
        a0c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470\
        a0c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470\
        a07777777777777777777777777777777777777777777777777777777777777777\
        f885\
        a0c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470\
        a0c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470\
        c0\
        a09c6b2c1b0d0b25a008e6c882cc7b415f309965c72ad2b944ac0931048ca31cd5\
        a0431c960cc8074e37b1e7ae919fabf244efb7a05c83f25ac15b746ab0140b59fb";

    /// The merged hash inside `MASTER_GENESIS_ROW`.
    const MASTER_GENESIS_STATE_ROOT_HASH: &str =
        "431c960cc8074e37b1e7ae919fabf244efb7a05c83f25ac15b746ab0140b59fb";

    /// The delta key padding inside `MASTER_GENESIS_ROW`, i.e. what
    /// `GENESIS_DELTA_MPT_KEY_PADDING` was on `master`.
    const MASTER_GENESIS_DELTA_PADDING: &str =
        "9c6b2c1b0d0b25a008e6c882cc7b415f309965c72ad2b944ac0931048ca31cd5";

    /// A steady state row of `master`, byte for byte: all four coordinate
    /// slots filled and the `Option` slot present, which is the shape of
    /// every row a running node writes after genesis. Produced by the encoder
    /// of `master` from the triplet `(0x01.., 0x02.., 0x03..)`, the epoch ids
    /// `0xaa..` and `0xbb..`, and the key prefixes `0xcc..` and `0xdd..`.
    const MASTER_STEADY_ROW: &str = "f9010d\
        f863\
        a00101010101010101010101010101010101010101010101010101010101010101\
        a00202020202020202020202020202020202020202020202020202020202020202\
        a00303030303030303030303030303030303030303030303030303030303030303\
        f8a6\
        a0aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\
        a0bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\
        e1a0cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc\
        a0dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd\
        a00918b65016ec47e1613e6629a797fb3685353e3b8101d0e5250ab52e53f27b88";

    /// The merged hash inside `MASTER_STEADY_ROW`.
    const MASTER_STEADY_STATE_ROOT_HASH: &str =
        "0918b65016ec47e1613e6629a797fb3685353e3b8101d0e5250ab52e53f27b88";

    fn unhex(s: &str) -> Vec<u8> {
        let digits: Vec<u8> = s
            .bytes()
            .filter(|b| !b.is_ascii_whitespace())
            .map(|b| match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => b - b'a' + 10,
                other => panic!("not a hex digit: {}", other as char),
            })
            .collect();
        assert_eq!(digits.len() % 2, 0, "odd number of hex digits");
        digits
            .chunks(2)
            .map(|pair| pair[0] << 4 | pair[1])
            .collect()
    }

    fn hash(byte: u8) -> MerkleHash { MerkleHash::from([byte; 32]) }

    fn hash_of(hex: &str) -> MerkleHash { MerkleHash::from_slice(&unhex(hex)) }

    fn triplet(snapshot: u8, intermediate: u8, delta: u8) -> StateRoot {
        StateRoot {
            snapshot_root: hash(snapshot),
            intermediate_delta_root: hash(intermediate),
            delta_root: hash(delta),
        }
    }

    /// One aux row in the old shape. `option_slot` is the raw content of the
    /// `Option` slot, `None` standing for the empty list an old node wrote
    /// when it had no intermediate key prefix.
    fn old_aux_row(
        snapshot_epoch_id: MerkleHash, intermediate_epoch_id: MerkleHash,
        option_slot: Option<&[u8]>, delta_key_padding: &[u8],
        state_root_hash: MerkleHash,
    ) -> Vec<u8> {
        let mut s = RlpStream::new_list(5);
        s.append(&snapshot_epoch_id);
        s.append(&intermediate_epoch_id);
        match option_slot {
            None => {
                s.begin_list(0);
            }
            Some(padding) => {
                s.begin_list(1).append(&padding);
            }
        }
        s.append(&delta_key_padding);
        s.append(&state_root_hash);
        s.out().to_vec()
    }

    fn old_row(state_root: &StateRoot, aux: &[u8]) -> Vec<u8> {
        let mut s = RlpStream::new_list(2);
        s.append(state_root);
        s.append_raw(aux, 1);
        s.out().to_vec()
    }

    // ---- 1. a row written by an old node decodes here ------------------

    #[test]
    fn old_row_decodes_with_the_new_decoder() {
        let state_root = triplet(0x01, 0x02, 0x03);
        let state_root_hash = state_root.compute_state_root_hash();
        let aux = old_aux_row(
            hash(0xaa),
            hash(0xbb),
            Some(&[0xcc; 32]),
            &[0xdd; 32],
            state_root_hash,
        );
        let bytes = old_row(&state_root, &aux);

        let decoded = StateRootWithAuxInfo::decode(&Rlp::new(&bytes)).unwrap();

        // The one field which kept its meaning.
        assert_eq!(decoded.aux_info.state_root_hash, state_root_hash);
        assert_eq!(decoded.state_root, state_root);
        // The coordinate slots are kept as the raw bytes the old row carried,
        // uninterpreted.
        assert_eq!(decoded.aux_info.placeholder_slot_0.0, [0xaa; 32]);
        assert_eq!(decoded.aux_info.placeholder_slot_1.0, [0xbb; 32]);
        assert_eq!(
            decoded.aux_info.placeholder_slot_2.as_ref().unwrap().0,
            [0xcc; 32]
        );
        assert_eq!(decoded.aux_info.placeholder_slot_3.0, [0xdd; 32]);
    }

    #[test]
    fn old_row_with_an_empty_option_slot_decodes_to_none() {
        // The shape an old node wrote for genesis and for every epoch whose
        // snapshot db was the one of its own intermediate epoch id.
        let state_root = StateRoot::genesis(&hash(0x77));
        let state_root_hash = state_root.compute_state_root_hash();
        let aux = old_aux_row(
            MerkleHash::default(),
            MerkleHash::default(),
            None,
            &[0x9c; 32],
            state_root_hash,
        );
        let bytes = old_row(&state_root, &aux);

        let decoded = StateRootWithAuxInfo::decode(&Rlp::new(&bytes)).unwrap();

        assert!(decoded.aux_info.placeholder_slot_2.is_none());
        assert_eq!(decoded.aux_info.state_root_hash, state_root_hash);
        assert_eq!(decoded.state_root, state_root);
    }

    // ---- 2. a row written here decodes on an old node ------------------

    /// The old type is gone from the tree, so "an old decoder reads it" is
    /// asserted on the shape instead: the old decoder read slots 0, 1 and 4
    /// as `H256`, slot 2 as `Option<DeltaMptKeyPadding>` and slot 3 as
    /// `DeltaMptKeyPadding`, and both padding types demanded exactly 32
    /// bytes. A row which satisfies every one of those demands is a row that
    /// decoder accepts.
    #[test]
    fn new_row_has_the_shape_the_old_decoder_demands() {
        let state_root = triplet(0x01, 0x02, 0x03);
        let bytes = StateRootWithAuxInfo::from_state_root(state_root.clone())
            .rlp_bytes();
        let row = Rlp::new(&bytes);

        assert_eq!(row.item_count().unwrap(), 2);
        // Slot 0 of the row is the triplet, untouched by this change.
        assert_eq!(row.val_at::<StateRoot>(0).unwrap(), state_root);

        let aux = row.at(1).unwrap();
        assert_eq!(aux.item_count().unwrap(), 5);

        // Slots 0, 1 and 4: what the old decoder read as `H256`.
        for slot in [0usize, 1, 4] {
            let item = aux.at(slot).unwrap();
            assert!(item.is_data(), "slot {} is not a byte string", slot);
            assert_eq!(
                item.data().unwrap().len(),
                32,
                "slot {} is not 32 bytes wide",
                slot
            );
            aux.val_at::<MerkleHash>(slot).unwrap();
        }

        // Slot 2: a list of one 32 byte item, which is how the old decoder's
        // `Option<DeltaMptKeyPadding>` reads a `Some`.
        let option_slot = aux.at(2).unwrap();
        assert!(option_slot.is_list());
        assert_eq!(option_slot.item_count().unwrap(), 1);
        assert_eq!(option_slot.at(0).unwrap().data().unwrap().len(), 32);

        // Slot 3: what the old decoder read as `DeltaMptKeyPadding`, which
        // rejected anything other than 32 bytes.
        let padding_slot = aux.at(3).unwrap();
        assert!(padding_slot.is_data());
        assert_eq!(padding_slot.data().unwrap().len(), 32);

        // The four coordinate slots are written as zeroes.
        for slot in [0usize, 1, 3] {
            assert_eq!(aux.at(slot).unwrap().data().unwrap(), &[0u8; 32]);
        }
        assert_eq!(option_slot.at(0).unwrap().data().unwrap(), &[0u8; 32]);
    }

    #[test]
    fn new_row_round_trips_through_the_new_decoder() {
        let original =
            StateRootWithAuxInfo::from_state_root(triplet(0x11, 0x22, 0x33));
        let bytes = original.rlp_bytes();
        let decoded = StateRootWithAuxInfo::decode(&Rlp::new(&bytes)).unwrap();
        assert_eq!(decoded, original);
    }

    // ---- 3. the width is frozen at what the old row gave it -------------

    #[test]
    fn master_genesis_row_decodes_and_keeps_its_bytes() {
        let bytes = unhex(MASTER_GENESIS_ROW);
        let decoded = StateRootWithAuxInfo::decode(&Rlp::new(&bytes)).unwrap();

        assert_eq!(
            decoded.aux_info.state_root_hash,
            hash_of(MASTER_GENESIS_STATE_ROOT_HASH)
        );
        assert_eq!(decoded.state_root.delta_root, hash(0x77));
        assert_eq!(
            decoded.state_root.compute_state_root_hash(),
            decoded.aux_info.state_root_hash,
            "the merged hash of a master row is the hash of its own triplet"
        );
        // The empty `Option` slot of the real genesis row.
        assert!(decoded.aux_info.placeholder_slot_2.is_none());
        // Slot 3 of that row carried the real genesis key prefix; it comes
        // back as the same raw bytes and nothing interprets them.
        assert_eq!(
            &decoded.aux_info.placeholder_slot_3.0[..],
            &unhex(MASTER_GENESIS_DELTA_PADDING)[..]
        );
    }

    #[test]
    fn master_steady_row_decodes_and_keeps_its_bytes() {
        let bytes = unhex(MASTER_STEADY_ROW);
        let decoded = StateRootWithAuxInfo::decode(&Rlp::new(&bytes)).unwrap();

        assert_eq!(
            decoded.aux_info.state_root_hash,
            hash_of(MASTER_STEADY_STATE_ROOT_HASH)
        );
        assert_eq!(decoded.state_root, triplet(0x01, 0x02, 0x03));
        assert_eq!(decoded.aux_info.placeholder_slot_0.0, [0xaa; 32]);
        assert_eq!(decoded.aux_info.placeholder_slot_1.0, [0xbb; 32]);
        assert_eq!(
            decoded.aux_info.placeholder_slot_2.as_ref().unwrap().0,
            [0xcc; 32]
        );
        assert_eq!(decoded.aux_info.placeholder_slot_3.0, [0xdd; 32]);
    }

    /// The frozen width, read straight off the bytes of a `master` row: every
    /// slot of that row is `AUX_INFO_PLACEHOLDER_BYTES` wide. Should the
    /// engine ever change the width of a key prefix and drag this constant
    /// along, this test fails, which is what it is here for.
    #[test]
    fn placeholder_width_is_the_width_of_the_master_row() {
        let bytes = unhex(MASTER_STEADY_ROW);
        let row = Rlp::new(&bytes);
        let aux = row.at(1).unwrap();

        for slot in [0usize, 1, 3] {
            assert_eq!(
                aux.at(slot).unwrap().data().unwrap().len(),
                AUX_INFO_PLACEHOLDER_BYTES
            );
        }
        assert_eq!(
            aux.at(2).unwrap().at(0).unwrap().data().unwrap().len(),
            AUX_INFO_PLACEHOLDER_BYTES
        );
    }

    /// Same width, seen from the other side: a row written here occupies as
    /// many bytes as the `master` row of the same triplet. Both write the
    /// `Option` slot as `Some`, so the two rows differ only in slot content.
    #[test]
    fn new_row_is_as_wide_as_the_master_row() {
        let master = unhex(MASTER_STEADY_ROW);
        let ours =
            StateRootWithAuxInfo::from_state_root(triplet(0x01, 0x02, 0x03))
                .rlp_bytes();

        assert_eq!(ours.len(), master.len());

        let master_aux = Rlp::new(&master).at(1).unwrap();
        let ours_rlp = Rlp::new(&ours);
        let our_aux = ours_rlp.at(1).unwrap();
        for slot in 0..5 {
            assert_eq!(
                our_aux.at(slot).unwrap().as_raw().len(),
                master_aux.at(slot).unwrap().as_raw().len(),
                "slot {} changed width",
                slot
            );
        }
    }

    // ---- 4. the `Option` slot in detail --------------------------------

    #[test]
    fn option_slot_none_is_an_empty_list_and_some_is_a_one_item_list() {
        let with_none = StateRootAuxInfo {
            placeholder_slot_0: Default::default(),
            placeholder_slot_1: Default::default(),
            placeholder_slot_2: None,
            placeholder_slot_3: Default::default(),
            state_root_hash: hash(0x42),
        };
        let bytes = with_none.rlp_bytes();
        let slot = Rlp::new(&bytes).at(2).unwrap();
        assert!(slot.is_list());
        assert_eq!(slot.item_count().unwrap(), 0);
        assert_eq!(slot.as_raw(), &[0xc0u8]);
        assert_eq!(
            StateRootAuxInfo::decode(&Rlp::new(&bytes)).unwrap(),
            with_none
        );

        let with_some = StateRootAuxInfo::new(hash(0x42));
        let bytes = with_some.rlp_bytes();
        let slot = Rlp::new(&bytes).at(2).unwrap();
        assert!(slot.is_list());
        assert_eq!(slot.item_count().unwrap(), 1);
        // 0xe1 = list of 33 payload bytes, 0xa0 = a 32 byte string.
        assert_eq!(&slot.as_raw()[..2], &[0xe1u8, 0xa0]);
        assert_eq!(
            StateRootAuxInfo::decode(&Rlp::new(&bytes)).unwrap(),
            with_some
        );
        // `new` writes `Some`, so that the slot keeps the width the old row
        // gave it.
        assert!(with_some.placeholder_slot_2.is_some());
    }

    #[test]
    fn a_slot_of_the_wrong_width_is_rejected() {
        let state_root_hash = hash(0x42);
        for width in [0usize, 31, 33] {
            let wrong = vec![0x5au8; width];

            // Slot 3, read directly as a placeholder.
            let aux = old_aux_row(
                hash(0xaa),
                hash(0xbb),
                Some(&[0xcc; 32]),
                &wrong,
                state_root_hash,
            );
            let err = StateRootAuxInfo::decode(&Rlp::new(&aux)).unwrap_err();
            assert!(
                matches!(err, DecoderError::RlpInconsistentLengthAndData),
                "slot 3 of width {} gave {:?}",
                width,
                err
            );

            // Slot 2, the same rejection one level down, inside the `Some`.
            let aux = old_aux_row(
                hash(0xaa),
                hash(0xbb),
                Some(&wrong),
                &[0xdd; 32],
                state_root_hash,
            );
            let err = StateRootAuxInfo::decode(&Rlp::new(&aux)).unwrap_err();
            assert!(
                matches!(err, DecoderError::RlpInconsistentLengthAndData),
                "slot 2 of width {} gave {:?}",
                width,
                err
            );
        }
    }

    #[test]
    fn an_option_slot_holding_two_items_is_rejected() {
        let mut s = RlpStream::new_list(5);
        s.append(&hash(0xaa));
        s.append(&hash(0xbb));
        s.begin_list(2)
            .append(&&[0xcc; 32][..])
            .append(&&[0xce; 32][..]);
        s.append(&&[0xdd; 32][..]);
        s.append(&hash(0x42));
        let aux = s.out().to_vec();

        assert!(StateRootAuxInfo::decode(&Rlp::new(&aux)).is_err());
    }

    #[test]
    fn an_option_slot_which_is_not_a_list_is_rejected() {
        let mut s = RlpStream::new_list(5);
        s.append(&hash(0xaa));
        s.append(&hash(0xbb));
        s.append(&&[0xcc; 32][..]);
        s.append(&&[0xdd; 32][..]);
        s.append(&hash(0x42));
        let aux = s.out().to_vec();

        assert!(StateRootAuxInfo::decode(&Rlp::new(&aux)).is_err());
    }
}
