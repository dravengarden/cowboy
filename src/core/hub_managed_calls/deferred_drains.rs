//! Deferred queue drain and notice-wake replay for sessions fenced by an
//! in-flight acknowledged `StoreWrite::UpdatePending`.

use super::types::{DeferredDrain, DeferredDrainActions, PendingRecoveryPromptItem};
use super::*;

impl Hub {
    pub(in crate::core) fn record_deferred_drain_if_fenced(
        &self,
        session_id: &str,
        head_id: Option<&str>,
        manual: bool,
        allow_revive: bool,
        trigger: Option<&crate::prompt_origin::RequestCaller>,
    ) -> bool {
        let mut claims = self.inner.notice_claims.lock();
        if self.is_pending_writes_poisoned(session_id) {
            return true;
        }
        let Some(deferred) = claims.in_flight_pending_commits.get_mut(session_id) else {
            return false;
        };
        if let Some(row_id) = head_id {
            if manual {
                deferred.drains.manual_drain = Some(DeferredDrain {
                    expected_head_id: row_id.to_owned(),
                    manual: true,
                    allow_revive,
                    trigger: trigger.cloned(),
                });
            } else if allow_revive {
                deferred.drains.revive_drain = Some(DeferredDrain {
                    expected_head_id: row_id.to_owned(),
                    manual: false,
                    allow_revive: true,
                    trigger: trigger.cloned(),
                });
            }
        }
        true
    }

    pub(in crate::core) fn record_deferred_notice_wake_if_fenced(
        &self,
        session_id: &str,
        completion_id: &str,
    ) -> bool {
        let mut claims = self.inner.notice_claims.lock();
        if self.is_pending_writes_poisoned(session_id) {
            return true;
        }
        let Some(deferred) = claims.in_flight_pending_commits.get_mut(session_id) else {
            return false;
        };
        if !deferred
            .drains
            .notice_wakes
            .iter()
            .any(|cid| cid == completion_id)
        {
            deferred.drains.notice_wakes.push(completion_id.to_owned());
        }
        true
    }

    pub(super) fn replay_deferred_drains(&self, session_id: &str, actions: DeferredDrainActions) {
        if let Some(d) = actions.manual_drain {
            self.drain_head_checked(
                session_id,
                d.allow_revive,
                d.manual,
                Some(d.expected_head_id.as_str()),
                None,
                d.trigger.as_ref(),
            );
        }
        for cid in actions.notice_wakes {
            let _ = self.wake_queued_call_notice(session_id, &cid);
        }
        if let Some(d) = actions.revive_drain {
            self.drain_head_checked(
                session_id,
                d.allow_revive,
                d.manual,
                Some(d.expected_head_id.as_str()),
                None,
                d.trigger.as_ref(),
            );
        }
        self.try_drain(session_id);
    }
}

impl Hub {
    pub(in crate::core) fn next_healed_qid(&self, existing_id: &str) -> String {
        if existing_id.starts_with("recovery-") {
            format!("recovery-{}", self.next_qid())
        } else {
            self.next_qid()
        }
    }

    pub(in crate::core) fn restored_recovery_cmids(
        &self,
        id: &str,
        drafts: &[QueuedMessage],
    ) -> HashSet<String> {
        let mut cmids: HashSet<String> = self
            .setting(&format!(
                "{}{id}",
                settings_keys::SESSION_RECOVERY_CMIDS_PREFIX
            ))
            .and_then(|value| serde_json::from_value::<Vec<String>>(value).ok())
            .map(|items| items.into_iter().collect())
            .unwrap_or_default();
        for m in drafts {
            if m.id.starts_with("recovery-")
                && let Some(cmid) = m.cmid.as_ref()
            {
                cmids.insert(cmid.clone());
            }
        }
        cmids
    }

