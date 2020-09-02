// Copyright 2020 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

use super::{
    abi::{ABIDecodable, ABIEncodable},
    SolidityFunctionTrait,
};
use crate::{
    state::{State, Substate},
    vm::{self, ActionParams, CallType, GasLeft, ReturnData, Spec},
};
use cfx_types::U256;

/// The standard implementation of `SolidityFunctionTrait`. The
/// `SolidityFunctionTrait` will be implemented automatically if the following
/// traits is implemented:
///
/// - `InterfaceTrait`: Specifies the types of input, output and function
///   interface. It is implemented automatically when constructing a new struct
///   with macro `make_solidity_function`.
///
/// - `PreExecCheckTrait`: Check if the current function call matches some
///   constraints, s.t. no transferred value to a non-payable function. If the
///   internal contract implements `PreExecCheckConfTrait` to specify whether
///   such internal function is payable and whether it should forbid static call
///   (has write operation), `PreExecCheckTrait` will be implemented
///   automatically. `PreExecCheckConfTrait` can be implemented by macro
///  `impl_function_type`.
///
/// - `UpfrontPaymentTrait`: The gas charged before function execution. If the
///   upfront payment is a constant value, it can be implemented by macro
///   `impl_function_type`.
///
/// - `ExecutionTrait`: The execution task for this function.
impl<T> SolidityFunctionTrait for T
where T: InterfaceTrait
        + PreExecCheckTrait
        + UpfrontPaymentTrait
        + ExecutionTrait
{
    fn execute(
        &self, input: &[u8], params: &ActionParams, spec: &Spec,
        state: &mut State, substate: &mut Substate,
    ) -> vm::Result<GasLeft>
    {
        self.pre_execution_check(params)?;
        let solidity_params = <T::Input as ABIDecodable>::abi_decode(&input)?;

        let cost =
            self.upfront_gas_payment(&solidity_params, params, spec, state);
        if cost > params.gas {
            return Err(vm::Error::OutOfGas);
        }

        let output =
            self.execute_inner(solidity_params, params, spec, state, substate)?;

        let encoded_output = output.abi_encode();
        let length = encoded_output.len();
        let return_cost = (length + 31) / 32 * spec.memory_gas;
        if params.gas < cost + return_cost {
            Err(vm::Error::OutOfGas)
        } else {
            Ok(GasLeft::NeedsReturn {
                gas_left: params.gas - cost - return_cost,
                data: ReturnData::new(encoded_output, 0, length),
                apply_state: true,
            })
        }
    }

    fn name(&self) -> &'static str { return Self::NAME_AND_PARAMS; }
}

pub trait InterfaceTrait: Send + Sync {
    type Input: ABIDecodable;
    type Output: ABIEncodable;
    const NAME_AND_PARAMS: &'static str;
}

pub trait PreExecCheckTrait: Send + Sync {
    fn pre_execution_check(&self, params: &ActionParams) -> vm::Result<()>;
}

pub trait ExecutionTrait: Send + Sync + InterfaceTrait {
    fn execute_inner(
        &self, input: Self::Input, params: &ActionParams, spec: &Spec,
        state: &mut State, substate: &mut Substate,
    ) -> vm::Result<<Self as InterfaceTrait>::Output>;
}

pub trait UpfrontPaymentTrait: Send + Sync + InterfaceTrait {
    fn upfront_gas_payment(
        &self, input: &Self::Input, params: &ActionParams, spec: &Spec,
        state: &State,
    ) -> U256;
}

pub trait PreExecCheckConfTrait: Send + Sync {
    const PAYABLE: bool;
    const FORBID_STATIC: bool;
}

impl<T: PreExecCheckConfTrait> PreExecCheckTrait for T {
    fn pre_execution_check(&self, params: &ActionParams) -> vm::Result<()> {
        if !Self::PAYABLE && !params.value.value().is_zero() {
            return Err(vm::Error::InternalContract(
                "should not transfer balance to Staking contract",
            ));
        }
        if Self::FORBID_STATIC && params.call_type == CallType::StaticCall {
            return Err(vm::Error::MutableCallInStaticContext);
        }

        Ok(())
    }
}

