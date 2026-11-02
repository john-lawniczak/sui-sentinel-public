//! Deployment identity shared by local validation and checkpoint capture.

use std::{fs, path::Path};

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

pub const MAINNET_CHAIN: &str = "4btiuiMPvEENsttpZC7CZ53DruC3MAgfznDbASZ7DR6S";

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub suilend: SuilendConfig,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SuilendConfig {
    pub network: Network,
    pub original_package_id: String,
    pub published_package_id: String,
    pub upgrade_cap_id: String,
    pub package_version: u64,
    pub market_version: u64,
    pub lending_market_id: String,
    pub lending_market_type: String,
}

/// The initial monitor targets mainnet only.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    Mainnet,
}

impl Config {
    /// Read, parse, and validate a local file. This performs no chain queries.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let input = fs::read_to_string(path)
            .with_context(|| format!("failed to read configuration {}", path.display()))?;
        Self::parse(&input).with_context(|| format!("invalid configuration {}", path.display()))
    }

    /// Reject missing/unknown fields and malformed deployment identifiers.
    pub fn parse(input: &str) -> Result<Self> {
        let config: Self = toml::from_str(input).context("failed to parse configuration TOML")?;
        config.validate()?;
        Ok(config)
    }

    /// Validate local consistency, not deployment identity or source equivalence.
    pub fn validate(&self) -> Result<()> {
        let suilend = &self.suilend;
        for (field, value) in [
            ("original_package_id", &suilend.original_package_id),
            ("published_package_id", &suilend.published_package_id),
            ("upgrade_cap_id", &suilend.upgrade_cap_id),
            ("lending_market_id", &suilend.lending_market_id),
        ] {
            ensure!(
                is_full_address(value),
                "suilend.{field} must be 0x followed by exactly 64 hexadecimal digits"
            );
        }
        ensure!(
            suilend.package_version > 0,
            "suilend.package_version must be positive"
        );

        let parts: Vec<_> = suilend.lending_market_type.split("::").collect();
        ensure!(
            parts.len() == 3
                && parts[0].eq_ignore_ascii_case(&suilend.original_package_id)
                && is_identifier(parts[1])
                && is_identifier(parts[2]),
            "suilend.lending_market_type must be original_package_id::module::Type"
        );
        Ok(())
    }
}

pub(crate) fn is_full_address(value: &str) -> bool {
    value
        .strip_prefix("0x")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn is_identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = include_str!("../deploy/config/suilend.toml");

    #[test]
    fn accepts_repository_configuration() {
        Config::parse(CONFIG).unwrap();
    }

    #[test]
    fn rejects_schema_errors() {
        for input in [
            CONFIG.replace("network = \"mainnet\"", "network = \"testnet\""),
            CONFIG.replace("network = \"mainnet\"", ""),
            CONFIG.replace("package_version = 25", "package_version = -1"),
            format!("{CONFIG}\nunknown = true\n"),
            format!("unknown = true\n{CONFIG}"),
            "[suilend".to_owned(),
        ] {
            assert!(Config::parse(&input).is_err());
        }
    }

    #[test]
    fn rejects_invalid_addresses_for_each_field() {
        let config = Config::parse(CONFIG).unwrap();
        for original in [
            config.suilend.original_package_id,
            config.suilend.published_package_id,
            config.suilend.upgrade_cap_id,
            config.suilend.lending_market_id,
        ] {
            for invalid in [
                "0x1".to_owned(),
                format!("0x{}", "g".repeat(64)),
                "".to_owned(),
            ] {
                assert!(Config::parse(&CONFIG.replace(&original, &invalid)).is_err());
            }
        }
    }

    #[test]
    fn rejects_zero_package_version() {
        assert!(
            Config::parse(&CONFIG.replace("package_version = 25", "package_version = 0")).is_err()
        );
    }

    #[test]
    fn rejects_mismatched_or_malformed_market_type() {
        for suffix in [
            "",
            "::suilend",
            "::suilend::",
            "::9module::MAIN_POOL",
            "::suilend::MAIN_POOL::extra",
        ] {
            let mut config = Config::parse(CONFIG).unwrap();
            config.suilend.lending_market_type =
                format!("{}{suffix}", config.suilend.original_package_id);
            assert!(config.validate().is_err());
        }
        let mut config = Config::parse(CONFIG).unwrap();
        config.suilend.lending_market_type = format!(
            "{}::suilend::MAIN_POOL",
            config.suilend.published_package_id
        );
        assert!(config.validate().is_err());
    }
}
