// SPDX-License-Identifier: GPL-3.0-or-later
//! Shared admission lineage retained by native buffers and their text snapshots.
//! The application owns the pool; no protocol value can construct a permit.
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

pub const MAX_BUFFERS: usize = 64;

/// Clones retain one charge, including on a background thread. Not serialized.
#[derive(Clone, Debug)]
pub struct Permit {
    _charge: Arc<Charge>,
}

#[derive(Debug)]
struct Charge(Arc<AtomicUsize>);

#[derive(Debug)]
pub struct CapacityExceeded;

impl std::fmt::Display for CapacityExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("private native buffer acquisition capacity exceeded")
    }
}
impl std::error::Error for CapacityExceeded {}

impl Permit {
    pub fn acquire(used: &Arc<AtomicUsize>) -> Result<Self, CapacityExceeded> {
        used.fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
            (count < MAX_BUFFERS).then(|| count + 1)
        })
        .map_err(|_| CapacityExceeded)?;
        Ok(Self {
            _charge: Arc::new(Charge(used.clone())),
        })
    }
}

impl Drop for Charge {
    fn drop(&mut self) {
        let previous = self.0.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0);
    }
}
