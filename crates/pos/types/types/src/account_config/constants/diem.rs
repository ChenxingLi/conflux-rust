// Copyright (c) The Diem Core Contributors
// SPDX-License-Identifier: Apache-2.0

// Copyright 2021 Conflux Foundation. All rights reserved.
// Conflux is free software and distributed under GNU General Public License.
// See http://www.gnu.org/licenses/

use anyhow::{bail, Result};
use move_core_types::identifier::Identifier;

/// In addition to the constraints for valid Move identifiers, currency codes
/// should consist entirely of uppercase alphanumeric characters (e.g., no
/// underscores).
pub fn allowed_currency_code_string(
    possible_currency_code_string: &str,
) -> bool {
    possible_currency_code_string
        .chars()
        .all(|chr| matches!(chr, 'A'..='Z' | '0'..='9'))
        && Identifier::is_valid(possible_currency_code_string)
}

pub fn from_currency_code_string(
    currency_code_string: &str,
) -> Result<Identifier> {
    if !allowed_currency_code_string(currency_code_string) {
        bail!("Invalid currency code '{}'", currency_code_string)
    }
    Identifier::new(currency_code_string)
}
