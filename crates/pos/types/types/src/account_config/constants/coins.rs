// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

// Copyright 2021 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

use crate::account_config::constants::{
    from_currency_code_string, CORE_CODE_ADDRESS,
};
use move_core_types::language_storage::{StructTag, TypeTag};

pub const XDX_NAME: &str = "XDX";
pub const XUS_NAME: &str = "XUS";

pub fn xus_tag() -> TypeTag {
    TypeTag::Struct(StructTag {
        address: CORE_CODE_ADDRESS,
        module: from_currency_code_string(XUS_NAME).unwrap(),
        name: from_currency_code_string(XUS_NAME).unwrap(),
        type_params: vec![],
    })
}

pub fn xdx_type_tag() -> TypeTag {
    TypeTag::Struct(StructTag {
        address: CORE_CODE_ADDRESS,
        module: from_currency_code_string(XDX_NAME).unwrap(),
        name: from_currency_code_string(XDX_NAME).unwrap(),
        type_params: vec![],
    })
}
