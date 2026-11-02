//! Exact decimal strings shared by report artifacts and calculation policies.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DecimalString(#[serde(with = "rust_decimal::serde::str")] pub Decimal);
