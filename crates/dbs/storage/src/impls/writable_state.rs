use crate::{
    impls::{errors::*, single_mpt_state::SingleMptState, state::State},
    state::{replay_changeset, Changeset, CommitMeta, StorageView},
    MptKeyValue,
};
use cfx_internal_common::StateRootWithAuxInfo;
use cfx_types::Space;
use parking_lot::Mutex;
use primitives::{EpochId, StateRoot, StorageKey, StorageKeyWithSpace};
use std::{
    sync::mpsc::{channel, Sender},
    thread::{self, JoinHandle},
};

/// The writable state of one epoch: the layered state the engine executes on,
/// plus the single MPT replica when this node keeps one.
///
/// Every writable state the engine hands out is one of these. With
/// `replication` at `None` every method here is a plain forward to the layered
/// state.
pub(crate) struct WritableState {
    state: State,
    replication: Option<ReplicationHandler>,
}

pub trait StateFilter: Sync + Send {
    fn keep_key(&self, _key: &StorageKeyWithSpace) -> bool;
}

impl StateFilter for Space {
    fn keep_key(&self, key: &StorageKeyWithSpace) -> bool { key.space == *self }
}

impl WritableState {
    /// A writable state that writes to the layered state alone.
    pub(crate) fn plain(state: State) -> Self {
        Self {
            state,
            replication: None,
        }
    }

    /// A writable state that also mirrors every write into `replica`, keeping
    /// the keys `filter` accepts.
    pub(crate) fn replicated(
        state: State, replica: SingleMptState,
        filter: Option<Box<dyn StateFilter>>,
    ) -> Self {
        Self {
            state,
            replication: Some(ReplicationHandler::new(replica, filter)),
        }
    }

    fn send_op(&self, op: StateOperation) {
        if let Some(replication) = &self.replication {
            replication.send_op(op);
        }
    }
}

struct ReplicationHandler {
    filter: Option<Box<dyn StateFilter>>,
    // We need `Mutex` to make the struct `Sync`.
    op_sender: Mutex<Sender<StateOperation>>,
    thread_handle: Option<JoinHandle<Result<()>>>,
}

impl ReplicationHandler {
    fn new(
        mut replicated_state: SingleMptState,
        filter: Option<Box<dyn StateFilter>>,
    ) -> ReplicationHandler {
        let (op_tx, op_rx) = channel();
        let thread_handle = thread::Builder::new()
            .name("state_replication".into())
            .spawn(move || {
                for op in op_rx {
                    trace!("replicated_state: op={:?}", op);
                    let err = match op {
                        StateOperation::Set { access_key, value } => {
                            replicated_state
                                .set(access_key.as_storage_key(), value)
                                .err()
                        }
                        StateOperation::Delete { access_key } => {
                            replicated_state
                                .delete(access_key.as_storage_key())
                                .err()
                        }
                        StateOperation::ComputeStateRoot => {
                            replicated_state.compute_state_root().err()
                        }
                        StateOperation::Commit { epoch_id } => {
                            return replicated_state
                                .commit(epoch_id)
                                .map(|_| ());
                        }
                    };
                    if let Some(e) = err {
                        error!("StateReplication Error: err={:?}", e);
                        return Err(e);
                    }
                }
                Ok(())
            })
            .expect("spawn error");
        Self {
            filter,
            op_sender: Mutex::new(op_tx),
            thread_handle: Some(thread_handle),
        }
    }

    fn send_op(&self, op: StateOperation) {
        if let Some(filter) = &self.filter {
            if let Some(key) = op.get_key() {
                if !filter.keep_key(&key) {
                    // This key should not be stored in the replicated state.
                    return;
                }
            }
        }
        if let Err(e) = self.op_sender.lock().send(op) {
            error!("send_op: err={:?}", e);
        }
    }
}