#[macro_export]
/// Make a solidity function.
///
/// # Arguments
/// 1. The type of input parameters.
/// 2. The string to compute interface signature.
/// 3. (Optional) The type of output parameters. (If the third parameter is
/// omitted, the return type is regarded as `()`)
///
/// # Example
/// Make a function with interface
/// `get_whitelist(address user, address contract) public returns bool`
/// ```
/// use cfxcore::make_solidity_function;
/// use cfx_types::{Address,U256};
/// use cfxcore::executive::function::InterfaceTrait;
///
/// make_solidity_function!{
///     struct WhateverStructName((Address, Address), "get_whitelist(address,address)", bool);
/// }
/// ```
macro_rules! make_solidity_function {
    ( $(#[$attr:meta])* $visibility:vis struct $name:ident ($input:ty, $interface:expr ); ) => {
        $crate::make_solidity_function! {
            $(#[$attr])* $visibility struct $name ($input, $interface, () );
        }
    };
    ( $(#[$attr:meta])* $visibility:vis struct $name:ident ($input:ty, $interface:expr, $output:ty ); ) => {
        $(#[$attr])*
        #[derive(Copy, Clone)]
        $visibility struct $name;

        impl InterfaceTrait for $name {
            type Input = $input;
            type Output = $output;
            const NAME_AND_PARAMS: &'static str = $interface;
        }
    };
}

#[macro_export]
/// When making a solidity function with macro `make_solidity_function`, the
/// `InterfaceTrait` is implemented automatically. This macro can implement
/// `PreExecCheckTrait` and `UpfrontPaymentTrait`.
///
/// # Arguments
///
/// 1. A string specifies function type. (One of `non_payable_write`,
/// `payable_write`, `query` and `query_with_default_gas`) 2. `gas` - (Optional)
/// Gas required in upfront payment, argument name `gas` is required.
///
/// Specially, if the function type is `query_with_default_gas`, the `gas`
/// parameter should not be provided.
///
/// # Examples
///
/// ```ignore
/// use crate::{
///     evm::{ActionParams, Spec},
///     executive::function::*,
///     impl_function_type, make_solidity_function,
///     state::State,
/// };
/// use cfx_types::{Address, U256};
///
///
/// make_solidity_function! {
///     struct AddPrivilege(Vec<Address>, "addPrivilege(address[])");
/// }
/// impl_function_type!(AddPrivilege, "non_payable_write");
///
///
/// make_solidity_function! {
///     struct IsWhitelisted((Address,Address), "isWhitelisted(address,address)", bool);
/// }
/// impl_function_type!(IsWhitelisted, "query", gas: 200);
/// ```
macro_rules! impl_function_type {
    ( $name:ident, "non_payable_write" $(, gas: $gas:expr)? ) => {
        $crate::impl_function_type!(@inner, $name, false, true $(, $gas)?);
    };
    ( $name:ident, "payable_write" $(, gas: $gas:expr)? ) => {
        $crate::impl_function_type!(@inner, $name, true, true $(, $gas)?);
    };
    ( $name:ident, "query" $(, gas: $gas:expr)? ) => {
        $crate::impl_function_type!(@inner, $name, false, false $(, $gas)?);
    };
    ( @inner, $name:ident, $payable:expr, $forbid_static:expr $(, $gas:expr)? ) => {
        impl PreExecCheckConfTrait for $name {
            const PAYABLE: bool = $payable;
            const FORBID_STATIC: bool = $forbid_static;
        }
        $(
            impl UpfrontPaymentTrait for $name {
                fn upfront_gas_payment(
                    &self, _input: &Self::Input, _params: &ActionParams,_spec: &Spec, _state: &State,
                ) -> U256 {
                    U256::from($gas)
                }
            }
        )?
    };
    ( $name:ident, "query_with_default_gas" ) => {
        impl PreExecCheckConfTrait for $name {
            const PAYABLE: bool = false ;
            const FORBID_STATIC: bool = false;
        }

        impl UpfrontPaymentTrait for $name {
            fn upfront_gas_payment(
                &self, _input: &Self::Input, _params: &ActionParams,spec: &Spec, _state: &State,
            ) -> U256 {
                U256::from(spec.balance_gas)
            }
        }
    };
}
