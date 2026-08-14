//! The genesis block of every network configuration, pinned by value.
//!
//! Every expected value below was recorded on master (commit 3790f62a7b) by
//! running this file in a checkout of master. A failure means the refactor
//! moved a live network's genesis, which forks that network: it needs a human
//! decision, not a new expected value.
//!
//! The cases are the configurations which produce different genesis blocks.
//! Test or dev mode and whether the genesis transactions are executed are the
//! two inputs that move the state root, and they move it independently, so all
//! four combinations appear; `chain_id` moves the block hash alone. The test
//! goes through `Configuration` rather than calling `genesis_block` directly
//! because it is that step which decides the internal contracts and the CIPs
//! the genesis state is built with.

use std::{fs, path::PathBuf, sync::Arc};

use cfx_config::Configuration;
use cfx_executor::machine::{Machine, VmFactory};
use cfx_parameters::genesis::GENESIS_ACCOUNT_ADDRESS;
use cfx_storage::StorageManager;
use cfx_types::{H256, U256};
use cfxcore::{
    genesis_block::{default as genesis_default, genesis_block},
    NodeType,
};
use primitives::block_header::CIP112_TRANSITION_HEIGHT;

/// One network configuration, and the genesis block it has to produce.
struct Case {
    name: &'static str,
    conf: Configuration,
    node_type: NodeType,
    expected_hash: &'static str,
    expected_state_root: &'static str,
}

/// Conflux mainnet, as configured by `run/hydra.toml`.
fn mainnet() -> Configuration {
    let mut conf = Configuration::default();
    conf.raw_conf.chain_id = Some(1029);
    conf.raw_conf.evm_chain_id = Some(1030);
    // Any value above zero produces the same genesis: it only decides whether
    // the client loads the PoS initial nodes, and PoS was enabled on both live
    // networks long after their genesis. The mainnet value is used verbatim.
    conf.raw_conf.pos_reference_enable_height = 37_400_000;
    conf
}

/// Conflux testnet: same production parameters, its own chain ids.
fn testnet() -> Configuration {
    let mut conf = mainnet();
    conf.raw_conf.chain_id = Some(1);
    conf.raw_conf.evm_chain_id = Some(71);
    conf
}

/// A single node development chain, `mode = "dev"`.
///
/// `pos_reference_enable_height` has to be set to something finite: dev mode
/// puts the CIP-1559 transition at height one, and `common_params` refuses a
/// PoS transition later than that. One is the smallest value that keeps the
/// PoS initial nodes out, which is what this case wants.
fn dev() -> Configuration {
    let mut conf = Configuration::default();
    conf.raw_conf.mode = Some("dev".into());
    conf.raw_conf.pos_reference_enable_height = 1;
    conf
}

/// The integration test suite, transcribed from `small_local_test_conf` in
/// `tests/conflux/config.py`, keeping only the keys that reach the genesis
/// construction.
///
/// `pos_reference_enable_height = 0` means the real suite hands
/// `genesis_block` a set of PoS initial nodes; this case passes none. The
/// documentation of `build` says why a genesis carrying initial nodes cannot
/// be pinned by value at all.
fn local_test() -> Configuration {
    let mut conf = Configuration::default();
    conf.raw_conf.mode = Some("test".into());
    conf.raw_conf.chain_id = Some(10);
    conf.raw_conf.execute_genesis = false;
    conf.raw_conf.pos_reference_enable_height = 0;
    conf.raw_conf.hydra_transition_height = Some(0);
    conf.raw_conf.hydra_transition_number = Some(0);
    conf.raw_conf.cip43_init_end_number = Some(u32::MAX as u64);
    conf.raw_conf.dao_vote_transition_number = Some(1 << 31);
    conf.raw_conf.dao_vote_transition_height = Some(1 << 31);
    conf.raw_conf.cip172_transition_height = Some(1 << 31);
    // Only an archive node honours this flag, hence `NodeType::Archive` for
    // this case: it is the one configuration that also builds the single MPT
    // replica while committing the genesis.
    conf.raw_conf.enable_single_mpt_storage = true;
    conf
}

