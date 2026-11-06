//! A non-reviving, non-queueing supervision delivery. An uncertain dispatch is
//! never retried automatically; the Controller ledger preserves its reservation.
use super::*;

impl DispatchOwnership {
    fn supervised(
        hub: &Hub,
        session: &str,
        epoch: u64,
        context: crate::superagent::DispatchContext,
    ) -> Arc<Self> {
        Arc::new(Self {
            hub: Arc::downgrade(&hub.inner),
            session: session.into(),
            epoch,
            supervision: Some(context),
        })
    }
    pub fn supervision(&self) -> Option<&crate::superagent::DispatchContext> {
        self.supervision.as_ref()
    }
}

impl Hub {
    pub(super) fn note_dispatched(s: &mut Session, cmid: Option<&str>) {
        let Some(cmid) = cmid.filter(|c| Self::tracks_delivery(c)) else {
            return;
        };
        if s.dispatched_cmids.iter().any(|seen| seen.cmid == cmid) {
            return;
        }
        if s.dispatched_cmids.len() >= 64 {
            s.dispatched_cmids.pop_front();
        }
        s.dispatched_cmids.push_back(DispatchedPrompt {
            cmid: cmid.to_owned(),
            epoch: s.in_flight_epoch,
            echoed: false,
        });
    }

    pub fn abandon_superagent_prompt(&self, session_id: &str, cmid: &str) {
        {
            let mut sessions = self.inner.sessions.lock();
            if let Some(s) = sessions.get_mut(session_id)
                && s.dispatched_cmids
                    .iter()
                    .any(|p| p.cmid == cmid && p.epoch == s.in_flight_epoch)
            {
                s.in_flight = false;
                s.dispatch_pending = None;
            }
        }
        // Failed supervision never enters the durable user queue.
        self.try_drain(session_id);
    }

    pub fn superagent_dispatch_current(
        &self,
        context: &crate::superagent::DispatchContext,
        cmid: &str,
    ) -> bool {
        let sessions = self.inner.sessions.lock();
        [context.target(), context.peer()].iter().all(|id| {
            sessions.get(*id).is_some_and(|s| {
                s.meta.owner_user_id.as_deref() == Some(context.owner.as_str())
                    && !s.meta.paused
                    && !s.meta.closing
                    && !s.meta.system
                    && s.meta.trashed_at_ms.is_none()
                    && s.meta.attention.is_empty()
                    && s.queue.is_empty()
                    && matches!(s.meta.status, Status::Running | Status::Busy)
            })
        }) && sessions.get(context.target()).is_some_and(|s| {
            s.meta.status == Status::Running
                && s.dispatch_pending == Some(s.in_flight_epoch)
                && s.dispatched_cmids
                    .iter()
                    .any(|p| p.cmid == cmid && p.epoch == s.in_flight_epoch)
        })
    }
    /// Old retained workers may label a new Controller-origin prompt as human.
    /// Normalize its admitted echo before persistence, without a worker upgrade.
    pub(super) fn annotate_dispatched_prompt(s: &Session, event: &mut Event, cmid: Option<&str>) {
        let Some(cmid) = cmid.filter(|id| s.dispatched_cmids.iter().any(|p| p.cmid == *id)) else {
            return;
        };
        let origin = crate::prompt_origin::origin_from_cmid(Some(cmid));
        if !origin.is_human()
            && let Event::Update { update } = event
            && update["sessionUpdate"] == "user_message_chunk"
        {
            crate::prompt_origin::apply_prompt_origin(update, &origin);
        }
    }
    #[must_use]
    pub fn session_has_in_flight_prompt(&self, session_id: &str) -> bool {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .is_some_and(|s| s.in_flight)
    }

