// Copyright 2021 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

use cfx_parameters::internal_contract_addresses::DA_CONTRACT_ADDRESS;
use cfx_types::{Address, U256};

use super::{super::impls::da::*, preludes::*};

type Bytes = Vec<u8>;

make_solidity_contract! {
    pub struct DataAvail(DA_CONTRACT_ADDRESS, generate_fn_table, initialize: |params: &CommonParams| params.transition_heights.cipda, is_active: |spec: &Spec| spec.cipda);
}

fn generate_fn_table() -> SolFnTable {
    make_function_table!(
        EpochNumber,
        GetAggPkG1,
        GetQuorum,
        GetQuorumRow,
        GetSigner,
        IsSigner,
        QuorumCount,
        RegisteredEpoch,
        RegisterNextEpoch,
        RegisterSigner,
        UpdateSocket
    )
}

group_impl_is_active!(
    |spec: &Spec| spec.cipda,
    EpochNumber,
    GetAggPkG1,
    GetQuorum,
    GetQuorumRow,
    GetSigner,
    IsSigner,
    QuorumCount,
    RegisteredEpoch,
    RegisterNextEpoch,
    RegisterSigner,
    UpdateSocket
);

// Events
make_solidity_event! {
    /// event NewSigner(address indexed signer, BN254.G1Point pkG1, BN254.G2Point pkG2);
    pub struct NewSignerEvent("NewSigner(address,(uint256,uint256),(uint256[2],uint256[2]))", indexed: Address, non_indexed: (G1Point, G2Point));
}

make_solidity_event! {
    /// event SocketUpdated(address indexed signer, string socket);
    pub struct SocketUpdatedEvent("SocketUpdated(address,string)", indexed: Address, non_indexed: String);
}

pub mod events {
    #[allow(unused)]
    pub use super::{NewSignerEvent, SocketUpdatedEvent};
}

// View Functions
make_solidity_function! {
    /// function epochNumber() external view returns (uint);
    pub struct EpochNumber((), "epochNumber()", U256);
}

impl_function_type!(EpochNumber, "query");

impl UpfrontPaymentTrait for EpochNumber {
    fn upfront_gas_payment(
        &self, _input: &(), _params: &ActionParams,
        _context: &InternalRefContext,
    ) -> DbResult<U256> {
        Ok(U256::zero())
    }
}

impl SimpleExecutionTrait for EpochNumber {
    fn execute_inner(
        &self, _input: (), _params: &ActionParams,
        context: &mut InternalRefContext,
    ) -> vm::Result<U256> {
        epoch_number(context)
    }
}

make_solidity_function! {
    /// function getAggPkG1(uint _epoch, uint _quorumId, bytes memory _quorumBitmap) external view returns (BN254.G1Point memory aggPkG1, uint total, uint hit);
    pub struct GetAggPkG1((U256, U256, Bytes), "getAggPkG1(uint256,uint256,bytes)", (G1Point, U256, U256));
}

impl_function_type!(GetAggPkG1, "query");

impl UpfrontPaymentTrait for GetAggPkG1 {
    fn upfront_gas_payment(
        &self, _input: &(U256, U256, Bytes), _params: &ActionParams,
        _context: &InternalRefContext,
    ) -> DbResult<U256> {
        Ok(U256::zero())
    }
}

impl SimpleExecutionTrait for GetAggPkG1 {
    fn execute_inner(
        &self, input: (U256, U256, Bytes), _params: &ActionParams,
        context: &mut InternalRefContext,
    ) -> vm::Result<(G1Point, U256, U256)> {
        get_agg_pk_g1(input, context)
    }
}

make_solidity_function! {
    /// function getQuorum(uint _epoch, uint _quorumId) external view returns (address[] memory);
    pub struct GetQuorum((U256, U256), "getQuorum(uint256,uint256)", Vec<Address>);
}

impl_function_type!(GetQuorum, "query");

impl UpfrontPaymentTrait for GetQuorum {
    fn upfront_gas_payment(
        &self, _input: &(U256, U256), _params: &ActionParams,
        _context: &InternalRefContext,
    ) -> DbResult<U256> {
        Ok(U256::zero())
    }
}

impl SimpleExecutionTrait for GetQuorum {
    fn execute_inner(
        &self, input: (U256, U256), _params: &ActionParams,
        context: &mut InternalRefContext,
    ) -> vm::Result<Vec<Address>> {
        get_quorum(input, context)
    }
}

make_solidity_function! {
    /// function getQuorumRow(uint _epoch, uint _quorumId, uint32 _rowIndex) external view returns (address);
    pub struct GetQuorumRow((U256, U256, u32), "getQuorumRow(uint256,uint256,uint32)", Address);
}

impl_function_type!(GetQuorumRow, "query");

impl UpfrontPaymentTrait for GetQuorumRow {
    fn upfront_gas_payment(
        &self, _input: &(U256, U256, u32), _params: &ActionParams,
        _context: &InternalRefContext,
    ) -> DbResult<U256> {
        Ok(U256::zero())
    }
}

impl SimpleExecutionTrait for GetQuorumRow {
    fn execute_inner(
        &self, input: (U256, U256, u32), _params: &ActionParams,
        context: &mut InternalRefContext,
    ) -> vm::Result<Address> {
        get_quorum_row(input, context)
    }
}

make_solidity_function! {
    /// function getSigner(address[] memory _account) external view returns (SignerDetail[] memory);
    pub struct GetSigner(Vec<Address>, "getSigner(address[])", Vec<SignerDetail>);
}

