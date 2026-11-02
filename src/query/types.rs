use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub scope: String,
    #[serde(default)]
    pub checkpoint: Option<String>,
    #[serde(default)]
    pub generation: Option<String>,
    #[serde(default)]
    pub after: Option<String>,
    #[serde(default = "limit")]
    pub limit: u32,
    #[serde(default)]
    pub id: Option<String>,
}
fn limit() -> u32 {
    25
}
impl Request {
    pub fn validate(&self) -> Result<()> {
        ensure!((1..=100).contains(&self.limit), "invalid_arguments");
        for v in [&self.id, &self.after, &self.checkpoint, &self.generation]
            .into_iter()
            .flatten()
        {
            ensure!(
                !v.is_empty()
                    && v.len() <= 128
                    && v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
                "invalid_arguments"
            );
        }
        Ok(())
    }
}
