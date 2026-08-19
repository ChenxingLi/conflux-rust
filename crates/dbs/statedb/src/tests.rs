// Copyright 2020 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

use super::{inmemory_manager::InmemoryManager, StateDbGeneric};
use cfx_storage::utils::access_mode;
use primitives::{EpochId, StorageKey, StorageKeyWithSpace, MERKLE_NULL_NODE};
use std::collections::BTreeMap;

type StorageValue = Box<[u8]>;
type RawStorage = BTreeMap<Vec<u8>, StorageValue>;

type StateDbTest = StateDbGeneric;

/// The epoch these tests read their fixture out of; it was seeded into the
/// in-memory storage engine, never committed.
fn base_epoch() -> EpochId { EpochId::from_low_u64_be(1) }

// convert `key` to storage interface format
fn storage_key(key: &'static [u8]) -> StorageKeyWithSpace<'static> {
    StorageKey::AccountKey(key).with_native_space()
}

// convert `key` to raw storage format
fn key(key: &'static [u8]) -> Vec<u8> { storage_key(key).to_key_bytes() }

// convert `value` to raw storage format
fn value(value: &'static [u8]) -> StorageValue { value.into() }

fn init_state_db() -> StateDbTest {
    let mut contents = RawStorage::new();
    contents.insert(key(b"00"), value(b"v0"));
    contents.insert(key(b"01"), value(b"v0"));
    contents.insert(key(b"11"), value(b"v0"));
    contents.insert(key(b"22"), value(b"v0"));

    let manager = InmemoryManager::new();
    manager.seed(base_epoch(), contents);
    StateDbTest::new_for_commit(manager, base_epoch(), 1)
        .expect("the base epoch was just seeded")
}

#[allow(unused)]
fn print_raw(raw: &RawStorage) {
    let mut keys: Vec<_> = raw.keys().collect();
    keys.sort();

    for k in keys {
        let v = &raw[k];
        println!(
            "k = {:?}; v = {:?}",
            std::str::from_utf8(k).unwrap(),
            std::str::from_utf8(v).unwrap()
        );
    }
}

#[test]
fn test_basic() {
    let mut state_db = init_state_db();

    // (11, v0) --> (11, v1)
    state_db
        .set_raw(storage_key(b"11"), value(b"v1"), None)
        .unwrap();

    // delete (22, v0)
    state_db.delete(storage_key(b"22"), None).unwrap();

    // delete (00, v0) and (01, v0)
    state_db
        .delete_all::<access_mode::Write>(storage_key(b"0"), None)
        .unwrap();

    state_db.commit(MERKLE_NULL_NODE, None).unwrap();
    // FIXME(lpl): Enable tests. The disabled assertions checked the contents
    // of the committed epoch and how many reads and writes reaching it took;
    // they have to be rewritten against the storage engine behind `StateDb`,
    // which is what holds the committed epoch now.
}
