//! Suilend cached observation adapter. Fresh valuation remains unsupported.
use super::{
    adapter::{Adapter, Projection},
    suilend,
};
use crate::{
    analysis::risk,
    config::Config,
    model::{Checkpoint, Market, Object, array, integer, string},
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub struct Suilend {
    pub config: Config,
}
impl Adapter for Suilend {
    fn id(&self) -> &'static str {
        "suilend.cached.v1"
    }
    fn configuration_hash(&self) -> Result<String> {
        Ok(crate::storage::sha256_hex_of(&serde_json::to_vec(
            &self.config,
        )?))
    }
    fn project(&self, object: &Object) -> Result<Projection> {
        object.validate()?;
        let c = &self.config.suilend;
        let mut projection = Projection {
            kind: "dependency",
            dependencies: vec![],
            invalidates: vec![format!("object:{}", object.address)],
        };
        if object.address == c.upgrade_cap_id {
            ensure!(
                integer(&object.contents, "version")? == c.package_version
                    && string(&object.contents, "package")? == c.published_package_id,
                "unsupported deployment upgrade; review adapter before resuming"
            );
        } else if object.address
            == "0x0000000000000000000000000000000000000000000000000000000000000006"
        {
            integer(&object.contents, "timestamp_ms")?;
        } else if let Some(argument) =
            suilend::market_type_argument(&object.move_type, &c.original_package_id)
        {
            ensure!(
                argument == c.lending_market_type && object.address == c.lending_market_id,
                "incremental adapter scope mismatch"
            );
            ensure!(
                integer(&object.contents, "version")? == c.market_version,
                "unsupported market schema"
            );
            Market::new(object.clone(), &c.original_package_id)?;
            projection.kind = "market";
        } else if object.move_type
            == suilend::obligation_type(&c.original_package_id, &c.lending_market_type)
        {
            ensure!(
                string(&object.contents, "lending_market_id")? == c.lending_market_id,
                "position scope mismatch"
            );
            projection.kind = "position";
            projection.dependencies = vec![
                format!("object:{}", c.lending_market_id),
                format!("object:{}", c.upgrade_cap_id),
                format!("position:{}", object.address),
            ];
            for side in ["deposits", "borrows"] {
                for leg in array(&object.contents, side)? {
                    projection
                        .dependencies
                        .push(format!("asset:{}", string(leg, "coin_type")?));
                    projection.dependencies.push(format!(
                        "reserve:{}:{}",
                        c.lending_market_id,
                        integer(leg, "reserve_array_index")?
                    ));
                }
            }
        } else if object.move_type
            == suilend::cap_type(&c.original_package_id, &c.lending_market_type)
        {
            projection.kind = "capability";
            projection.invalidates.push(format!(
                "position:{}",
                string(&object.contents, "obligation_id")?
            ));
        } else {
            anyhow::bail!("unsupported Suilend object type")
        }
        projection.dependencies.sort();
        projection.dependencies.dedup();
        Ok(projection)
    }
    fn evaluation_inputs_unchanged(&self, before: &Object, after: &Object) -> Result<bool> {
        if before.address != self.config.suilend.lending_market_id
            || before.address != after.address
            || before.move_type != after.move_type
        {
            return Ok(false);
        }
        // Validate deployment/schema even if no cached calculation needs rewriting.
        self.project(before)?;
        self.project(after)?;
        let references = |o: &Object| -> Result<Vec<(u64, String)>> {
            array(&o.contents, "reserves")?
                .iter()
                .map(|r| {
                    Ok((
                        integer(r, "array_index")?,
                        string(r, "coin_type")?.to_owned(),
                    ))
                })
                .collect()
        };
        Ok(references(before)? == references(after)?)
    }
    fn evaluate(
        &self,
        position: &Object,
        inputs: &BTreeMap<String, std::sync::Arc<Object>>,
        checkpoint: &Checkpoint,
    ) -> Result<Value> {
        let market = inputs
            .get(&self.config.suilend.lending_market_id)
            .context("missing market dependency")?;
        suilend::validate_position(
            position,
            market,
            &self.config.suilend.original_package_id,
            &self.config.suilend.lending_market_type,
        )?;
        let caps: Vec<_> = inputs
            .values()
            .filter(|o| {
                o.contents.get("obligation_id").and_then(Value::as_str) == Some(&position.address)
            })
            .map(|o| json!({"capability":o.address,"owner":o.owner}))
            .collect();
        let diagnostic = risk::assess(position);
        Ok(json!({
            "basis":"unverified_cached_diagnostics", "sui_collateral_native_usdc_debt_focus":focus(position)?, "checkpoint":checkpoint,
            "position_version":position.version,"capabilities":caps,
            "cached_diagnostics":diagnostic.as_ref().ok(),"diagnostic_error":diagnostic.err().map(|e|e.to_string()),
            "fresh_risk":null,"valuation_time":null,"execution_eligible":false,
            "limitation":"Only dependencies used by cached diagnostics trigger recalculation. Clock and unchanged reserve mappings may reuse the stored calculation checkpoint; totals are not refreshed prices or interest."
        }))
    }
}

/// Watchlist selection uses coin types, never ambiguous ticker symbols.
fn focus(position: &Object) -> Result<bool> {
    fn equivalent(a: &str, b: &str) -> bool {
        a.trim_start_matches("0x").trim_start_matches('0')
            == b.trim_start_matches("0x").trim_start_matches('0')
    }
    Ok(array(&position.contents, "deposits")?.iter().any(|leg| {
        leg["coin_type"]
            .as_str()
            .is_some_and(|ty| equivalent(ty, suilend::SUI))
    }) && array(&position.contents, "borrows")?.iter().any(|leg| {
        leg["coin_type"]
            .as_str()
            .is_some_and(|ty| equivalent(ty, suilend::NATIVE_USDC))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn focus_uses_native_usdc_and_accepts_padded_sui_type_names() {
        let mut object = Object {
            address: String::new(),
            version: 1,
            digest: String::new(),
            move_type: String::new(),
            owner: None,
            contents: json!({"deposits":[{"coin_type":format!("{}2::sui::SUI","0".repeat(63))}],"borrows":[{"coin_type":suilend::NATIVE_USDC}]}),
        };
        assert!(focus(&object).unwrap());
        object.contents["borrows"][0]["coin_type"] =
            json!("0x5d4b302506645c37ff133b98c4b50a5ae14841659738d6d733d59d0d217a93bf::coin::COIN");
        assert!(!focus(&object).unwrap());
    }
}
