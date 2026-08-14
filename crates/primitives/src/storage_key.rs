// Copyright 2019 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

use crate::static_bool::{self, StaticBool};

pub type CheckInput = static_bool::Yes;
pub type SkipInputCheck = static_bool::No;

/// `cfg!` is evaluated per crate, so the storage engine cannot test this
/// crate's feature; it reads this constant instead.
pub const NO_ACCOUNT_LENGTH_CHECK: bool =
    cfg!(feature = "test_no_account_length_check");

/// Width of the account key part of a delta MPT key, used here only to locate
/// the space flag byte.
pub const DELTA_MPT_ACCOUNT_KEYPART_BYTES: usize = 32;

pub trait ConditionalReturnValue<'a> {
    type Output;

    fn from_key(k: StorageKeyWithSpace<'a>) -> Self::Output;
    fn from_result(r: Result<StorageKeyWithSpace<'a>, String>) -> Self::Output;
}

pub struct FromKeyBytesResult<ShouldCheckInput: StaticBool> {
    phantom: std::marker::PhantomData<ShouldCheckInput>,
}

impl<'a> ConditionalReturnValue<'a> for FromKeyBytesResult<SkipInputCheck> {
    type Output = StorageKeyWithSpace<'a>;

    fn from_key(k: StorageKeyWithSpace<'a>) -> Self::Output { k }

    fn from_result(
        _r: Result<StorageKeyWithSpace<'a>, String>,
    ) -> Self::Output {
        unreachable!()
    }
}

impl<'a> ConditionalReturnValue<'a> for FromKeyBytesResult<CheckInput> {
    type Output = Result<StorageKeyWithSpace<'a>, String>;

    fn from_key(k: StorageKeyWithSpace<'a>) -> Self::Output { Ok(k) }

    fn from_result(r: Result<StorageKeyWithSpace<'a>, String>) -> Self::Output {
        r
    }
}

// The original StorageKeys unprocessed, in contrary to StorageKey which is
// processed to use in DeltaMpt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StorageKey<'a> {
    AccountKey(&'a [u8]),
    StorageRootKey(&'a [u8]),
    StorageKey {
        address_bytes: &'a [u8],
        storage_key: &'a [u8],
    },
    CodeRootKey(&'a [u8]),
    CodeKey {
        address_bytes: &'a [u8],
        code_hash_bytes: &'a [u8],
    },
    DepositListKey(&'a [u8]),
    VoteListKey(&'a [u8]),
    // Empty key is used to traverse all key and value pairs.
    EmptyKey,
    // Address prefix key is used to search all keys with the same address
    // prefix, eg [1, 2](0x0102) will search all keys with prefix 0x0102
    AddressPrefixKey(&'a [u8]),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StorageKeyWithSpace<'a> {
    pub key: StorageKey<'a>,
    pub space: Space,
}

impl<'a> StorageKey<'a> {
    pub const fn with_space(self, space: Space) -> StorageKeyWithSpace<'a> {
        StorageKeyWithSpace { key: self, space }
    }

    pub const fn with_native_space(self) -> StorageKeyWithSpace<'a> {
        self.with_space(Space::Native)
    }

    pub const fn with_evm_space(self) -> StorageKeyWithSpace<'a> {
        self.with_space(Space::Ethereum)
    }

    pub const fn new_account_key(address: &'a Address) -> Self {
        StorageKey::AccountKey(&address.0)
    }

    pub const fn new_storage_root_key(address: &'a Address) -> Self {
        StorageKey::StorageRootKey(&address.0)
    }

    pub const fn new_storage_key(
        address: &'a Address, storage_key: &'a [u8],
    ) -> Self {
        StorageKey::StorageKey {
            address_bytes: &address.0,
            storage_key,
        }
    }

    pub fn new_code_root_key(address: &'a Address) -> Self {
        StorageKey::CodeRootKey(&address.0)
    }

    pub fn new_code_key(address: &'a Address, code_hash: &'a H256) -> Self {
        StorageKey::CodeKey {
            address_bytes: &address.0,
            code_hash_bytes: &code_hash.0,
        }
    }

    pub fn new_deposit_list_key(address: &'a Address) -> Self {
        StorageKey::DepositListKey(&address.0)
    }

    pub fn new_vote_list_key(address: &'a Address) -> Self {
        StorageKey::VoteListKey(&address.0)
    }
}

impl<'a> StorageKey<'a> {
    // Compatible interface with rpc
    pub fn to_key_bytes(&self) -> Vec<u8> {
        (*self).with_native_space().to_key_bytes()
    }
}

// Conversion methods.
impl<'a> StorageKeyWithSpace<'a> {
    // Public because the delta MPT key encoder in cfx-storage builds the same
    // layout.
    pub const ACCOUNT_BYTES: usize = 20;
    pub const CODE_HASH_BYTES: usize = 32;
    pub const CODE_HASH_PREFIX: &'static [u8] = b"code";
    pub const CODE_HASH_PREFIX_LEN: usize = 4;
    pub const DEPOSIT_LIST_LEN: usize = 7;
    pub const DEPOSIT_LIST_PREFIX: &'static [u8] = b"deposit";
    pub const EVM_SPACE_TYPE: &'static [u8] = b"\x81";
    pub const STORAGE_PREFIX: &'static [u8] = b"data";
    pub const STORAGE_PREFIX_LEN: usize = 4;
    pub const VOTE_LIST_LEN: usize = 4;
    pub const VOTE_LIST_PREFIX: &'static [u8] = b"vote";

    pub fn to_key_bytes(&self) -> Vec<u8> {
        let key_bytes = match self.key {
            StorageKey::AccountKey(address_bytes) => {
                let mut key = Vec::with_capacity(Self::ACCOUNT_BYTES);
                key.extend_from_slice(address_bytes);

                key
            }
            StorageKey::StorageRootKey(address_bytes) => {
                let mut key = Vec::with_capacity(
                    Self::ACCOUNT_BYTES + Self::STORAGE_PREFIX_LEN,
                );
                key.extend_from_slice(address_bytes);
                key.extend_from_slice(Self::STORAGE_PREFIX);

                key
            }
            StorageKey::StorageKey {
                address_bytes,
                storage_key,
            } => {
                let mut key = Vec::with_capacity(
                    Self::ACCOUNT_BYTES
                        + Self::STORAGE_PREFIX_LEN
                        + storage_key.len(),
                );
                key.extend_from_slice(address_bytes);
                key.extend_from_slice(Self::STORAGE_PREFIX);
                key.extend_from_slice(storage_key);

                key
            }
            StorageKey::CodeRootKey(address_bytes) => {
                let mut key = Vec::with_capacity(
                    Self::ACCOUNT_BYTES + Self::CODE_HASH_PREFIX_LEN,
                );
                key.extend_from_slice(address_bytes);
                key.extend_from_slice(Self::CODE_HASH_PREFIX);

                key
            }
            StorageKey::CodeKey {
                address_bytes,
                code_hash_bytes,
            } => {
                let mut key = Vec::with_capacity(
                    Self::ACCOUNT_BYTES
                        + Self::CODE_HASH_PREFIX_LEN
                        + Self::CODE_HASH_BYTES,
                );
                key.extend_from_slice(address_bytes);
                key.extend_from_slice(Self::CODE_HASH_PREFIX);
                key.extend_from_slice(code_hash_bytes);

                key
            }
            StorageKey::DepositListKey(address_bytes) => {
                let mut key = Vec::with_capacity(
                    Self::ACCOUNT_BYTES + Self::DEPOSIT_LIST_LEN,
                );
                key.extend_from_slice(address_bytes);
                key.extend_from_slice(Self::DEPOSIT_LIST_PREFIX);

                key
            }
            StorageKey::VoteListKey(address_bytes) => {
                let mut key = Vec::with_capacity(
                    Self::ACCOUNT_BYTES + Self::VOTE_LIST_LEN,
                );
                key.extend_from_slice(address_bytes);
                key.extend_from_slice(Self::VOTE_LIST_PREFIX);

                key
            }
            StorageKey::EmptyKey => {
                return vec![];
            }
            StorageKey::AddressPrefixKey(address_bytes) => {
                let mut key = Vec::with_capacity(address_bytes.len());
                key.extend_from_slice(address_bytes);
                return key;
            }
        };

        if self.space == Space::Native {
            key_bytes
        } else {
            // Insert "0x81" at the position 20.
            [
                &key_bytes[..Self::ACCOUNT_BYTES],
                Self::EVM_SPACE_TYPE,
                &key_bytes[Self::ACCOUNT_BYTES..],
            ]
            .concat()
        }
    }

    // from_key_bytes::<CheckInput>(...) returns Result<StorageKey, String>
    // from_key_bytes::<SkipInputCheck>(...) returns StorageKey, crashes on
    // error
    pub fn from_key_bytes<ShouldCheckInput: StaticBool>(
        bytes: &'a [u8],
    ) -> <FromKeyBytesResult<ShouldCheckInput> as ConditionalReturnValue<'a>>::Output
    where FromKeyBytesResult<ShouldCheckInput>: ConditionalReturnValue<'a>{
        let key = if bytes.len() <= Self::ACCOUNT_BYTES {
            StorageKey::AccountKey(bytes).with_native_space()
        } else if bytes.len() == Self::ACCOUNT_BYTES + 1 {
            StorageKey::AccountKey(&bytes[..Self::ACCOUNT_BYTES])
                .with_evm_space()
        } else {
            let address_bytes = &bytes[0..Self::ACCOUNT_BYTES];

            let extension_bit: bool = bytes[Self::ACCOUNT_BYTES] & 0x80 != 0;
            let bytes = if extension_bit {
                // check for incorrect extension byte
                if ShouldCheckInput::value() == CheckInput::value()
                    && bytes[Self::ACCOUNT_BYTES] != Self::EVM_SPACE_TYPE[0]
                {
                    return <FromKeyBytesResult<ShouldCheckInput> as ConditionalReturnValue<'a>>::from_result(
                        Err(format!("Unexpected extension byte: {:?}", bytes[Self::ACCOUNT_BYTES]))
                    );
                }

                // with `SkipInputCheck`, crash on incorrect extension byte
                assert_eq!(bytes[Self::ACCOUNT_BYTES], Self::EVM_SPACE_TYPE[0]);
                &bytes[(Self::ACCOUNT_BYTES + 1)..]
            } else {
                &bytes[Self::ACCOUNT_BYTES..]
            };

            let storage_key_no_space = if bytes
                .starts_with(Self::STORAGE_PREFIX)
            {
                let bytes = &bytes[Self::STORAGE_PREFIX_LEN..];
                if !bytes.is_empty() {
                    StorageKey::StorageKey {
                        address_bytes,
                        storage_key: bytes,
                    }
                } else {
                    StorageKey::StorageRootKey(address_bytes)
                }
            } else if bytes.starts_with(Self::CODE_HASH_PREFIX) {
                let bytes = &bytes[Self::CODE_HASH_PREFIX_LEN..];
                if !bytes.is_empty() {
                    StorageKey::CodeKey {
                        address_bytes,
                        code_hash_bytes: bytes,
                    }
                } else {
                    StorageKey::CodeRootKey(address_bytes)
                }
            } else if bytes.starts_with(Self::DEPOSIT_LIST_PREFIX) {
                StorageKey::DepositListKey(address_bytes)
            } else if bytes.starts_with(Self::VOTE_LIST_PREFIX) {
                StorageKey::VoteListKey(address_bytes)
            }
            // unknown key format => we report an error or crash
            // depending on the generic parameter
            else if ShouldCheckInput::value() == CheckInput::value() {
                return <FromKeyBytesResult<ShouldCheckInput> as ConditionalReturnValue<'a>>::from_result(
                    Err(format!("Unable to parse storage key: {:?} - {:?}", address_bytes, bytes))
                );
            } else {
                unreachable!(
                    "Invalid key format. Unrecognized: {:?}, account: {:?}",
                    bytes, address_bytes
                );
            };

            let space = if extension_bit {
                Space::Ethereum
            } else {
                Space::Native
            };

            storage_key_no_space.with_space(space)
        };

        <FromKeyBytesResult<ShouldCheckInput> as ConditionalReturnValue<'a>>::from_key(key)
    }
}

// This enum is used to filter only the wanted contract storage key when
// traversal the trie for example, when traverse eth space key/value, we can
// only filter the eSpace storage key/value to accelerate the traversal
// speed
// Native means filter(keep) the native space storage key/value
// Ethereum means filter(keep) the ethereum space storage key/value
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpaceStorageFilter(pub Space);

impl From<Space> for SpaceStorageFilter {
    fn from(space: Space) -> Self { SpaceStorageFilter(space) }
}

impl From<SpaceStorageFilter> for Space {
    fn from(filter: SpaceStorageFilter) -> Self { filter.0 }
}

impl SpaceStorageFilter {
    pub fn is_native(&self) -> bool { matches!(self.0, Space::Native) }

    pub fn is_ethereum(&self) -> bool { matches!(self.0, Space::Ethereum) }

    // return the flag index according the trie type
    // if is_delta_mpt is true, then the space flag is at the 32th index
    // otherwise, the space flag is at the 20th index
    pub fn space_flag_index(is_delta_mpt: bool) -> usize {
        if is_delta_mpt {
            DELTA_MPT_ACCOUNT_KEYPART_BYTES
        } else {
            StorageKeyWithSpace::ACCOUNT_BYTES
        }
    }

    // return true if the key is filtered out
    pub fn is_filtered(&self, is_delta_mpt: bool, key: &[u8]) -> bool {
        let flag_index = Self::space_flag_index(is_delta_mpt);
        if key.len() > flag_index {
            match self.0 {
                Space::Native => {
                    if key[flag_index] == StorageKeyWithSpace::EVM_SPACE_TYPE[0]
                    {
                        return true;
                    }
                }
                Space::Ethereum => {
                    if key[flag_index] != StorageKeyWithSpace::EVM_SPACE_TYPE[0]
                    {
                        return true;
                    }
                }
            }
        }
        false
    }
}

use cfx_types::{Address, Space, H256};
use std::vec::Vec;