impl_function_type!(GetSigner, "query");

impl UpfrontPaymentTrait for GetSigner {
    fn upfront_gas_payment(
        &self, _input: &Vec<Address>, _params: &ActionParams,
        _context: &InternalRefContext,
    ) -> DbResult<U256> {
        Ok(U256::zero())
    }
}

impl SimpleExecutionTrait for GetSigner {
    fn execute_inner(
        &self, input: Vec<Address>, _params: &ActionParams,
        context: &mut InternalRefContext,
    ) -> vm::Result<Vec<SignerDetail>> {
        get_signer(input, context)
    }
}

make_solidity_function! {
    /// function isSigner(address _account) external view returns (bool);
    pub struct IsSigner(Address, "isSigner(address)", bool);
}

impl_function_type!(IsSigner, "query");

impl UpfrontPaymentTrait for IsSigner {
    fn upfront_gas_payment(
        &self, _input: &Address, _params: &ActionParams,
        _context: &InternalRefContext,
    ) -> DbResult<U256> {
        Ok(U256::zero())
    }
}

impl SimpleExecutionTrait for IsSigner {
    fn execute_inner(
        &self, input: Address, _params: &ActionParams,
        context: &mut InternalRefContext,
    ) -> vm::Result<bool> {
        is_signer(input, context)
    }
}

make_solidity_function! {
    /// function quorumCount(uint _epoch) external view returns (uint);
    pub struct QuorumCount(U256, "quorumCount(uint256)", U256);
}

impl_function_type!(QuorumCount, "query");

impl UpfrontPaymentTrait for QuorumCount {
    fn upfront_gas_payment(
        &self, _input: &U256, _params: &ActionParams,
        _context: &InternalRefContext,
    ) -> DbResult<U256> {
        Ok(U256::zero())
    }
}

impl SimpleExecutionTrait for QuorumCount {
    fn execute_inner(
        &self, input: U256, _params: &ActionParams,
        context: &mut InternalRefContext,
    ) -> vm::Result<U256> {
        quorum_count(input, context)
    }
}

make_solidity_function! {
    /// function registeredEpoch(address _account, uint _epoch) external view returns (bool);
    pub struct RegisteredEpoch((Address, U256), "registeredEpoch(address,uint256)", bool);
}

impl_function_type!(RegisteredEpoch, "query");

impl UpfrontPaymentTrait for RegisteredEpoch {
    fn upfront_gas_payment(
        &self, _input: &(Address, U256), _params: &ActionParams,
        _context: &InternalRefContext,
    ) -> DbResult<U256> {
        Ok(U256::zero())
    }
}

impl SimpleExecutionTrait for RegisteredEpoch {
    fn execute_inner(
        &self, input: (Address, U256), _params: &ActionParams,
        context: &mut InternalRefContext,
    ) -> vm::Result<bool> {
        registered_epoch(input, context)
    }
}

// State-Changing Functions
make_solidity_function! {
    /// function registerNextEpoch(BN254.G1Point memory _signature) external;
    pub struct RegisterNextEpoch(G1Point, "registerNextEpoch((uint256,uint256))");
}

impl_function_type!(RegisterNextEpoch, "non_payable_write");

impl UpfrontPaymentTrait for RegisterNextEpoch {
    fn upfront_gas_payment(
        &self, _input: &G1Point, _params: &ActionParams,
        _context: &InternalRefContext,
    ) -> DbResult<U256> {
        Ok(U256::zero())
    }
}

impl SimpleExecutionTrait for RegisterNextEpoch {
    fn execute_inner(
        &self, input: G1Point, params: &ActionParams,
        context: &mut InternalRefContext,
    ) -> vm::Result<()> {
        register_next_epoch(input, params, context)
    }
}

make_solidity_function! {
    /// function registerSigner(SignerDetail memory _signer, BN254.G1Point memory _signature) external;
    pub struct RegisterSigner((SignerDetail, G1Point), "registerSigner((address,string,(uint256,uint256),(uint256[2],uint256[2])),(uint256,uint256))");
}

impl_function_type!(RegisterSigner, "non_payable_write");

impl UpfrontPaymentTrait for RegisterSigner {
    fn upfront_gas_payment(
        &self, _input: &(SignerDetail, G1Point), _params: &ActionParams,
        _context: &InternalRefContext,
    ) -> DbResult<U256> {
        Ok(U256::zero())
    }
}

impl SimpleExecutionTrait for RegisterSigner {
    fn execute_inner(
        &self, input: (SignerDetail, G1Point), params: &ActionParams,
        context: &mut InternalRefContext,
    ) -> vm::Result<()> {
        register_signer(input, params, context)
    }
}

make_solidity_function! {
    /// function updateSocket(string memory _socket) external;
    pub struct UpdateSocket(String, "updateSocket(string)");
}

impl_function_type!(UpdateSocket, "non_payable_write");

impl UpfrontPaymentTrait for UpdateSocket {
    fn upfront_gas_payment(
        &self, _input: &String, _params: &ActionParams,
        _context: &InternalRefContext,
    ) -> DbResult<U256> {
        Ok(U256::zero())
    }
}

impl SimpleExecutionTrait for UpdateSocket {
    fn execute_inner(
        &self, input: String, params: &ActionParams,
        context: &mut InternalRefContext,
    ) -> vm::Result<()> {
        update_socket(input, params, context)
    }
}
