// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

// Copyright 2021 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

use diem_crypto::{
    hash::{CryptoHash, CryptoHasher},
    HashValue,
};
use diem_crypto_derive::CryptoHasher;
#[cfg(any(test, feature = "fuzzing"))]
use proptest::{arbitrary::Arbitrary, prelude::*};
use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize, CryptoHasher)]
pub struct AccountStateBlob {
    blob: Vec<u8>,
}

impl fmt::Debug for AccountStateBlob {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "AccountStateBlob {{ \n \
             Raw: 0x{} \n \
             }}",
            hex::encode(&self.blob),
        )
    }
}

impl AsRef<[u8]> for AccountStateBlob {
    fn as_ref(&self) -> &[u8] { &self.blob }
}

impl From<&AccountStateBlob> for Vec<u8> {
    fn from(account_state_blob: &AccountStateBlob) -> Vec<u8> {
        account_state_blob.blob.clone()
    }
}

impl From<AccountStateBlob> for Vec<u8> {
    fn from(account_state_blob: AccountStateBlob) -> Vec<u8> {
        Self::from(&account_state_blob)
    }
}

impl From<Vec<u8>> for AccountStateBlob {
    fn from(blob: Vec<u8>) -> AccountStateBlob { AccountStateBlob { blob } }
}

impl CryptoHash for AccountStateBlob {
    type Hasher = AccountStateBlobHasher;

    fn hash(&self) -> HashValue {
        let mut hasher = Self::Hasher::default();
        hasher.update(&self.blob);
        hasher.finish()
    }
}

#[cfg(any(test, feature = "fuzzing"))]
prop_compose! {
    fn account_state_blob_strategy()(
        entries in proptest::collection::btree_map(
            proptest::collection::vec(any::<u8>(), 1..32),
            proptest::collection::vec(any::<u8>(), 1..64),
            0..5,
        )
    ) -> AccountStateBlob {
        AccountStateBlob::from(bcs::to_bytes(&entries).unwrap())
    }
}

#[cfg(any(test, feature = "fuzzing"))]
impl Arbitrary for AccountStateBlob {
    type Parameters = ();
    type Strategy = BoxedStrategy<Self>;

    fn arbitrary_with(_args: Self::Parameters) -> Self::Strategy {
        account_state_blob_strategy().boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bcs::test_helpers::assert_canonical_encode_decode;
    use proptest::collection::vec;

    fn hash_blob(blob: &[u8]) -> HashValue {
        let mut hasher = AccountStateBlobHasher::default();
        hasher.update(blob);
        hasher.finish()
    }

    proptest! {
        #[test]
        fn account_state_blob_hash(blob in vec(any::<u8>(), 1..100)) {
            prop_assert_eq!(hash_blob(&blob), AccountStateBlob::from(blob).hash());
        }

        #[test]
        fn account_state_blob_bcs_roundtrip(account_state_blob in any::<AccountStateBlob>()) {
            assert_canonical_encode_decode(account_state_blob);
        }
    }

    #[test]
    fn test_debug_does_not_panic() {
        let _ = format!("{:#?}", AccountStateBlob::from(vec![1u8, 2u8, 3u8]));
    }
}
