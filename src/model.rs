//! Lossless chain observations. Stored valuations are not fresh risk calculations.

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Checkpoint {
    #[serde(rename = "sequenceNumber")]
    pub sequence_number: u64,
    pub timestamp: String,
    pub digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Object {
    pub address: String,
    pub version: u64,
    pub digest: String,
    pub move_type: String,
    pub contents: Value,
    pub owner: Option<Owner>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Owner {
    pub kind: String,
    /// Immediate owner only. An ObjectOwner address is not a wallet address.
    pub address: Option<String>,
}

impl Object {
    pub fn from_graphql(mut value: Value, wrapped: bool) -> Result<Self> {
        let mut contents = if wrapped {
            value
                .get_mut("asMoveObject")
                .and_then(|v| v.get_mut("contents"))
        } else {
            value.get_mut("contents")
        }
        .context("missing Move contents")?
        .take();
        let object = Self {
            address: string(&value, "address")?.to_owned(),
            version: integer(&value, "version")?,
            digest: string(&value, "digest")?.to_owned(),
            move_type: string(&contents["type"], "repr")?.to_owned(),
            contents: contents
                .get_mut("json")
                .context("missing Move JSON")?
                .take(),
            owner: value
                .get("owner")
                .filter(|v| !v.is_null())
                .map(|v| {
                    Ok::<_, anyhow::Error>(Owner {
                        kind: string(v, "__typename")?.to_owned(),
                        address: v["address"]["address"].as_str().map(str::to_owned),
                    })
                })
                .transpose()?,
        };
        object.validate()?;
        Ok(object)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            crate::config::is_full_address(&self.address),
            "invalid object address"
        );
        ensure!(
            self.version > 0 && !self.digest.is_empty(),
            "missing object provenance"
        );
        ensure!(self.contents.is_object(), "missing Move object contents");
        ensure!(
            string(&self.contents, "id")? == self.address,
            "object ID mismatch"
        );
        if let Some(address) = self.owner.as_ref().and_then(|o| o.address.as_ref()) {
            ensure!(
                crate::config::is_full_address(address),
                "invalid owner address"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Market {
    pub object: Object,
    pub type_argument: String,
    pub table_id: String,
    pub expected_obligations: u64,
}

impl Market {
    pub fn new(object: Object, package: &str) -> Result<Self> {
        object.validate()?;
        let type_argument =
            crate::protocols::suilend::market_type_argument(&object.move_type, package)
                .context("unexpected market type")?
                .to_owned();
        let table = &object.contents["obligations"];
        let table_id = string(table, "id")?.to_owned();
        let expected_obligations = integer(table, "size")?;
        ensure!(
            crate::config::is_full_address(&table_id),
            "invalid table ID"
        );
        for (index, reserve) in array(&object.contents, "reserves")?.iter().enumerate() {
            ensure!(
                integer(reserve, "array_index")? == index as u64,
                "reserve order mismatch"
            );
            ensure!(
                string(reserve, "lending_market_id")? == object.address,
                "reserve market mismatch"
            );
        }
        Ok(Self {
            object,
            type_argument,
            table_id,
            expected_obligations,
        })
    }

    pub fn obligation_type(&self, package: &str) -> String {
        crate::protocols::suilend::obligation_type(package, &self.type_argument)
    }

    pub fn cap_type(&self, package: &str) -> String {
        crate::protocols::suilend::cap_type(package, &self.type_argument)
    }

    pub fn validate_obligation(&self, object: &Object, package: &str) -> Result<()> {
        crate::protocols::suilend::validate_position(
            object,
            &self.object,
            package,
            &self.type_argument,
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capture {
    pub market: Market,
    pub obligations: u64,
    pub obligations_exhausted: bool,
    pub owner_caps: u64,
    pub owner_caps_exhausted: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub schema_version: u32,
    pub chain_identifier: String,
    pub checkpoint: Checkpoint,
    pub original_package_id: String,
    pub upgrade_cap: Object,
    pub clock: Object,
    pub scope: String,
    pub complete: bool,
    pub markets: Vec<Capture>,
}

pub fn string<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .with_context(|| format!("missing string {field}"))
}

pub fn integer(value: &Value, field: &str) -> Result<u64> {
    let value = value
        .get(field)
        .with_context(|| format!("missing integer {field}"))?;
    if let Some(n) = value.as_u64() {
        return Ok(n);
    }
    value
        .as_str()
        .context("expected unsigned integer string")?
        .parse()
        .with_context(|| format!("invalid integer {field}"))
}

pub fn array<'a>(value: &'a Value, field: &str) -> Result<&'a [Value]> {
    value
        .get(field)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .with_context(|| format!("missing array {field}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn full_address(byte: &str) -> String {
        format!("0x{}", byte.repeat(64))
    }

    fn market_object(move_type: String) -> Object {
        let address = full_address("1");
        Object {
            address: address.clone(),
            version: 1,
            digest: "deadbeef".to_owned(),
            move_type,
            contents: json!({
                "id": address,
                "obligations": {"id": full_address("2"), "size": 0},
                "reserves": [],
            }),
            owner: None,
        }
    }

    /// S01's adapter extraction must not change this behavior: a market
    /// object from a package other than the one the caller configured is
    /// rejected, not silently accepted because the rest of its shape looks
    /// familiar (temp.md section 2's "do not silently accept a new
    /// package because its fields look familiar").
    #[test]
    fn market_new_rejects_an_object_from_an_unsupported_or_different_package() {
        let configured_package = full_address("a");
        let deployed_package = full_address("b");
        let object = market_object(format!(
            "{deployed_package}::lending_market::LendingMarket<{}::pool::MAIN>",
            full_address("c")
        ));
        let error = Market::new(object, &configured_package).unwrap_err();
        assert!(error.to_string().contains("unexpected market type"));
    }

    #[test]
    fn market_new_accepts_a_matching_package_and_extracts_its_type_argument() {
        let package = full_address("a");
        let type_argument = format!("{}::pool::MAIN", full_address("c"));
        let object = market_object(format!(
            "{package}::lending_market::LendingMarket<{type_argument}>"
        ));
        let market = Market::new(object, &package).unwrap();
        assert_eq!(market.type_argument, type_argument);
    }
}
