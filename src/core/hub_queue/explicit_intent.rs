//! The explicit send intent a held queue row carries through a resource wait,
//! and the wait on an execution Machine that only maintenance may end.
//!
//! The intent lives on the row itself ([`QueuedMessage::explicit`]), so the
//! queue order, the intent and the held command id are one durable write: a
//! reorder, removal or requeue can never leave an intent bound to the wrong
//! row, and a Controller crash between two writes cannot split them. Only the
//! head row carries one; every queue write ends any other row's intent.
use super::*;

/// The resource wait of an admitted prompt whose execution Machine is offline
/// or booting. While it is recorded, only the maintenance retry, which first
/// checks that Machine, delivers the session's queue head.
pub const TARGET_WAIT_REASON: &str = "the execution Machine is not ready";

/// The persisted wait field that keeps an admitted prompt's right to the
/// maintenance retry once its execution Machine is back but another resource
/// (a worker slot, memory) now blocks it: the displayed reason changes, the
/// retry eligibility does not.
pub const TARGET_RETRY_FIELD: &str = "retry_target";

/// How long a client may replay a held Send now whose reply it lost. A replay
/// issued earlier is refused without effect; the Controller remembers every
/// ended held command at least this long, so no replay inside it is mistaken
/// for a new send.
pub const HELD_SEND_REPLAY_HORIZON_MS: i64 = 24 * 60 * 60 * 1000;

/// A safety bound on remembered held commands per session. Pruning is by age
/// ([`HELD_SEND_REPLAY_HORIZON_MS`]); this only bounds a pathological burst.
const SETTLED_HELD_SENDS: usize = 1024;

/// The session's explicit-send bookkeeping beside its rows' intents.
#[derive(Default)]
pub(in crate::core) struct ExplicitIntent {
    /// Command ids of held Send nows whose intent ended (sent, superseded,
    /// removed), with when. A replay of one sends nothing. Persisted.
    settled: VecDeque<(String, i64)>,
    /// `settled` changed since it was last written.
    settled_dirty: bool,
    /// The held command the head carried at the last queue write. Once no
    /// row carries it (the row was removed, cleared or edited away), it ended.
    held: Option<String>,
    /// The prompt cmid a held send was dispatched under, and its command id:
    /// a transient refusal that requeues that prompt gives the command back.
    dispatched: Option<(String, String)>,
    /// The persisted wait is for the execution Machine (see
    /// [`TARGET_WAIT_REASON`]), or keeps that retry right. Kept with the queue
    /// so a drain checks it under the same lock that takes the head; written
    /// with the wait itself.
    pub(in crate::core) awaits_target: bool,
    /// A resource wait is persisted, so maintenance will retry the session.
    waiting: bool,
}

/// The persisted wait that `reason` blocks next, after `current`: it keeps
/// when the wait began and the execution-Machine retry right.
pub fn next_wait(current: Option<&serde_json::Value>, reason: &str) -> serde_json::Value {
    let since = current
        .and_then(|v| v.get("since_ms").and_then(serde_json::Value::as_i64))
        .unwrap_or_else(|| chrono::Utc::now().timestamp_millis());
    let mut wait = serde_json::json!({"reason": reason, "since_ms": since});
    if reason == TARGET_WAIT_REASON || current.is_some_and(wait_awaits_target) {
        wait[TARGET_RETRY_FIELD] = serde_json::Value::Bool(true);
    }
    wait
}

/// Whether a persisted wait keeps the execution-Machine retry right.
pub fn wait_awaits_target(wait: &serde_json::Value) -> bool {
    wait.get("reason").and_then(serde_json::Value::as_str) == Some(TARGET_WAIT_REASON)
        || wait
            .get(TARGET_RETRY_FIELD)
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
}

