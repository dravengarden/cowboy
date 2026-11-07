//! Bounded receipt retirement retains the original runtime through confirmation.
use super::{Reference, Reply, RunningCodeRuntime, State, request};
use anyhow::Result;
use parking_lot::Mutex;
use serde_json::json;
use std::{collections::VecDeque, sync::Arc};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

struct Retirement {
    runtime: Arc<RunningCodeRuntime>,
    reference: Reference,
    _permit: OwnedSemaphorePermit,
}
pub(in crate::machine_code_plugins) struct Retirements {
    permits: Arc<Semaphore>,
    pending: Mutex<VecDeque<Retirement>>,
}
impl Default for Retirements {
    fn default() -> Self {
        Self {
            permits: Arc::new(Semaphore::new(1024)),
            pending: Mutex::default(),
        }
    }
}
struct Guard<'a> {
    queue: &'a Mutex<VecDeque<Retirement>>,
    item: Option<Retirement>,
}
impl Drop for Guard<'_> {
    fn drop(&mut self) {
        if let Some(item) = self.item.take() {
            self.queue.lock().push_back(item);
        }
    }
}

impl Retirements {
    pub(in crate::machine_code_plugins) fn reserve(&self) -> Result<OwnedSemaphorePermit> {
        Ok(self.permits.clone().try_acquire_owned()?)
    }
    pub(super) fn retire(
        &self,
        runtime: Arc<RunningCodeRuntime>,
        reference: Reference,
        permit: OwnedSemaphorePermit,
    ) {
        self.pending.lock().push_back(Retirement {
            runtime,
            reference,
            _permit: permit,
        });
    }
    pub(in crate::machine_code_plugins) async fn observe(&self) {
        let count = self.pending.lock().len().min(4);
        for _ in 0..count {
            let Some(item) = self.pending.lock().pop_front() else {
                break;
            };
            let mut guard = Guard {
                queue: &self.pending,
                item: Some(item),
            };
            let item = guard.item.as_ref().unwrap();
            if matches!(item.runtime.is_running(), Ok(false)) {
                guard.item.take();
                continue;
            }
            let result = request(
                &item.runtime.socket,
                &json!({"type":"agentAcquisition","acquisition":item.reference,"action":"retire"}),
            )
            .await
            .and_then(|value| Reply::parse(value, Some(&item.reference)));
            if matches!(result, Ok(reply) if reply.state == State::Retired) {
                guard.item.take();
            }
        }
    }
}