fn cases() -> Vec<Case> {
    let mut mainnet_no_execute = mainnet();
    mainnet_no_execute.raw_conf.execute_genesis = false;

    vec![
        Case {
            name: "mainnet",
            conf: mainnet(),
            node_type: NodeType::Full,
            expected_hash:
                "0xf6cec5af1ee95f56038fc30589bcfeb4b960075c3c76b8a0eaa6d36e8e623b50",
            expected_state_root:
                "0x8f0b97e63fc7c697347c85f7ad97dbf0301604573dbd0e4ebfebcdd1ceba83c6",
        },
        Case {
            name: "testnet",
            conf: testnet(),
            node_type: NodeType::Full,
            expected_hash:
                "0x71367f36b8799418feea0d8ec635e016e22c33a27e0e01c7e166d587601a32db",
            // Same state root as mainnet: the chain id only reaches the
            // genesis transactions, hence the transactions root and the block
            // hash, never the state.
            expected_state_root:
                "0x8f0b97e63fc7c697347c85f7ad97dbf0301604573dbd0e4ebfebcdd1ceba83c6",
        },
        Case {
            name: "mainnet_without_genesis_execution",
            conf: mainnet_no_execute,
            node_type: NodeType::Full,
            expected_hash:
                "0x5b05c329a2075b3e423034f03cc1c6cfeb1c950c8af0c568e510b81d67e01318",
            expected_state_root:
                "0x3eb0fed612753120edabb0249df8d8e2eecc9d0e0e7f0284f824ef6c2f055c7d",
        },
        Case {
            name: "dev",
            conf: dev(),
            node_type: NodeType::Full,
            expected_hash:
                "0x263ff7e4091a3c7cdb4abe89b3086ae7af83cf828dccc2159216954d84956019",
            expected_state_root:
                "0xb36be8cc045c8b27b8457328f6e6214327f799635d1345664b9797e7033fbab7",
        },
        Case {
            name: "local_test",
            conf: local_test(),
            node_type: NodeType::Archive,
            expected_hash:
                "0xc065cee41743d1717ce202c22e5821ffe558b461e39468b24e9b8a2c7f5e7252",
            expected_state_root:
                "0xb03c12753d2c8488e10360bdac99fd38c08a3e43a807144b9a80ce3cc1e1ebbb",
        },
    ]
}

/// Construct the genesis block of one configuration and answer with its hash
/// and its deferred state root.
///
/// The PoS initial nodes are always absent. They are the one genesis input
/// that cannot be pinned: `genesis_block` executes each node's registration
/// transaction, and `register_transaction` builds that transaction's calldata
/// around a sigma protocol proof drawn from `OsRng`, so the transaction, and
/// with it the resulting state, differs on every construction. Three
/// constructions from one fixed key set were measured to give three different
/// genesis roots. The integration suite generates its PoS keys per run, so its
/// genesis hash is not a constant either.
fn build(case: &Case) -> (H256, H256) {
    let dir = scratch_dir(case.name);
    let mut conf = case.conf.clone();
    conf.raw_conf.conflux_data_dir = dir.to_str().unwrap().to_string();

    let storage_manager = Arc::new(
        StorageManager::new(conf.storage_config(&case.node_type))
            .expect("failed to initialize storage"),
    );
    let machine = Arc::new(Machine::new_with_builtin(
        conf.common_params(),
        VmFactory::new(1024 * 32),
    ));
    let block = genesis_block(
        &storage_manager,
        genesis_default(conf.is_test_or_dev_mode()),
        GENESIS_ACCOUNT_ADDRESS,
        U256::zero(),
        machine,
        conf.raw_conf.execute_genesis,
        conf.raw_conf.chain_id,
        &None,
    );

    let answer = (block.hash(), *block.block_header.deferred_state_root());
    drop(storage_manager);
    let _ = fs::remove_dir_all(&dir);
    answer
}

fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "cfx-genesis-root-test-{}-{}",
        std::process::id(),
        name
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("failed to create the data directory");
    dir
}

#[test]
fn genesis_block_of_every_network_configuration() {
    // `Configuration::parse` sets this process global, and nothing else does.
    // Genesis sits at height zero, so any transition height above zero leaves
    // the block header encoding alone; every deployed value is above zero.
    CIP112_TRANSITION_HEIGHT.get_or_init(|| u64::MAX);

    let mut mismatches = Vec::new();
    for case in cases() {
        let (hash, state_root) = build(&case);
        let expected_hash: H256 = case.expected_hash.parse().unwrap();
        let expected_state_root: H256 =
            case.expected_state_root.parse().unwrap();
        if hash != expected_hash || state_root != expected_state_root {
            mismatches.push(format!(
                "{}:\n  block hash          {:?}\n  expected            \
                 {:?}\n  deferred state root {:?}\n  expected            {:?}",
                case.name, hash, expected_hash, state_root, expected_state_root,
            ));
        }
    }

    assert!(
        mismatches.is_empty(),
        "the genesis block of a network configuration changed, which forks \
         that network:\n{}",
        mismatches.join("\n"),
    );
}
