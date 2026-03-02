// Copyright 2019 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

use cfx_bytes as bytes;
use cfxkey as keylib;
use log::{debug, info, trace, warn};

use crate::bytes::Bytes;
use cfx_types::{
    address_util::AddressUtil, Address, AddressSpaceUtil, BigEndianHash, H256,
    H512, U256, U512,
};
use cfx_vm_types::{contract_address, CreateContractAddress};
use cfxcore::{
    SharedConsensusGraph, SharedSynchronizationService, SharedTransactionPool,
};
use keylib::{public_to_address, Generator, KeyPair, Random, Secret};
use lazy_static::lazy_static;
use metrics::{register_meter_with_group, Meter};
use parking_lot::RwLock;
use primitives::{
    transaction::{native_transaction::NativeTransaction, Action},
    Account, SignedTransaction, Transaction,
};
use rand::{prelude::*, random_range};
use rlp::Encodable;
use rustc_hex::{FromHex, ToHex};
use secret_store::SharedSecretStore;
use std::{
    collections::HashMap,
    convert::TryFrom,
    sync::Arc,
    thread,
    time::{self, Instant},
};
use time::Duration;

lazy_static! {
    static ref TX_GEN_METER: Arc<dyn Meter> =
        register_meter_with_group("system_metrics", "tx_gen");
    static ref TX_GEN_ERROR_METER: Arc<dyn Meter> =
        register_meter_with_group("system_metrics", "tx_gen_error");
}

enum TransGenState {
    Start,
    Stop,
}

pub struct TransactionGeneratorConfig {
    pub generate_tx: bool,
    pub period: time::Duration,
    pub account_count: usize,
    pub batch_size: usize,
}

impl TransactionGeneratorConfig {
    pub fn new(
        generate_tx: bool, period_ms: u64, account_count: usize,
        batch_size: usize,
    ) -> Self {
        TransactionGeneratorConfig {
            generate_tx,
            period: time::Duration::from_micros(period_ms),
            account_count,
            batch_size,
        }
    }
}

pub struct TransactionGenerator {
    consensus: SharedConsensusGraph,
    sync: SharedSynchronizationService,
    txpool: SharedTransactionPool,
    secret_store: SharedSecretStore,
    state: RwLock<TransGenState>,
    account_start_index: RwLock<Option<usize>>,
    join_handle: RwLock<Option<thread::JoinHandle<()>>>,
}

pub type SharedTransactionGenerator = Arc<TransactionGenerator>;

impl TransactionGenerator {
    // FIXME: rename to start and return Result<Self>
    pub fn new(
        consensus: SharedConsensusGraph, txpool: SharedTransactionPool,
        sync: SharedSynchronizationService, secret_store: SharedSecretStore,
    ) -> Self {
        TransactionGenerator {
            consensus,
            txpool,
            sync,
            secret_store,
            state: RwLock::new(TransGenState::Start),
            account_start_index: RwLock::new(Option::None),
            join_handle: RwLock::new(None),
        }
    }

    pub fn stop(&self) {
        *self.state.write() = TransGenState::Stop;
        if let Some(join_handle) = self.join_handle.write().take() {
            join_handle.join().ok();
        }
    }

    pub fn set_genesis_accounts_start_index(&self, index: usize) {
        let mut account_start = self.account_start_index.write();
        *account_start = Some(index);
    }

    pub fn set_join_handle(&self, join_handle: thread::JoinHandle<()>) {
        self.join_handle.write().replace(join_handle);
    }

