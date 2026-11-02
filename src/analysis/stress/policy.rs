use std::{
    collections::{BTreeMap, HashSet},
    path::Path,
};

use anyhow::{Result, ensure};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub schema_version: u32,
    #[serde(with = "rust_decimal::serde::str")]
    pub loss_tolerance: Decimal,
    pub scenarios: Vec<Scenario>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenario {
    pub name: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub default_price_factor: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub recovery_discount: Decimal,
    #[serde(default)]
    pub price_factors: BTreeMap<String, crate::decimal::DecimalString>,
}

impl Policy {
    pub fn builtin() -> Result<Self> {
        Self::parse(include_str!("../../../deploy/config/stress.toml"))
    }

    pub fn load(path: &Path) -> Result<Self> {
        Self::parse(&std::fs::read_to_string(path)?)
    }

    pub fn parse(text: &str) -> Result<Self> {
        let policy: Self = toml::from_str(text)?;
        policy.validate()?;
        Ok(policy)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(self.schema_version == 1, "unsupported stress policy schema");
        ensure!(
            self.loss_tolerance > Decimal::ZERO && self.loss_tolerance <= Decimal::ONE,
            "loss tolerance must be in (0, 1]"
        );
        ensure!(
            !self.scenarios.is_empty() && self.scenarios.len() <= 16,
            "expected 1..=16 scenarios"
        );
        let mut names = HashSet::new();
        for scenario in &self.scenarios {
            ensure!(
                !scenario.name.is_empty()
                    && scenario.name.len() <= 64
                    && scenario
                        .name
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'_')
                    && names.insert(&scenario.name),
                "invalid or duplicate scenario name"
            );
            ensure!(
                (Decimal::ZERO..=Decimal::ONE).contains(&scenario.recovery_discount),
                "recovery discount must be in [0, 1]"
            );
            ensure!(
                scenario.price_factors.len() <= 128,
                "too many price overrides"
            );
            for (asset, factor) in &scenario.price_factors {
                ensure!(
                    valid_asset(asset),
                    "price overrides require exact canonical coin types"
                );
                validate_factor(factor.0)?;
            }
            validate_factor(scenario.default_price_factor)?;
        }
        Ok(())
    }
}

fn validate_factor(value: Decimal) -> Result<()> {
    ensure!(
        value >= Decimal::ZERO && value <= Decimal::from(10),
        "price factor must be in [0, 10]"
    );
    Ok(())
}

fn valid_asset(asset: &str) -> bool {
    let Some((package, rest)) = asset.split_once("::") else {
        return false;
    };
    package.len() == 64
        && package
            .bytes()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        && !rest.is_empty()
        && asset.len() <= 1024
        && !asset.bytes().any(|c| c.is_ascii_whitespace())
}
