// SPDX-License-Identifier: GPL-3.0-or-later
//! One application-wide admission pool for native file/untitled acquisitions.
//! Text snapshots retain the same lineage, not another acquisition slot.
//! This is not an all-writer history, detached Rope or process RSS budget.
use gpui::{App, Global};
use std::sync::{Arc, atomic::AtomicUsize};

pub use text::cowboy_acquisition::{CapacityExceeded, MAX_BUFFERS, Permit};

#[derive(Default)]
struct Budget(Arc<AtomicUsize>);
impl Global for Budget {}

pub fn acquire(cx: &mut App) -> Result<Permit, CapacityExceeded> {
    Permit::acquire(&cx.default_global::<Budget>().0)
}

#[cfg(any(test, feature = "test-support"))]
pub fn in_use(cx: &App) -> usize {
    cx.try_global::<Budget>().map_or(0, |budget| {
        budget.0.load(std::sync::atomic::Ordering::Acquire)
    })
}