#[derive(Debug)]
enum StateOperation {
    Set {
        access_key: OwnedStorageKeyWithSpace,
        value: Box<[u8]>,
    },
    Delete {
        access_key: OwnedStorageKeyWithSpace,
    },
    ComputeStateRoot,
    Commit {
        epoch_id: EpochId,
    },
}

impl StateOperation {
    fn get_key(&self) -> Option<StorageKeyWithSpace<'_>> {
        match self {
            StateOperation::Set { access_key, .. }
            | StateOperation::Delete { access_key, .. } => {
                Some(access_key.as_storage_key())
            }
            StateOperation::ComputeStateRoot
            | StateOperation::Commit { .. } => None,
        }
    }
}

#[derive(Debug)]
enum OwnedStorageKey {
    AccountKey(Vec<u8>),
    StorageRootKey(Vec<u8>),
    StorageKey {
        address_bytes: Vec<u8>,
        storage_key: Vec<u8>,
    },
    CodeRootKey(Vec<u8>),
    CodeKey {
        address_bytes: Vec<u8>,
        code_hash_bytes: Vec<u8>,
    },
    DepositListKey(Vec<u8>),
    VoteListKey(Vec<u8>),
    EmptyKey,
    AddressPrefixKey(Vec<u8>),
}

impl OwnedStorageKey {
    fn as_storage_key(&self) -> StorageKey<'_> {
        match &self {
            OwnedStorageKey::AccountKey(k) => {
                StorageKey::AccountKey(k.as_slice())
            }
            OwnedStorageKey::StorageRootKey(k) => {
                StorageKey::StorageRootKey(k.as_slice())
            }
            OwnedStorageKey::StorageKey {
                address_bytes,
                storage_key,
            } => StorageKey::StorageKey {
                address_bytes: address_bytes.as_slice(),
                storage_key: &storage_key,
            },
            OwnedStorageKey::CodeRootKey(k) => {
                StorageKey::CodeRootKey(k.as_slice())
            }
            OwnedStorageKey::CodeKey {
                address_bytes,
                code_hash_bytes,
            } => StorageKey::CodeKey {
                address_bytes: &address_bytes,
                code_hash_bytes: &code_hash_bytes,
            },
            OwnedStorageKey::DepositListKey(k) => {
                StorageKey::DepositListKey(k.as_slice())
            }
            OwnedStorageKey::VoteListKey(k) => {
                StorageKey::VoteListKey(k.as_slice())
            }
            OwnedStorageKey::EmptyKey => StorageKey::EmptyKey,
            OwnedStorageKey::AddressPrefixKey(k) => {
                StorageKey::AddressPrefixKey(k.as_slice())
            }
        }
    }
}

#[derive(Debug)]
struct OwnedStorageKeyWithSpace {
    pub key: OwnedStorageKey,
    pub space: Space,
}

impl OwnedStorageKeyWithSpace {
    fn as_storage_key(&self) -> StorageKeyWithSpace<'_> {
        StorageKeyWithSpace {
            key: self.key.as_storage_key(),
            space: self.space,
        }
    }
}

impl<'a> From<StorageKey<'a>> for OwnedStorageKey {
    fn from(ref_key: StorageKey<'a>) -> Self {
        match ref_key {
            StorageKey::AccountKey(k) => {
                OwnedStorageKey::AccountKey(k.to_vec())
            }
            StorageKey::StorageRootKey(k) => {
                OwnedStorageKey::StorageRootKey(k.to_vec())
            }
            StorageKey::StorageKey {
                address_bytes,
                storage_key,
            } => OwnedStorageKey::StorageKey {
                address_bytes: address_bytes.to_vec(),
                storage_key: storage_key.to_vec(),
            },
            StorageKey::CodeRootKey(k) => {
                OwnedStorageKey::CodeRootKey(k.to_vec())
            }
            StorageKey::CodeKey {
                address_bytes,
                code_hash_bytes,
            } => OwnedStorageKey::CodeKey {
                address_bytes: address_bytes.to_vec(),
                code_hash_bytes: code_hash_bytes.to_vec(),
            },
            StorageKey::DepositListKey(k) => {
                OwnedStorageKey::DepositListKey(k.to_vec())
            }
            StorageKey::VoteListKey(k) => {
                OwnedStorageKey::VoteListKey(k.to_vec())
            }
            StorageKey::EmptyKey => OwnedStorageKey::EmptyKey,
            StorageKey::AddressPrefixKey(k) => {
                OwnedStorageKey::AddressPrefixKey(k.to_vec())
            }
        }
    }
}

