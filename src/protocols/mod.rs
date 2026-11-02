//! Protocol adapters for typed incremental observation and capability-specific evaluation.

pub mod adapter;
pub mod registry;
pub mod suilend;
pub mod suilend_incremental;

/// What an adapter declares about itself (temp.md section 9: "protocol
/// ID, supported package/type origins and versions, discovery strategy,
/// required objects/feeds, capabilities, confidence requirements, and
/// adapter build/version"). Only the fields this repository has a real,
/// checkable answer for today are present; a field with no current
/// meaning (e.g. a confidence-requirements schema) is not stubbed in with
/// a placeholder value, per this project's standing rule against
/// inventing fields nothing populates.
pub struct AdapterManifest {
    pub protocol_id: &'static str,
    /// The adapter crate/build version - currently the whole binary's
    /// version, since every adapter ships in the same build. A
    /// per-adapter version becomes meaningful once a second adapter can
    /// actually diverge from it.
    pub adapter_version: &'static str,
    /// What this adapter can currently produce. Each entry here must
    /// correspond to real, implemented behavior - "cached_lending_health"
    /// reflects that `analysis::risk`/`analysis::report` compute cached,
    /// unverified diagnostics; it does not claim verified valuation,
    /// which remains blocked (see docs/suilend-notes.md).
    pub capabilities: &'static [&'static str],
}
