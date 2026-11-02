//! Pure diagnostics over stored USD aggregates, not deployed liquidation verification.

use anyhow::{Context, Result, ensure};
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::Value;

use crate::model::{Object, array, string};

#[derive(Debug, Serialize)]
pub struct Diagnostics {
    pub basis: &'static str,
    pub collateral_usd: String,
    pub debt_usd: String,
    pub weighted_debt_usd: String,
    pub liquidation_capacity_usd: String,
    pub ltv: Option<String>,
    pub effective_close_ltv: Option<String>,
    pub health_factor: Option<String>,
    pub liquidation_buffer_usd: String,
    pub health_bucket: &'static str,
}

/// Parse a Move 1e18-scaled USD value without first fitting its raw u256 in Decimal.
/// Reject unsupported precision/range instead of rounding or converting to float.
pub fn usd(value: &Value) -> Result<Decimal> {
    let digits = string(value, "value")?;
    ensure!(
        !digits.is_empty() && digits.len() <= 78 && digits.bytes().all(|c| c.is_ascii_digit()),
        "invalid unsigned fixed-point decimal"
    );
    let padded = format!("{digits:0>19}");
    let split = padded.len() - 18;
    let whole = padded[..split].trim_start_matches('0');
    let whole = if whole.is_empty() { "0" } else { whole };
    let fraction = padded[split..].trim_end_matches('0');
    let text = if fraction.is_empty() {
        whole.to_owned()
    } else {
        format!("{whole}.{fraction}")
    };
    Decimal::from_str_exact(&text).context("USD value exceeds exact diagnostic decimal range")
}

fn ratio(numerator: Decimal, denominator: Decimal) -> Result<Option<String>> {
    if denominator.is_zero() {
        return Ok(None);
    }
    Ok(Some(
        numerator
            .checked_div(denominator)
            .context("ratio overflow")?
            .normalize()
            .to_string(),
    ))
}

pub fn assess(object: &Object) -> Result<Diagnostics> {
    let value = &object.contents;
    let collateral = usd(&value["deposited_value_usd"])?;
    let debt = usd(&value["unweighted_borrowed_value_usd"])?;
    let weighted = usd(&value["weighted_borrowed_value_usd"])?;
    let capacity = usd(&value["unhealthy_borrow_value_usd"])?;
    // Presence of a borrow with zero cached debt does not prove a debt-free account.
    let has_borrows = !array(value, "borrows")?.is_empty();
    let bucket = if weighted.is_zero() {
        if has_borrows || !debt.is_zero() {
            "unknown_zero_cached_debt"
        } else {
            "no_debt"
        }
    } else if weighted > capacity {
        "below_1"
    } else if capacity
        < weighted
            .checked_mul(Decimal::new(105, 2))
            .context("bucket overflow")?
    {
        "1_to_1_05"
    } else if capacity
        < weighted
            .checked_mul(Decimal::new(110, 2))
            .context("bucket overflow")?
    {
        "1_05_to_1_1"
    } else if capacity
        < weighted
            .checked_mul(Decimal::new(125, 2))
            .context("bucket overflow")?
    {
        "1_1_to_1_25"
    } else {
        "at_least_1_25"
    };
    Ok(Diagnostics {
        basis: "cached_obligation_valuations_not_refreshed_at_checkpoint",
        collateral_usd: collateral.normalize().to_string(),
        debt_usd: debt.normalize().to_string(),
        weighted_debt_usd: weighted.normalize().to_string(),
        liquidation_capacity_usd: capacity.normalize().to_string(),
        ltv: ratio(debt, collateral)?,
        effective_close_ltv: ratio(capacity, collateral)?,
        health_factor: ratio(capacity, weighted)?,
        liquidation_buffer_usd: capacity
            .checked_sub(weighted)
            .context("buffer overflow")?
            .normalize()
            .to_string(),
        health_bucket: bucket,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn position(collateral: &str, debt: &str, weighted: &str, capacity: &str) -> Object {
        let scaled = |s: &str| json!({"value": format!("{s}000000000000000000")});
        Object {
            address: "test".into(),
            version: 1,
            digest: "test".into(),
            move_type: "test".into(),
            owner: None,
            contents: json!({"deposited_value_usd":scaled(collateral), "unweighted_borrowed_value_usd":scaled(debt),
                "weighted_borrowed_value_usd":scaled(weighted), "unhealthy_borrow_value_usd":scaled(capacity),
                "borrows": if debt == "0" {vec![]} else {vec![json!({})]}}),
        }
    }

    #[test]
    fn distinguishes_actual_ltv_and_weighted_health() {
        let result = assess(&position("100", "60", "72", "75")).unwrap();
        assert_eq!(result.ltv.as_deref(), Some("0.6"));
        assert_eq!(result.effective_close_ltv.as_deref(), Some("0.75"));
        assert_eq!(result.liquidation_buffer_usd, "3");
        assert_eq!(result.health_bucket, "1_to_1_05");
    }

    #[test]
    fn preserves_strict_liquidation_boundary_and_zero_debt() {
        assert_eq!(
            assess(&position("100", "75", "75", "75"))
                .unwrap()
                .health_bucket,
            "1_to_1_05"
        );
        assert_eq!(
            assess(&position("100", "76", "76", "75"))
                .unwrap()
                .health_bucket,
            "below_1"
        );
        let result = assess(&position("0", "0", "0", "0")).unwrap();
        assert_eq!(result.health_bucket, "no_debt");
        assert!(result.ltv.is_none() && result.health_factor.is_none());
        let mut cached_zero = position("100", "0", "0", "75");
        cached_zero.contents["borrows"] = json!([{}]);
        assert_eq!(
            assess(&cached_zero).unwrap().health_bucket,
            "unknown_zero_cached_debt"
        );
    }

    #[test]
    fn parses_scaled_values_without_raw_96_bit_overflow() {
        assert_eq!(
            usd(&json!({"value":"1000000000000000000000000000000"}))
                .unwrap()
                .to_string(),
            "1000000000000"
        );
        assert_eq!(usd(&json!({"value":"1"})).unwrap(), Decimal::new(1, 18));
        for raw in [
            "-1",
            "1.2",
            "1e18",
            "",
            "115792089237316195423570985008687907853269984665640564039457584007913129639935",
        ] {
            assert!(usd(&json!({"value":raw})).is_err());
        }
    }
}
