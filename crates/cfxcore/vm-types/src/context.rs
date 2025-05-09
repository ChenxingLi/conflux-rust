// Copyright 2015-2018 Parity Technologies (UK) Ltd.
// This file is part of Parity.

// Parity is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Parity is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Parity.  If not, see <http://www.gnu.org/licenses/>.

// Copyright 2019 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

//! Interface for Evm externalities.

use super::{
    call_create_type::CallType,
    env::Env,
    error::{Result, TrapKind},
    return_data::ReturnData,
    spec::Spec,
    Error, InterpreterInfo,
};
use cfx_bytes::Bytes;
use cfx_db_errors::statedb::Result as DbResult;
pub use cfx_types::{
    cal_contract_address as contract_address,
    CreateContractAddressType as CreateContractAddress,
};
use cfx_types::{Address, Space, H256, U256};
use std::sync::Arc;

#[derive(Debug)]
/// Result of externalities create function.
pub enum ContractCreateResult {
    /// Returned when creation was successful.
    /// Contains an address of newly created contract and gas left.
    Created(Address, U256),
    /// Returned when contract creation failed.
    /// Returns the reason so block trace can record it.
    Failed(Error),
    /// Reverted with REVERT.
    Reverted(U256, ReturnData),
}

#[derive(Debug)]
/// Result of externalities call function.
pub enum MessageCallResult {
    /// Returned when message call was successful.
    /// Contains gas left and output data.
    Success(U256, ReturnData),
    /// Returned when message call failed.
    /// Returns the reason so block trace can record it.
    Failed(Error),
    /// Returned when message call was reverted.
    /// Contains gas left and output data.
    Reverted(U256, ReturnData),
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash)]
pub enum BlockHashSource {
    /// Before CIP-133, block hash is read from `Env`, same as the Ethereum
    Env,
    /// After CIP-133, block hash is read from `State`
    State,
}

/// Context for VMs
pub trait Context {
    /// Returns a value for given key.
    fn storage_at(&self, key: &[u8]) -> Result<U256>;

    /// Returns a value for given key.
    fn origin_storage_at(&self, key: &[u8]) -> Result<Option<U256>>;

    /// Stores a value for given key.
    fn set_storage(&mut self, key: Vec<u8>, value: U256) -> Result<()>;

    /// Returns a value for given key.
    fn transient_storage_at(&self, key: &Vec<u8>) -> Result<U256>;

    /// Stores a value for given key.
    fn transient_set_storage(
        &mut self, key: Vec<u8>, value: U256,
    ) -> Result<()>;

    /// Determine whether an account exists.
    fn exists(&self, address: &Address) -> Result<bool>;

    /// Determine whether an account exists and is not null (zero
    /// balance/nonce, no code).
    fn exists_and_not_null(&self, address: &Address) -> Result<bool>;

    /// Balance of the origin account.
    fn origin_balance(&self) -> Result<U256>;

    /// Returns address balance.
    fn balance(&self, address: &Address) -> Result<U256>;

    /// Returns the hash of one of the 256 most recent complete blocks.
    fn blockhash(&mut self, number: &U256) -> Result<H256>;

    /// Creates new contract.
    ///
    /// Returns gas_left and contract address if contract creation was
    /// succesfull.
    fn create(
        &mut self, gas: &U256, value: &U256, code: &[u8],
        address: CreateContractAddress,
    ) -> DbResult<::std::result::Result<ContractCreateResult, TrapKind>>;

    /// Message call.
    ///
    /// Returns Err, if we run out of gas.
    /// Otherwise returns call_result which contains gas left
    /// and true if subcall was successful.
    fn call(
        &mut self, gas: &U256, sender_address: &Address,
        receive_address: &Address, value: Option<U256>, data: &[u8],
        code_address: &Address, call_type: CallType,
    ) -> DbResult<::std::result::Result<MessageCallResult, TrapKind>>;

    /// Returns code at given address
    fn extcode(&self, address: &Address) -> Result<Option<Arc<Bytes>>>;

    /// Returns code hash at given address
    fn extcodehash(&self, address: &Address) -> Result<H256>;

    /// Returns code size at given address
    fn extcodesize(&self, address: &Address) -> Result<usize>;

    /// Creates log entry with given topics and data
    fn log(&mut self, topics: Vec<H256>, data: &[u8]) -> Result<()>;

    fn refund(&mut self, refund_gas: i64);

    /// Should be called when transaction calls `RETURN` opcode.
    /// Returns gas_left if cost of returning the data is not too high.
    fn ret(
        self, gas: &U256, data: &ReturnData, apply_state: bool,
    ) -> Result<U256>;

    /// Should be called when contract commits suicide.
    /// Address to which funds should be refunded.
    fn suicide(&mut self, refund_address: &Address) -> Result<()>;

    /// Returns specification.
    fn spec(&self) -> &Spec;

    /// Returns environment.
    fn env(&self) -> &Env;

    /// Returns the chain ID of the blockchain
    fn chain_id(&self) -> u64;

    /// Returns the space of the blockchain
    fn space(&self) -> Space;

    /// Returns current depth of execution.
    ///
    /// If contract A calls contract B, and contract B calls C,
    /// then A depth is 0, B is 1, C is 2 and so on.
    fn depth(&self) -> usize;

    fn is_warm_account(&self, account: Address) -> bool;

    fn is_warm_storage_entry(&self, key: &H256) -> Result<bool>;

    // /// Decide if any more operations should be traced. Passthrough for the
    // VM /// trace.
    // fn trace_next_instruction(
    //     &mut self, _pc: usize, _instruction: u8, _current_gas: U256,
    // ) -> bool {
    //     false
    // }

    // /// Prepare to trace an operation. Passthrough for the VM trace.
    // fn trace_prepare_execute(
    //     &mut self, _pc: usize, _instruction: u8, _gas_cost: U256,
    //     _mem_written: Option<(usize, usize)>,
    //     _store_written: Option<(U256, U256)>,
    // ) {
    // }

    // /// Trace the finalised execution of a single instruction.
    // fn trace_executed(
    //     &mut self, _gas_used: U256, _stack_push: &[U256], _mem: &[u8],
    // ) {
    // }

    fn trace_step(&mut self, interpreter: &dyn InterpreterInfo) {
        let _ = interpreter;
    }

    fn trace_step_end(&mut self, interpreter: &dyn InterpreterInfo) {
        let _ = interpreter;
    }

    fn opcode_trace_enabled(&self) -> bool { false }

    /// Check if running in static context.
    fn is_static(&self) -> bool;

    /// Check if running in static context or reentrancy context
    fn is_static_or_reentrancy(&self) -> bool;

    fn blockhash_source(&self) -> BlockHashSource;
}
