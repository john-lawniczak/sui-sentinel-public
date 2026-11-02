//! Small capability boundary for incremental observations. Protocol decoders
//! own type routing, dependency extraction and calculation interpretation.
use crate::model::{Checkpoint, Object};
use anyhow::Result;
use serde_json::Value;
use std::collections::BTreeMap;

pub struct Projection {
    pub kind: &'static str,
    pub dependencies: Vec<String>,
    pub invalidates: Vec<String>,
}

pub trait Adapter: Send + Sync {
    fn id(&self) -> &'static str;
    fn configuration_hash(&self) -> Result<String>;
    fn project(&self, object: &Object) -> Result<Projection>;
    /// Capability-specific equivalence, never assumed by shared orchestration.
    /// Returning true must preserve every input read by this adapter's evaluator.
    fn evaluation_inputs_unchanged(&self, _before: &Object, _after: &Object) -> Result<bool> {
        Ok(false)
    }
    fn evaluate(
        &self,
        position: &Object,
        inputs: &BTreeMap<String, std::sync::Arc<Object>>,
        checkpoint: &Checkpoint,
    ) -> Result<Value>;
}