    pub fn generate_transactions_with_multiple_genesis_accounts(
        txgen: Arc<TransactionGenerator>,
        tx_config: TransactionGeneratorConfig,
        _genesis_accounts: HashMap<Address, U256>,
    ) {
        loop {
            let account_start = txgen.account_start_index.read();
            if account_start.is_some() {
                break;
            }
        }
        let account_start_index = txgen.account_start_index.read().unwrap();

        let mut address_secret_pair: HashMap<Address, Secret> = HashMap::new();
        let mut addresses: Vec<Address> = Vec::new();

        debug!("Tx Generation Config {:?}", tx_config.generate_tx);

        let mut tx_n = 0;
        // Wait for initial tx
        loop {
            match *txgen.state.read() {
                TransGenState::Stop => return,
                _ => {}
            }

            // Do not generate tx in catch_up_mode
            if txgen.sync.catch_up_mode() {
                thread::sleep(Duration::from_millis(100));
                continue;
            }
            break;
        }

        debug!("Setup Usable Genesis Accounts");
        for i in 0..tx_config.account_count {
            let key_pair =
                txgen.secret_store.get_keypair(account_start_index + i);
            let address = key_pair.address();
            let secret = key_pair.secret().clone();
            addresses.push(address);
            address_secret_pair.insert(address, secret);
        }

        info!("Start Generating Workload");
        let start_time = Instant::now();
        // Generate more tx

        let account_count = address_secret_pair.len();
        loop {
            match *txgen.state.read() {
                TransGenState::Stop => return,
                _ => {}
            }

            // Randomly select sender and receiver.
            // Sender and receiver must exist in the account list.
            let mut receiver_index: usize = random_range(0..usize::MAX);
            receiver_index %= account_count;
            let receiver_address = addresses[receiver_index];

            let mut sender_index: usize = random_range(0..usize::MAX);
            sender_index %= account_count;
            let sender_address = addresses[sender_index];

            // Always send value 0
            let balance_to_transfer = U256::from(0);

            // Generate nonce for the transaction
            let maybe_next_nonce = txgen.txpool.get_next_nonce_for_tx_gen(
                &sender_address.with_native_space(),
                21000.into(),
            );
            let sender_nonce = if let Some(nonce) = maybe_next_nonce {
                nonce
            } else {
                addresses.remove(sender_index);
                if addresses.is_empty() {
                    break;
                } else {
                    continue;
                }
            };
            trace!(
                "receiver={:?} value={:?} nonce={:?} batch={}",
                receiver_address,
                balance_to_transfer,
                sender_nonce,
                tx_config.batch_size
            );

            // Generate the transaction, sign it, and push into the transaction
            // pool
            let mut tx_to_insert = Vec::new();
            for i in 0..tx_config.batch_size {
                let tx: Transaction = NativeTransaction {
                    nonce: sender_nonce + i,
                    gas_price: U256::from(1u64),
                    gas: U256::from(21000u64),
                    value: balance_to_transfer,
                    action: Action::Call(receiver_address),
                    storage_limit: 0,
                    chain_id: txgen.consensus.best_chain_id().in_native_space(),
                    epoch_height: txgen.consensus.best_epoch_number(),
                    data: Bytes::new(),
                }
                .into();
                let signed_tx = tx.sign(&address_secret_pair[&sender_address]);
                tx_to_insert.push(signed_tx.transaction);
            }
            let (success_txs, fail) =
                txgen.txpool.insert_new_transactions(tx_to_insert);

            let success_num = success_txs.len();
            tx_n += success_num as u32;
            TX_GEN_METER.mark(success_num);

            if !success_txs.is_empty() {
                txgen.sync.append_received_transactions(success_txs);
            }

            let mut tx_pool_is_full = false;
            let mut not_enough_balance = false;

            for (_, reason) in fail.into_iter() {
                use cfxcore::transaction_pool::TransactionPoolError::*;
                use primitives::transaction::TransactionError::{
                    AlreadyImported, InsufficientBalance, Stale,
                    TooCheapToReplace,
                };
                match reason {
                    OutOfBalance { .. }
                    | TransactionError(InsufficientBalance { .. }) => {
                        not_enough_balance = true;
                    }
                    TxPoolFull => {
                        tx_pool_is_full = true;
                    }
                    NonceTooDistant { .. }
                    | NonceTooStale { .. }
                    | TransactionError(Stale) => {
                        // Improper nonce, retry
                    }
                    HigherGasPriceNeeded { .. }
                    | TransactionError(AlreadyImported | TooCheapToReplace) => {
                        // Transaction for the same nonce has been inserted,
                        // ignore it.
                    }
                    StateDbError(e) => panic!("db error: {:?}", e),
                    err => {
                        TX_GEN_ERROR_METER.mark(1);
                        warn!("tx gen with unexpected error {:?}", err);
                    }
                }
            }

            if tx_pool_is_full {
                thread::sleep(tx_config.period);
            }

            if not_enough_balance {
                addresses.remove(sender_index);
                if addresses.is_empty() {
                    break;
                } else {
                    continue;
                }
            }

            let now = Instant::now();
            let time_elapsed = now.duration_since(start_time);
            if let Some(time_left) =
                (tx_config.period * tx_n).checked_sub(time_elapsed)
            {
                thread::sleep(time_left);
            } else {
                debug!(
                    "Elapsed time larger than the time needed for sleep: \
                     time_elapsed={:?} tx_n={}",
                    time_elapsed, tx_n
                );
            }
        }
    }
}