impl ExplicitIntent {
    /// A restored session's bookkeeping. A wait persisted before intents moved
    /// onto rows names its row: that head takes the intent back (`migrated`,
    /// so the queue is written with it).
    pub(in crate::core) fn restored(
        hub: &Hub,
        session_id: &str,
        queue: &mut [QueuedMessage],
        migrated: &mut bool,
    ) -> Self {
        let wait = hub.setting(&format!(
            "{}{session_id}",
            settings_keys::SESSION_RESOURCE_WAIT_PREFIX
        ));
        let field = |name: &str| {
            wait.as_ref()
                .and_then(|value| value.get(name))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        };
        let legacy = wait
            .as_ref()
            .and_then(|value| value.get("explicit"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if legacy
            && let Some(head) = queue.first_mut()
            && head.explicit.is_none()
            && field("row").is_none_or(|row| row == head.id)
        {
            head.explicit = Some(ExplicitSend {
                command: field("cmid"),
            });
            *migrated = true;
        }
        let now = chrono::Utc::now().timestamp_millis();
        let settled = hub
            .setting(&held_sends_key(session_id))
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|entry| match entry {
                // Before receipts carried a time: keep them a full horizon.
                serde_json::Value::String(cmid) => Some((cmid, now)),
                entry => Some((
                    entry.get("cmid")?.as_str()?.to_owned(),
                    entry.get("at_ms")?.as_i64()?,
                )),
            })
            .collect();
        let mut intent = Self {
            settled,
            settled_dirty: false,
            held: queue
                .first()
                .and_then(|head| head.explicit.as_ref())
                .and_then(|intent| intent.command.clone()),
            dispatched: None,
            awaits_target: wait.as_ref().is_some_and(wait_awaits_target),
            waiting: wait.is_some(),
        };
        intent.prune(now);
        intent
    }

    fn prune(&mut self, now: i64) {
        let before = self.settled.len();
        self.settled
            .retain(|(_, at)| now.saturating_sub(*at) <= HELD_SEND_REPLAY_HORIZON_MS);
        while self.settled.len() > SETTLED_HELD_SENDS {
            self.settled.pop_front();
        }
        self.settled_dirty |= before != self.settled.len();
    }

    fn settle(&mut self, cmid: String) {
        let now = chrono::Utc::now().timestamp_millis();
        self.settled.retain(|(settled, _)| settled != &cmid);
        self.settled.push_back((cmid, now));
        self.settled_dirty = true;
        self.prune(now);
    }

    fn unsettle(&mut self, cmid: &str) {
        let before = self.settled.len();
        self.settled.retain(|(settled, _)| settled != cmid);
        self.settled_dirty |= before != self.settled.len();
    }

    fn is_settled(&self, cmid: &str) -> bool {
        let now = chrono::Utc::now().timestamp_millis();
        self.settled.iter().any(|(settled, at)| {
            settled == cmid && now.saturating_sub(*at) <= HELD_SEND_REPLAY_HORIZON_MS
        })
    }
}

fn held_sends_key(session_id: &str) -> String {
    format!("{}{session_id}", settings_keys::SESSION_HELD_SENDS_PREFIX)
}

/// One serialized section for every resource-wait write and every held-row
/// change that must agree with it: a wait read before another writer's change
/// is never written back over it, and installing a hold cannot interleave with
/// a completion clearing the wait.
fn wait_serial() -> parking_lot::MutexGuard<'static, ()> {
    static SERIAL: parking_lot::Mutex<()> = parking_lot::Mutex::new(());
    SERIAL.lock()
}

impl Session {
    /// Whether the head row carries an explicit send intent.
    pub(in crate::core) fn explicit_head(&self) -> bool {
        self.queue
            .first()
            .is_some_and(|head| head.explicit.is_some())
    }

    /// End `row`'s intent; its held command id is remembered, so a client
    /// replaying that command after a reconnect sends nothing.
    fn end_row_intent(explicit: &mut ExplicitIntent, row: &mut QueuedMessage) {
        if let Some(command) = row.explicit.take().and_then(|intent| intent.command) {
            explicit.settle(command);
        }
    }

    /// End the head row's explicit send intent.
    pub(in crate::core) fn end_explicit_intent(&mut self) {
        if let Some(head) = self.queue.first_mut() {
            Self::end_row_intent(&mut self.explicit, head);
        }
    }

    /// Only the head may carry an intent, and no draft: a reorder, removal,
    /// front insert or move to drafts that took a held row off the head ended
    /// its intent (another row's Send now superseded it). Run before every
    /// queue write, so the intent commits with the order that ended it.
    pub(in crate::core) fn normalize_explicit_intents(&mut self) {
        let Self {
            queue,
            drafts,
            explicit,
            ..
        } = self;
        for row in queue.iter_mut().skip(1).chain(drafts.iter_mut()) {
            Self::end_row_intent(explicit, row);
        }
        let held = queue
            .first()
            .and_then(|head| head.explicit.as_ref())
            .and_then(|intent| intent.command.clone());
        if let Some(ended) = std::mem::replace(&mut explicit.held, held.clone())
            && Some(&ended) != held.as_ref()
        {
            explicit.settle(ended);
        }
    }

