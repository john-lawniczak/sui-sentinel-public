//! Shared transport counters; bytes are decoded HTTP body bytes, not wire bytes.
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

#[derive(Default)]
pub(super) struct Counters {
    pub attempts: AtomicU64,
    pub retries: AtomicU64,
    pub throttled: AtomicU64,
    pub response_bytes: AtomicU64,
    pub failed_attempts: AtomicU64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Transport {
    pub attempts: u64,
    pub retries: u64,
    pub throttled: u64,
    pub response_bytes: u64,
    pub failed_attempts: u64,
}
impl Counters {
    pub fn snapshot(&self) -> Transport {
        Transport {
            attempts: self.attempts.load(Relaxed),
            retries: self.retries.load(Relaxed),
            throttled: self.throttled.load(Relaxed),
            response_bytes: self.response_bytes.load(Relaxed),
            failed_attempts: self.failed_attempts.load(Relaxed),
        }
    }
}