/// This tx generator directly push simple transactions and erc20 transactions
/// into blocks. It's used in Ethereum e2d replay test.
pub struct DirectTransactionGenerator {
    // Key, simple tx, erc20 balance, array index.
    accounts: HashMap<Address, (KeyPair, Account, U256)>,
    address_by_index: Vec<Address>,
    erc20_address: Address,
}

#[allow(deprecated)]
impl DirectTransactionGenerator {
    const MAX_TOTAL_ACCOUNTS: usize = 100_000;

    pub fn new(
        start_key_pair: KeyPair, contract_creator: &Address,
        start_balance: U256, start_erc20_balance: U256,
    ) -> DirectTransactionGenerator {
        let start_address = public_to_address(start_key_pair.public(), true);
        let info = (
            start_key_pair,
            Account::new_empty_with_balance(
                &start_address.with_native_space(),
                &start_balance,
                &0.into(), /* nonce */
            ),
            start_erc20_balance,
        );
        let mut accounts = HashMap::<Address, (KeyPair, Account, U256)>::new();
        accounts.insert(start_address.clone(), info);
        let address_by_index = vec![start_address.clone()];

        let mut erc20_address = contract_address(
            CreateContractAddress::FromSenderNonceAndCodeHash,
            // A fake block_number. There field is unnecessary in Ethereum
            // replay test.
            0,
            &contract_creator,
            &0.into(),
            // A fake code. There field is unnecessary in Ethereum replay test.
            &[],
        )
        .0;
        erc20_address.set_contract_type_bits();

        debug!(
            "Special Transaction Generator: erc20 contract address: {:?}",
            erc20_address
        );

        DirectTransactionGenerator {
            accounts,
            address_by_index,
            erc20_address,
        }
    }