    pub(in crate::core) fn restore_recovery_cmids_and_clean_queue(
        &self,
        id: &str,
        queue: &mut Vec<QueuedMessage>,
        drafts: &[QueuedMessage],
        pending_dirty: &mut Vec<String>,
    ) -> HashSet<String> {
        let cmids = self.restored_recovery_cmids(id, drafts);
        let before = queue.len();
        queue.retain(|m| {
            !m.cmid
                .as_deref()
                .is_some_and(|c| Self::recovery_cmids_contain(&cmids, c))
        });
        if queue.len() != before && !pending_dirty.iter().any(|seen| seen == id) {
            pending_dirty.push(id.to_owned());
        }
        cmids
    }

    pub(in crate::core) fn restored_explicit_wait(&self, id: &str) -> bool {
        self.setting(&format!(
            "{}{id}",
            settings_keys::SESSION_RESOURCE_WAIT_PREFIX
        ))
        .and_then(|value| value.get("explicit").and_then(serde_json::Value::as_bool))
        .unwrap_or(false)
    }

    pub(in crate::core) fn restored_explicit_row(&self, id: &str) -> Option<String> {
        self.setting(&format!(
            "{}{id}",
            settings_keys::SESSION_RESOURCE_WAIT_PREFIX
        ))
        .and_then(|value| {
            value
                .get("row")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
    }

    /// Controller-owned durable recovery-park retry queue (Finding 1): survives
    /// `RemoteRuntime` disconnection/replacement and retries the exact same
    /// idempotent durable park until acknowledged by the store or shutdown.
    pub fn enqueue_durable_recovery_park(
        &self,
        session_id: String,
        text: String,
        content: Vec<serde_json::Value>,
        cmid: Option<String>,
        reason: String,
        on_committed: Option<Arc<dyn Fn() + Send + Sync>>,
    ) -> bool {
        if !self.has_store_writer() {
            self.add_recovery_draft(&session_id, text, content, cmid);
            if let Some(cb) = on_committed {
                cb();
            }
            self.broadcast_error(Some(session_id), reason);
            return true;
        }
        let already_committed = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(&session_id) else {
                return false;
            };
            if let Some(ref c) = cmid
                && !c.is_empty()
            {
                if Self::is_cmid_committed_recovery(s, c) {
                    true
                } else {
                    s.pending_recovery_cmids.insert(c.clone());
                    false
                }
            } else {
                false
            }
        };
        if already_committed {
            if let Some(c) = cmid.as_deref() {
                self.confirm_delivered(&session_id, c);
            }
            if let Some(cb) = on_committed {
                cb();
            }
            self.broadcast_error(Some(session_id), reason);
            return true;
        }
        let start_worker = {
            let mut claims = self.inner.notice_claims.lock();
            if self.is_pending_writes_poisoned(&session_id) {
                return false;
            }
            let queue = claims
                .pending_recovery_parks
                .entry(session_id.clone())
                .or_default();
            if let Some(c) = cmid.as_deref()
                && let Some(existing) = queue
                    .iter_mut()
                    .find(|item| item.cmid.as_deref() == Some(c))
            {
                if let Some(cb) = on_committed {
                    existing.on_committed.push(cb);
                }
            } else {
                queue.push(PendingRecoveryPromptItem {
                    text,
                    content,
                    cmid,
                    reason,
                    on_committed: on_committed.into_iter().collect(),
                });
            }
            claims
                .active_recovery_park_workers
                .insert(session_id.clone())
        };
        if start_worker {
            if tokio::runtime::Handle::try_current().is_ok() {
                let hub = self.clone();
                let sid = session_id.clone();
                self.spawn_teardown_requeue(async move {
                    hub.run_durable_recovery_park_worker(sid).await;
                });
            } else {
                self.inner
                    .notice_claims
                    .lock()
                    .active_recovery_park_workers
                    .remove(&session_id);
            }
        }
        // SQL commit has not been acknowledged yet; keep runtime pending command
        // fenced until `on_committed` fires after authoritative SQL ACK (Finding 7).
        false
    }

    async fn run_durable_recovery_park_worker(&self, session_id: String) {
        let mut backoff_ms = 50_u64;
        loop {
            let next_item = {
                let mut claims = self.inner.notice_claims.lock();
                if self.is_pending_writes_poisoned(&session_id) {
                    claims.active_recovery_park_workers.remove(&session_id);
                    None
                } else if let Some(item) = claims
                    .pending_recovery_parks
                    .get(&session_id)
                    .and_then(|q| q.first())
                    .cloned()
                {
                    Some(item)
                } else {
                    claims.active_recovery_park_workers.remove(&session_id);
                    None
                }
            };
            let Some(item) = next_item else {
                break;
            };
            if !self.has_store_writer() {
                self.inner
                    .notice_claims
                    .lock()
                    .active_recovery_park_workers
                    .remove(&session_id);
                break;
            }
            let Some(_commit_guard) = self
                .await_with_shutdown(self.pending_commit_lock(&session_id))
                .await
            else {
                self.poison_session_pending_writes(&session_id);
                tracing::error!(
                    session = %session_id,
                    "durable recovery park lock wait aborted during Controller shutdown"
                );
                self.inner
                    .notice_claims
                    .lock()
                    .active_recovery_park_workers
                    .remove(&session_id);
                break;
            };
            let outcome = match self.preview_park_recovery_prompt(
                &session_id,
                item.text.clone(),
                item.content.clone(),
                item.cmid.clone(),
            ) {
                RecoveryDraftPark::NoStore | RecoveryDraftPark::Duplicate => Ok(true),
                RecoveryDraftPark::Pending(mut preview, rx) => {
                    match self.await_with_shutdown(rx).await {
                        Some(Ok(Ok(true))) => {
                            self.commit_park_recovery_prompt(&mut preview);
                            Ok(true)
                        }
                        Some(Ok(Ok(false) | Err(_))) => {
                            self.abort_park_recovery_notices(&mut preview);
                            Ok(false)
                        }
                        Some(Err(_)) => {
                            self.abort_park_recovery_notices(&mut preview);
                            Err(false)
                        }
                        None => {
                            self.abandon_unknown_outcome_recovery_park(&mut preview);
                            tracing::error!(
                                session = %session_id,
                                "durable recovery park store ack timed out during Controller shutdown"
                            );
                            Err(false)
                        }
                    }
                }
                RecoveryDraftPark::Rejected("session unavailable") => Err(true),
                RecoveryDraftPark::Rejected(_) => Ok(false),
            };
            match outcome {
                Ok(true) => {
                    backoff_ms = 50;
                    if let Some(ref c) = item.cmid
                        && let Some(s) = self.inner.sessions.lock().get_mut(&session_id)
                    {
                        s.pending_recovery_cmids.remove(c);
                    }
                    let callbacks = {
                        let mut claims = self.inner.notice_claims.lock();
                        if let Some(q) = claims.pending_recovery_parks.get_mut(&session_id)
                            && !q.is_empty()
                        {
                            let popped = q.remove(0);
                            if q.is_empty() {
                                claims.pending_recovery_parks.remove(&session_id);
                            }
                            popped.on_committed
                        } else {
                            item.on_committed
                        }
                    };
                    for cb in callbacks {
                        cb();
                    }
                    self.broadcast_error(Some(session_id.clone()), item.reason);
                }
                Ok(false) => {
                    if self.inner.teardown_shutdown.load(Ordering::Acquire) {
                        self.poison_session_pending_writes(&session_id);
                        tracing::error!(
                            session = %session_id,
                            "durable recovery park retry aborted during Controller shutdown"
                        );
                        self.inner
                            .notice_claims
                            .lock()
                            .active_recovery_park_workers
                            .remove(&session_id);
                        break;
                    }
                    tokio::select! {
                        () = tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)) => {}
                        () = self.inner.teardown_shutdown_notify.notified() => {}
                    }
                    backoff_ms = (backoff_ms * 2).min(2_000);
                }
                Err(drop_queue) => {
                    let mut claims = self.inner.notice_claims.lock();
                    if drop_queue {
                        claims.pending_recovery_parks.remove(&session_id);
                    }
                    claims.active_recovery_park_workers.remove(&session_id);
                    break;
                }
            }
        }
    }
}