    /// The head row leaves the queue as prompt `dispatch_cmid`. Its intent
    /// travels with the dispatch (whether it was explicit), and its held
    /// command ends now: a replay while it is in flight sends nothing. A
    /// transient refusal gives the command back (see [`Hub::requeue_prompt`]).
    pub(in crate::core) fn take_dispatch_intent(
        &mut self,
        head: &mut QueuedMessage,
        dispatch_cmid: &str,
    ) -> bool {
        self.explicit.dispatched = None;
        let Some(intent) = head.explicit.take() else {
            return false;
        };
        if let Some(command) = intent.command {
            self.explicit.settle(command.clone());
            self.explicit.dispatched = Some((dispatch_cmid.to_owned(), command));
        }
        true
    }

    /// A prompt that left as `cmid` is back at `index`. Only an explicit one
    /// returned to the head keeps its intent, with its held command; a row
    /// already there that is returned as an ordinary send loses it.
    pub(in crate::core) fn restore_dispatch_intent(
        &mut self,
        index: usize,
        cmid: Option<&str>,
        explicit: bool,
    ) {
        let command = self
            .explicit
            .dispatched
            .take_if(|(dispatched, _)| cmid == Some(dispatched.as_str()))
            .map(|(_, command)| command);
        let Some(row) = self.queue.get_mut(index) else {
            return;
        };
        if !explicit {
            Self::end_row_intent(&mut self.explicit, row);
            return;
        }
        if index != 0 {
            return;
        }
        if row.explicit.is_some() {
            return;
        }
        if let Some(command) = &command {
            self.explicit.unsettle(command);
        }
        row.explicit = Some(ExplicitSend { command });
    }

    /// Whether only a retry that checked the session's eligibility may take
    /// the head: the session waits for its execution Machine, or the head is
    /// a held explicit send (an account park or offline target holds it). Any
    /// other automatic drain would take it out only for the dispatcher to
    /// refuse it and requeue it under a new id, which a terminal holding an
    /// edit of that row reads as a sent message and parks as a draft beside
    /// the original.
    pub(in crate::core) fn head_awaits_retry(&self) -> bool {
        self.explicit.awaits_target || (self.explicit.waiting && self.explicit_head())
    }

    /// The remembered held commands to write, once they changed.
    pub(in crate::core) fn take_settled_receipts(&mut self) -> Option<serde_json::Value> {
        if !std::mem::take(&mut self.explicit.settled_dirty) {
            return None;
        }
        Some(serde_json::Value::Array(
            self.explicit
                .settled
                .iter()
                .map(|(cmid, at)| serde_json::json!({"cmid": cmid, "at_ms": at}))
                .collect(),
        ))
    }
}

impl Hub {
    /// The store key and value of a session's remembered held commands, when
    /// they changed. The caller writes them before the queue snapshot that
    /// ended them, in the same FIFO.
    pub(in crate::core) fn settled_receipts_write(
        session_id: &str,
        s: &mut Session,
    ) -> Option<(String, serde_json::Value)> {
        s.take_settled_receipts()
            .map(|value| (held_sends_key(session_id), value))
    }

    /// Whether `cmid` is a held Send now this Hub already recorded: `Some(true)`
    /// while its intent still waits, `Some(false)` once that intent ended. A
    /// client replays a command whose reply it lost; the replay must not take
    /// the live send path, which would dispatch the row past the wait (and an
    /// edit of it) or send a row the user has since reordered.
    pub fn held_send_replay(&self, session_id: &str, cmid: &str) -> Option<bool> {
        let sessions = self.inner.sessions.lock();
        let s = sessions.get(session_id)?;
        if s.queue.first().is_some_and(|head| {
            head.explicit
                .as_ref()
                .is_some_and(|intent| intent.command.as_deref() == Some(cmid))
        }) {
            return Some(true);
        }
        s.explicit.is_settled(cmid).then_some(false)
    }

    /// Whether an edit holds back what a retry of this wait would send: the
    /// head row for an explicit send, any queued row for an ordinary one.
    pub fn retry_held_for_editing(&self, session_id: &str, explicit: bool) -> bool {
        self.inner.sessions.lock().get(session_id).is_some_and(|s| {
            let Some(editing) = s.editing.as_deref() else {
                return false;
            };
            if explicit {
                s.queue.first().is_some_and(|head| head.id == editing)
            } else {
                !s.drafts.iter().any(|draft| draft.id == editing)
            }
        })
    }

    /// End the head row's explicit intent (a wait that ended, a workspace
    /// preparation that will drain it itself).
    pub fn end_explicit_intent(&self, session_id: &str) {
        let ended = {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            let held = s.explicit_head();
            s.end_explicit_intent();
            held
        };
        if ended {
            self.emit_pending(session_id);
        }
    }

