//! Process-local evidence for one abandoned Open, never a replay capability.
use super::{
    Arc, AtomicBool, BufferLease, Fence, HashMap, Ordering, PathBuf, Zed, recover_observed,
};

#[derive(Clone)]
pub(crate) struct Recovery(pub(super) Arc<AtomicBool>);

impl Recovery {
    pub(crate) fn new() -> Self {
        // Until Attempt::track runs, this caller has dispatched no Open.
        Self(Arc::new(AtomicBool::new(true)))
    }

    pub(crate) async fn observe(
        &self,
        fence: &Fence,
        active: &HashMap<(PathBuf, PathBuf), BufferLease>,
        zed: Option<&Zed>,
    ) -> bool {
        if self.0.load(Ordering::Acquire) {
            return true;
        }
        let original = fence
            .receipt
            .lock()
            .expect("open receipt poisoned")
            .as_ref()
            .is_some_and(|receipt| Arc::ptr_eq(receipt, &self.0));
        if original && let Some(zed) = zed {
            // The caller retains the active write lock: no later attempt can
            // replace the original fence while its outcome is queried.
            let _ = recover_observed(fence, active, zed, false).await;
        }
        self.0.load(Ordering::Acquire)
    }

    /// Resolve only this acquisition, permitting its original orphan cleanup.
    /// The callback fences the caller immediately before the cleanup effect.
    pub(crate) async fn release(
        &self,
        fence: &Fence,
        active: &HashMap<(PathBuf, PathBuf), BufferLease>,
        zed: Option<&Zed>,
        dispatching: impl FnOnce(),
    ) -> anyhow::Result<bool> {
        if self.0.load(Ordering::Acquire) {
            return Ok(true);
        }
        let original = fence
            .receipt
            .lock()
            .expect("open receipt poisoned")
            .as_ref()
            .is_some_and(|receipt| Arc::ptr_eq(receipt, &self.0));
        if original && let Some(zed) = zed {
            super::recover_with(fence, active, zed, true, dispatching).await?;
        }
        Ok(self.0.load(Ordering::Acquire))
    }
}

#[derive(Default)]
pub(crate) struct Admission<'a> {
    pub(crate) expected_id: Option<u64>,
    pub(crate) recovery: Option<&'a Recovery>,
}