impl<'a> From<StorageKeyWithSpace<'a>> for OwnedStorageKeyWithSpace {
    fn from(ref_key: StorageKeyWithSpace<'a>) -> Self {
        Self {
            key: ref_key.key.into(),
            space: ref_key.space,
        }
    }
}

impl StorageView for WritableState {
    fn get(
        &self, access_key: StorageKeyWithSpace,
    ) -> Result<Option<Box<[u8]>>> {
        self.state.get(access_key)
    }

    fn iter_prefix(
        &self, access_key_prefix: StorageKeyWithSpace,
    ) -> Result<Box<dyn Iterator<Item = MptKeyValue>>> {
        self.state.iter_prefix(access_key_prefix)
    }
}

impl WritableState {
    pub(crate) fn set(
        &mut self, access_key: StorageKeyWithSpace, value: Box<[u8]>,
    ) -> Result<()> {
        self.send_op(StateOperation::Set {
            access_key: access_key.into(),
            value: value.clone(),
        });
        self.state.set(access_key, value)
    }

    pub(crate) fn delete(
        &mut self, access_key: StorageKeyWithSpace,
    ) -> Result<()> {
        self.send_op(StateOperation::Delete {
            access_key: access_key.into(),
        });
        self.state.delete(access_key)
    }

    pub(crate) fn compute_state_root(
        &mut self,
    ) -> Result<StateRootWithAuxInfo> {
        self.send_op(StateOperation::ComputeStateRoot);
        self.state.compute_state_root()
    }

    pub(crate) fn get_state_root(&self) -> Result<StateRootWithAuxInfo> {
        self.state.get_state_root()
    }

    pub(crate) fn commit(
        &mut self, epoch_id: EpochId,
    ) -> Result<StateRootWithAuxInfo> {
        let r = self.state.commit(epoch_id);
        self.send_op(StateOperation::Commit { epoch_id });
        if let Some(replication) = &mut self.replication {
            // TODO(lpl): This can be probably delayed.
            replication
                .thread_handle
                .take()
                .expect("only commit once")
                .join()
                .expect("ReplicationHandler thread join error")?;
        }
        r
    }

    /// Apply a whole changeset, in key order.
    fn apply_changeset(&mut self, changeset: &Changeset) -> Result<()> {
        replay_changeset(changeset, |access_key, value| match value {
            Some(value) => self.set(access_key, value),
            None => self.delete(access_key),
        })
    }

    /// The commit entry point: one changeset plus the meta of the epoch it
    /// belongs to, in a single call. Apply, compute the root, persist.
    ///
    /// The root is read back rather than recomputed when it is already there.
    /// Normally the delta nodes are still dirty, `get_state_root` fails on
    /// them, and the root is computed here.
    ///
    /// The answer is the consensus commitment, the state root triplet alone.
    /// The physical coordinates stay with the engine's own index.
    pub(crate) fn commit_changeset(
        &mut self, changeset: Changeset, meta: CommitMeta,
    ) -> Result<StateRoot> {
        self.apply_changeset(&changeset)?;

        let state_root = match self.get_state_root() {
            Ok(root) => root,
            Err(_) => self.compute_state_root()?,
        };

        self.commit(meta.epoch_id)?;

        Ok(state_root.state_root)
    }
}
