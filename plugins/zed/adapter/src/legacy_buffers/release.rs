//! Capture one legacy owner without resolving its path or adopting a new pin.
use super::{NativeKey, Registry, RequestKey};
use anyhow::{Context as _, Result, ensure};
use std::sync::Arc;

pub(crate) struct Capture {
    pub(crate) requested: RequestKey,
    pub(crate) key: NativeKey,
    pub(crate) buffer_id: u64,
    incarnation: Arc<()>,
}

impl Capture {
    pub(crate) fn same_owner(&self, other: &Self) -> bool {
        self.requested == other.requested
            && self.key == other.key
            && self.buffer_id == other.buffer_id
            && Arc::ptr_eq(&self.incarnation, &other.incarnation)
    }
}

impl Registry {
    pub(crate) fn capture(&self, requested: RequestKey) -> Result<Capture> {
        let pin = self
            .get(&requested)
            .context("original legacy owner is not retained")?;
        ensure!(
            pin.release.is_none(),
            "original legacy release remains fenced"
        );
        Ok(Capture {
            requested,
            key: pin.key.clone(),
            buffer_id: pin.buffer_id.context("legacy Open remains unconfirmed")?,
            incarnation: Arc::clone(&pin.incarnation),
        })
    }

    pub(crate) fn check_release(&self, captured: &Capture, operation: Option<u64>) -> Result<()> {
        let pin = self
            .get(&captured.requested)
            .context("original legacy owner disappeared")?;
        ensure!(
            Arc::ptr_eq(&pin.incarnation, &captured.incarnation)
                && pin.key == captured.key
                && pin.buffer_id == Some(captured.buffer_id)
                && pin.release == operation,
            "original legacy owner or release changed"
        );
        Ok(())
    }

    pub(crate) fn mark_release(&mut self, captured: &Capture, operation: u64) {
        for ((_, _, owner), pin) in &mut self.0 {
            if owner == &captured.requested.2 && pin.key == captured.key {
                pin.release = Some(operation);
            }
        }
    }
}