    /// Move a queued prompt to the front under `caller`'s send intent without
    /// dispatching it, holding it there as an explicit send that waits for
    /// `reason` (see [`next_wait`]). The wait is written first,
    /// so an automatic drain that sees the held row already finds the session
    /// waiting (and the wait commits before the row in the store FIFO). A row
    /// another terminal removed leaves the previous wait exactly as it was.
    /// Whether the row was held.
    pub fn hold_queued(
        &self,
        session_id: &str,
        id: &str,
        caller: &crate::prompt_origin::RequestCaller,
        command: Option<&str>,
        reason: &str,
    ) -> bool {
        if self.is_managed_child(session_id) {
            return false;
        }
        let _serial = wait_serial();
        let key = format!(
            "{}{session_id}",
            settings_keys::SESSION_RESOURCE_WAIT_PREFIX
        );
        let previous = self.setting(&key);
        let previous_reason = self
            .inner
            .sessions
            .lock()
            .get(session_id)
            .and_then(|s| s.meta.resource_wait.clone());
        self.set_resource_wait(session_id, Some(reason.to_owned()));
        self.edit_resource_wait_locked(session_id, |current| {
            Some(next_wait(current.as_ref(), reason))
        });
        let trigger = crate::prompt_origin::DeliveryOrigin::from_caller(caller);
        let held = {
            let mut sessions = self.inner.sessions.lock();
            sessions.get_mut(session_id).is_some_and(|s| {
                if s.steer_reservations.values().any(|r| r.row_id == id) {
                    return false;
                }
                let Some(pos) = s.queue.iter().position(|m| m.id == id) else {
                    return false;
                };
                let mut row = s.queue.remove(pos);
                row.initiator = Some(trigger);
                if let Some(intent) = row.explicit.take()
                    && let Some(old) = intent.command
                    && Some(old.as_str()) != command
                {
                    s.explicit.settle(old);
                }
                if let Some(command) = command {
                    s.explicit.unsettle(command);
                }
                row.explicit = Some(ExplicitSend {
                    command: command.map(str::to_owned),
                });
                s.queue.insert(0, row);
                true
            })
        };
        if held {
            self.emit_pending(session_id);
        } else {
            self.set_resource_wait(session_id, previous_reason);
            self.edit_resource_wait_locked(session_id, |_| previous);
        }
        held
    }

    /// Clear the session's resource wait, unless its head is a held explicit
    /// send (held meanwhile for another row): that keeps the wait it needs.
    /// Checked under the section that installs a hold. Whether it cleared.
    pub fn clear_resource_wait_unless_held(&self, session_id: &str) -> bool {
        let _serial = wait_serial();
        if self
            .inner
            .sessions
            .lock()
            .get(session_id)
            .is_some_and(Session::explicit_head)
        {
            return false;
        }
        self.set_resource_wait(session_id, None);
        self.edit_resource_wait_locked(session_id, |_| None);
        true
    }

    /// End the session's resource wait and the head's explicit intent with it.
    pub fn clear_resource_wait(&self, session_id: &str) {
        let _serial = wait_serial();
        self.set_resource_wait(session_id, None);
        self.end_explicit_intent(session_id);
        self.edit_resource_wait_locked(session_id, |_| None);
    }

    /// Replace the session's persisted resource wait with `edit(current)`
    /// (`None` ends it). Every writer goes through here, serialized: a reason
    /// read before another writer's change is never written back over it.
    pub fn edit_resource_wait(
        &self,
        session_id: &str,
        edit: impl FnOnce(Option<serde_json::Value>) -> Option<serde_json::Value>,
    ) {
        let _serial = wait_serial();
        self.edit_resource_wait_locked(session_id, edit);
    }

    fn edit_resource_wait_locked(
        &self,
        session_id: &str,
        edit: impl FnOnce(Option<serde_json::Value>) -> Option<serde_json::Value>,
    ) {
        let key = format!(
            "{}{session_id}",
            settings_keys::SESSION_RESOURCE_WAIT_PREFIX
        );
        let current = self.setting(&key);
        let mut edited = edit(current.clone());
        // The intent binding moved onto the rows; a legacy one is dropped.
        if let Some(serde_json::Value::Object(wait)) = &mut edited {
            for legacy in ["explicit", "row", "cmid"] {
                wait.remove(legacy);
            }
        }
        if let Some(s) = self.inner.sessions.lock().get_mut(session_id) {
            s.explicit.awaits_target = edited.as_ref().is_some_and(wait_awaits_target);
            s.explicit.waiting = edited.is_some();
        }
        match edited {
            Some(wait) if current.as_ref() != Some(&wait) => self.set_setting(key, wait),
            None if current.is_some() => self.set_setting(key, serde_json::Value::Null),
            _ => {}
        }
    }
}
