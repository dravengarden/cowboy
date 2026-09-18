// SPDX-License-Identifier: GPL-3.0-or-later
//! One application-wide admission pool for native file/untitled acquisitions.
//! This bounds admitted lifetimes, not CRDT history, snapshots or process RSS.
use gpui::{App, Global};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

pub const MAX_BUFFERS: usize = 64;

#[derive(Default)]
struct Budget(Arc<AtomicUsize>);
impl Global for Budget {}

/// Clones retain the same charge; only its last holder returns capacity.
/// Not serializable, minted by a client or released by an acknowledgement.
#[derive(Clone, Debug)]
pub struct Permit {
    _charge: Arc<Charge>,
}

#[derive(Debug)]
struct Charge {
    used: Arc<AtomicUsize>,
}

#[derive(Debug)]
pub struct CapacityExceeded;

impl std::fmt::Display for CapacityExceeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("private native buffer acquisition capacity exceeded")
    }
}
impl std::error::Error for CapacityExceeded {}

pub fn acquire(cx: &mut App) -> Result<Permit, CapacityExceeded> {
    let used = &cx.default_global::<Budget>().0;
    used.fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
        (count < MAX_BUFFERS).then_some(count + 1)
    })
    .map_err(|_| CapacityExceeded)?;
    Ok(Permit {
        _charge: Arc::new(Charge { used: used.clone() }),
    })
}

impl Drop for Charge {
    fn drop(&mut self) {
        let previous = self.used.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0);
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn in_use(cx: &App) -> usize {
    cx.try_global::<Budget>()
        .map_or(0, |budget| budget.0.load(Ordering::Acquire))
}
