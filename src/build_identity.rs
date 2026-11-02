//! A report identifies its executable; image provenance is supplied by release CI.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BuildIdentity {
    pub revision: String,
    pub executable_sha256: Option<String>,
    pub version: String,
    pub adapter: String,
}
pub fn current() -> BuildIdentity {
    static ID: std::sync::OnceLock<BuildIdentity> = std::sync::OnceLock::new();
    ID.get_or_init(|| BuildIdentity {
        revision: option_env!("SENTINEL_BUILD_REVISION")
            .unwrap_or("unrecorded")
            .into(),
        executable_sha256: std::env::current_exe()
            .ok()
            .and_then(|p| crate::storage::sha256_hex(&p).ok()),
        version: env!("CARGO_PKG_VERSION").into(),
        adapter: "suilend.cached.v1".into(),
    })
    .clone()
}