    /// No user queue item is overtaken and Pause/Stop cannot be overridden.
    /// The caller serializes policy admission and commits a receipt first.
    pub fn dispatch_superagent_once(
        &self,
        session_id: &str,
        text: String,
        cmid: String,
        context: crate::superagent::DispatchContext,
    ) -> bool {
        let dispatch = self.inner.dispatch_tx.lock();
        let Some(tx) = dispatch.as_ref() else {
            return false;
        };
        let Ok(permit) = tx.try_reserve() else {
            return false;
        };
        let req = {
            let mut sessions = self.inner.sessions.lock();
            if context.target() != session_id
                || sessions.get(context.peer()).is_none_or(|p| {
                    p.meta.paused
                        || p.meta.closing
                        || p.meta.trashed_at_ms.is_some()
                        || !matches!(p.meta.status, Status::Running | Status::Busy)
                        || !p.meta.attention.is_empty()
                        || !p.queue.is_empty()
                        || p.meta.owner_user_id.as_deref() != Some(context.owner.as_str())
                        || sessions
                            .get(session_id)
                            .is_none_or(|target| target.meta.owner_user_id != p.meta.owner_user_id)
                })
            {
                return false;
            }
            let Some(s) = sessions.get_mut(session_id) else {
                return false;
            };
            if s.meta.paused
                || s.meta.system
                || s.meta.closing
                || s.meta.trashed_at_ms.is_some()
                || !s.queue.is_empty()
                || !Self::ready(s, false)
                || !self.context_allows_dispatch(session_id, Some(&cmid))
                || Self::prompt_already_accepted(s, &cmid)
            {
                return false;
            }
            s.in_flight_epoch = s.in_flight_epoch.wrapping_add(1);
            s.in_flight = true;
            s.dispatch_pending = Some(s.in_flight_epoch);
            Self::note_dispatched(s, Some(&cmid));
            DispatchReq {
                ownership: DispatchOwnership::supervised(
                    self,
                    session_id,
                    s.in_flight_epoch,
                    context,
                ),
                session_id: session_id.to_owned(),
                text,
                content: vec![],
                cmid: Some(cmid),
                explicit: false,
            }
        };
        permit.send(req);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Hub {
        let hub = Hub::new();
        hub.create_local_session(
            "worker".into(),
            "codex".into(),
            "/tmp".into(),
            "worker".into(),
            SessionOrigin::Web,
            false,
        );
        hub.set_status("worker", Status::Running, None);
        hub.inner
            .sessions
            .lock()
            .get_mut("worker")
            .unwrap()
            .meta
            .owner_user_id = Some("owner".into());
        hub
    }

    fn context() -> crate::superagent::DispatchContext {
        crate::superagent::DispatchContext {
            owner: "owner".into(),
            coordinator: "worker".into(),
            worker: "worker".into(),
            revision: 1,
            evidence: "fixture".into(),
            follow_up: false,
        }
    }

    #[tokio::test]
    async fn superagent_never_queues_revives_or_overtakes_a_user_prompt() {
        for held in [
            "paused", "exited", "crashed", "trashed", "closing", "queued", "hold",
        ] {
            let hub = fixture();
            let (tx, mut rx) = tokio::sync::mpsc::channel(1);
            hub.set_dispatch_tx(tx);
            {
                let mut sessions = hub.inner.sessions.lock();
                let s = sessions.get_mut("worker").unwrap();
                match held {
                    "paused" => s.meta.paused = true,
                    "exited" => s.meta.status = Status::Exited,
                    "crashed" => s.meta.status = Status::Crashed,
                    "trashed" => s.meta.trashed_at_ms = Some(1),
                    "closing" => s.meta.closing = true,
                    "queued" => s.queue.push(QueuedMessage {
                        id: "human".into(),
                        text: "wait".into(),
                        content: vec![],
                        cmid: None,
                        schedule: None,
                    }),
                    "hold" => s.delivery_hold = Some("interruption".into()),
                    _ => unreachable!(),
                }
            }
            assert!(
                !hub.dispatch_superagent_once(
                    "worker",
                    "follow".into(),
                    "cowboy-superagent:one".into(),
                    context()
                ),
                "{held}"
            );
            assert!(rx.try_recv().is_err());
            assert_eq!(
                hub.session_info("worker").unwrap().queue_count,
                usize::from(held == "queued")
            );
        }
    }

    #[tokio::test]
    async fn superagent_final_handoff_rechecks_pause_and_late_failure_keeps_newer_work() {
        let hub = fixture();
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        hub.set_dispatch_tx(tx);
        let cmid = "cowboy-superagent:held";
        assert!(hub.dispatch_superagent_once("worker", "follow".into(), cmid.into(), context()));
        let _req = rx.recv().await.unwrap();
        assert!(hub.superagent_dispatch_current(&context(), cmid));
        hub.set_paused("worker", true);
        assert!(!hub.superagent_dispatch_current(&context(), cmid));
        hub.set_paused("worker", false);
        {
            let mut sessions = hub.inner.sessions.lock();
            let session = sessions.get_mut("worker").unwrap();
            session.in_flight_epoch += 1;
            session.in_flight = true;
            session.dispatch_pending = Some(session.in_flight_epoch);
        }
        hub.requeue_prompt("worker", "late failure".into(), vec![], Some(cmid.into()));
        assert!(hub.session_has_in_flight_prompt("worker"));
        assert_eq!(hub.session_info("worker").unwrap().queue_count, 0);
        assert!(!hub.superagent_dispatch_current(&context(), cmid));
    }

    #[tokio::test]
    async fn superagent_dispatch_has_no_human_origin_and_is_never_replayed() {
        let hub = fixture();
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        hub.set_dispatch_tx(tx);
        let cmid = "cowboy-superagent:reserved-operation";
        assert!(hub.dispatch_superagent_once("worker", "follow".into(), cmid.into(), context()));
        let req = rx.recv().await.unwrap();
        assert!(!req.explicit);
        assert!(!crate::prompt_origin::origin_from_cmid(req.cmid.as_deref()).is_human());
        hub.push_tagged(
            "worker",
            Event::Update {
                update: serde_json::json!({
                    "sessionUpdate":"user_message_chunk","content":{"type":"text","text":"follow"},
                    "promptOrigin":{"actor":"human","source":"composer"}
                }),
            },
            Some(cmid.into()),
        );
        let (log, _) = hub.snapshot("worker").unwrap();
        let Event::Update { update } = &log.last().unwrap().event else {
            panic!("missing echo");
        };
        assert_eq!(update["promptOrigin"]["source"], "superagent");
        assert!(!crate::prompt_origin::is_human_prompt_update(update));
        assert!(!hub.dispatch_superagent_once("worker", "follow".into(), cmid.into(), context()));
        hub.requeue_prompt("worker", "must not retry".into(), vec![], Some(cmid.into()));
        assert!(!hub.session_has_in_flight_prompt("worker"));
        assert_eq!(hub.session_info("worker").unwrap().queue_count, 0);
    }
}
