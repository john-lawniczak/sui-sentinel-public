//! Explicit adapter registry. Unknown IDs fail instead of falling through to Suilend.
use super::{adapter::Adapter, suilend_incremental::Suilend};
use crate::config::Config;
use anyhow::Result;
use serde_json::{Value, json};
use std::sync::Arc;
pub fn build(id: &str, configuration: Value) -> Result<Arc<dyn Adapter>> {
    match id {
        "suilend.cached.v1" => {
            let config: Config = serde_json::from_value(configuration)?;
            config.validate()?;
            Ok(Arc::new(Suilend { config }))
        }
        _ => anyhow::bail!("unsupported adapter ID"),
    }
}
pub fn catalog() -> Value {
    json!([{"id":"suilend.cached.v1","protocol":"suilend","build":crate::build_identity::current(),
        "capabilities":["cached_diagnostics","ownership_observation"],
        "deployment_binding":"validated configuration_hash and package/market versions",
        "dependencies":["position","market/reserves","owner capabilities","upgrade capability"],
        "clock_invalidation":false,"fresh_risk":false,"execution_eligible":false}])
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unknown_adapters_cannot_inherit_suilend_capabilities() {
        assert!(build("navi", json!({})).is_err());
        assert!(build("suilend.cached.v1", json!({})).is_err());
        let config = Config::load("deploy/config/suilend.toml").unwrap();
        assert_eq!(
            build("suilend.cached.v1", serde_json::to_value(config).unwrap())
                .unwrap()
                .id(),
            "suilend.cached.v1"
        );
        assert_eq!(catalog()[0]["fresh_risk"], false);
    }
}