    pub fn generate_transactions(
        &mut self, block_size_limit: &mut usize, mut num_txs_simple: usize,
        mut num_txs_erc20: usize, chain_id: u32,
    ) -> Vec<Arc<SignedTransaction>> {
        let mut result = vec![];
        // Generate new address with 10% probability
        while num_txs_simple > 0 {
            let number_of_accounts = self.address_by_index.len();

            let sender_index: usize = random_range(0..number_of_accounts);
            let sender_address =
                self.address_by_index.get(sender_index).unwrap().clone();
            let sender_kp;
            let sender_balance;
            let sender_nonce;
            {
                let sender_info = self.accounts.get(&sender_address).unwrap();
                sender_kp = sender_info.0.clone();
                sender_balance = sender_info.1.balance;
                sender_nonce = sender_info.1.nonce;
            }

            let gas = U256::from(100_000u64);
            let gas_price = U256::from(1u64);
            let transaction_fee = U256::from(100_000u64);

            if sender_balance <= transaction_fee {
                self.accounts.remove(&sender_address);
                self.address_by_index.swap_remove(sender_index);
                continue;
            }

            let balance_to_transfer = U256::try_from(
                H512::random().into_uint() % U512::from(sender_balance),
            )
            .unwrap();

            let is_send_to_new_address = (number_of_accounts
                <= Self::MAX_TOTAL_ACCOUNTS)
                && ((number_of_accounts < 10)
                    || (rand::thread_rng().random_range(0..10) == 0));

            let receiver_address = match is_send_to_new_address {
                false => {
                    let index: usize = random_range(0..number_of_accounts);
                    self.address_by_index.get(index).unwrap().clone()
                }
                true => loop {
                    let kp =
                        Random.generate().expect("Fail to generate KeyPair.");
                    let address = public_to_address(kp.public(), true);
                    if self.accounts.get(&address).is_none() {
                        self.accounts.insert(
                            address,
                            (
                                kp,
                                Account::new_empty_with_balance(
                                    &address.with_native_space(),
                                    &0.into(), /* balance */
                                    &0.into(), /* nonce */
                                ),
                                0.into(),
                            ),
                        );
                        self.address_by_index.push(address.clone());

                        break address;
                    }
                },
            };

            let tx: Transaction = NativeTransaction {
                nonce: sender_nonce,
                gas_price,
                gas,
                value: balance_to_transfer,
                action: Action::Call(receiver_address),
                storage_limit: 0,
                // FIXME: We will have to setup TRANSACTION_EPOCH_BOUND to a
                // large value to avoid FIXME: this sloppy zero
                // becomes an issue in the experiments.
                epoch_height: 0,
                chain_id,
                data: vec![0u8; 128],
            }
            .into();
            let signed_transaction = tx.sign(sender_kp.secret());
            let rlp_size = signed_transaction.transaction.rlp_bytes().len();
            if *block_size_limit <= rlp_size {
                break;
            }
            *block_size_limit -= rlp_size;

            self.accounts.get_mut(&sender_address).unwrap().1.balance -=
                balance_to_transfer;
            self.accounts.get_mut(&sender_address).unwrap().1.nonce += 1.into();
            self.accounts.get_mut(&receiver_address).unwrap().1.balance +=
                balance_to_transfer;

            result.push(Arc::new(signed_transaction));

            num_txs_simple -= 1;
        }

        while num_txs_erc20 > 0 {
            let number_of_accounts = self.address_by_index.len();

            let sender_index: usize = random_range(0..number_of_accounts);
            let sender_address =
                self.address_by_index.get(sender_index).unwrap().clone();
            let sender_kp;
            let sender_balance;
            let sender_erc20_balance;
            let sender_nonce;
            {
                let sender_info = self.accounts.get(&sender_address).unwrap();
                sender_kp = sender_info.0.clone();
                sender_balance = sender_info.1.balance;
                sender_erc20_balance = sender_info.2.clone();
                sender_nonce = sender_info.1.nonce;
            }

            let gas = U256::from(100_000u64);
            let gas_price = U256::from(1u64);
            let transaction_fee = U256::from(100_000u64);

            if sender_balance <= transaction_fee {
                self.accounts.remove(&sender_address);
                self.address_by_index.swap_remove(sender_index);
                continue;
            }

            let balance_to_transfer = if sender_erc20_balance == 0.into() {
                continue;
            } else {
                U256::try_from(
                    H512::random().into_uint()
                        % U512::from(sender_erc20_balance),
                )
                .unwrap()
            };

            let receiver_index = random_range(0..number_of_accounts);
            let receiver_address =
                self.address_by_index.get(receiver_index).unwrap().clone();

            if receiver_index == sender_index {
                continue;
            }

            // Calls transfer of ERC20 contract.
            let tx_data = (String::new()
                + "a9059cbb000000000000000000000000"
                + &receiver_address.0.to_hex::<String>()[2..]
                + {
                    let h: H256 =
                        BigEndianHash::from_uint(&balance_to_transfer);
                    &h.0.to_hex::<String>()[2..]
                })
            .from_hex()
            .unwrap();

            let tx: Transaction = NativeTransaction {
                nonce: sender_nonce,
                gas_price,
                gas,
                value: 0.into(),
                action: Action::Call(self.erc20_address.clone()),
                storage_limit: 0,
                // FIXME: We will have to setup TRANSACTION_EPOCH_BOUND to a
                // large value to avoid FIXME: this sloppy zero
                // becomes an issue in the experiments.
                epoch_height: 0,
                chain_id,
                data: tx_data,
            }
            .into();
            let signed_transaction = tx.sign(sender_kp.secret());
            let rlp_size = signed_transaction.transaction.rlp_bytes().len();
            if *block_size_limit <= rlp_size {
                break;
            }
            *block_size_limit -= rlp_size;

            self.accounts.get_mut(&sender_address).unwrap().2 -=
                balance_to_transfer;
            self.accounts.get_mut(&sender_address).unwrap().1.nonce += 1.into();
            self.accounts.get_mut(&receiver_address).unwrap().2 +=
                balance_to_transfer;

            result.push(Arc::new(signed_transaction));

            num_txs_erc20 -= 1;
        }

        result
    }
}
