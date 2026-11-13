//! Atomic U64 implementation for cross-platform atomic operations.

use std::sync::atomic::{AtomicU64 as StdAtomicU64, Ordering};

/// Wrapper around AtomicU64 with a simpler API
#[derive(Debug, Default)]
pub struct AtomicU64(pub StdAtomicU64);

impl AtomicU64 {
    pub fn new(val: u64) -> Self {
        Self(StdAtomicU64::new(val))
    }

    pub fn load(&self, order: Ordering) -> u64 {
        self.0.load(order)
    }

    pub fn store(&self, val: u64, order: Ordering) {
        self.0.store(val, order)
    }

    pub fn swap(&self, val: u64, order: Ordering) -> u64 {
        self.0.swap(val, order)
    }

    pub fn compare_exchange(
        &self,
        current: u64,
        new: u64,
        success: Ordering,
        failure: Ordering,
    ) -> Result<u64, u64> {
        self.0.compare_exchange(current, new, success, failure)
    }

    pub fn compare_exchange_weak(
        &self,
        current: u64,
        new: u64,
        success: Ordering,
        failure: Ordering,
    ) -> Result<u64, u64> {
        self.0.compare_exchange_weak(current, new, success, failure)
    }

    pub fn fetch_add(&self, val: u64, order: Ordering) -> u64 {
        self.0.fetch_add(val, order)
    }

    pub fn fetch_sub(&self, val: u64, order: Ordering) -> u64 {
        self.0.fetch_sub(val, order)
    }

    pub fn fetch_and(&self, val: u64, order: Ordering) -> u64 {
        self.0.fetch_and(val, order)
    }

    pub fn fetch_or(&self, val: u64, order: Ordering) -> u64 {
        self.0.fetch_or(val, order)
    }

    pub fn fetch_xor(&self, val: u64, order: Ordering) -> u64 {
        self.0.fetch_xor(val, order)
    }
}

impl Clone for AtomicU64 {
    fn clone(&self) -> Self {
        Self(StdAtomicU64::new(self.0.load(Ordering::Relaxed)))
    }
}

impl From<u64> for AtomicU64 {
    fn from(val: u64) -> Self {
        Self::new(val)
    }
}
