use cfx_bytes::Bytes;
use cfx_types::{Address, U256};
use cfx_vm_types::ActionParams;
use solidity_abi_derive::ABIVariable;

use crate::internal_contract::InternalRefContext;
use cfx_vm_types as vm;

#[derive(Debug, ABIVariable)]
pub struct G1Point(U256, U256);

#[derive(Debug, ABIVariable)]
pub struct G2Point([U256; 2], [U256; 2]);

#[derive(Debug, ABIVariable)]
pub struct SignerDetail(Address, String, G1Point, G2Point);

pub fn epoch_number(_context: &mut InternalRefContext) -> vm::Result<U256> {
    todo!()
}

pub fn get_agg_pk_g1(
    _input: (U256, U256, Bytes), _context: &mut InternalRefContext,
) -> vm::Result<(G1Point, U256, U256)> {
    todo!()
}

pub fn get_quorum(
    _input: (U256, U256), _context: &mut InternalRefContext,
) -> vm::Result<Vec<Address>> {
    todo!()
}

pub fn get_quorum_row(
    _input: (U256, U256, u32), _context: &mut InternalRefContext,
) -> vm::Result<Address> {
    todo!()
}

pub fn get_signer(
    _input: Vec<Address>, _context: &mut InternalRefContext,
) -> vm::Result<Vec<SignerDetail>> {
    todo!()
}

pub fn is_signer(
    _input: Address, _context: &mut InternalRefContext,
) -> vm::Result<bool> {
    todo!()
}

pub fn quorum_count(
    _input: U256, _context: &mut InternalRefContext,
) -> vm::Result<U256> {
    todo!()
}

pub fn registered_epoch(
    _input: (Address, U256), _context: &mut InternalRefContext,
) -> vm::Result<bool> {
    todo!()
}

pub fn register_next_epoch(
    _input: G1Point, _params: &ActionParams, _context: &mut InternalRefContext,
) -> vm::Result<()> {
    todo!()
}

pub fn register_signer(
    _input: (SignerDetail, G1Point), _params: &ActionParams,
    _context: &mut InternalRefContext,
) -> vm::Result<()> {
    todo!()
}

pub fn update_socket(
    _input: String, _params: &ActionParams, _context: &mut InternalRefContext,
) -> vm::Result<()> {
    todo!()
}
