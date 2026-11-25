//! The explicit send intent a held queue row carries through a resource wait,
//! and the wait on an execution Machine that only maintenance may end.
use super::*;

/// The resource wait of an admitted prompt whose execution Machine is offline
/// or booting. While it is recorded, only the maintenance retry, which first
/// checks that Machine, delivers the session's queue head.
pub const TARGET_WAIT_REASON: &str = "the execution Machine is not ready";

/// How many ended held-send command ids a session remembers for late replays.
const SETTLED_HELD_SENDS: usize = 64;

/// What an explicit send intent (`Session::explicit_wait`) belongs to.
#[derive(Default)]
pub(in crate::core) struct ExplicitIntent {
    /// The queued row it was recorded for. Another row at the head (the user
    /// removed or reordered it) does not inherit it. `None` for an intent
    /// recorded before rows were bound.
    pub(in crate::core) row: Option<String>,
    /// The client command id of the held Send now that recorded it. A client
    /// that lost the "waiting" reply replays that command after a reconnect;
    /// it must find the wait, not send the row itself.
    pub(in crate::core) cmid: Option<String>,
    /// Command ids of held Send nows whose intent ended (sent, superseded,
    /// removed). A late replay of one sends nothing. Bounded, persisted.
    settled: VecDeque<String>,
    /// The prompt cmid a held send was dispatched under, and its command id:
    /// a transient refusal that requeues that prompt keeps the command's
    /// intent instead of reporting it handled.
    dispatched: Option<(String, String)>,
    /// The persisted wait is for the execution Machine (see
    /// [`TARGET_WAIT_REASON`]). Kept with the queue so a drain checks it under
    /// the same lock that takes the head; written with the wait itself.
    pub(in crate::core) awaits_target: bool,
    /// A resource wait is persisted, so maintenance will retry the session.
    waiting: bool,
}

impl ExplicitIntent {
    /// The row and command a restored session's persisted wait names.
    pub(in crate::core) fn restored(hub: &Hub, session_id: &str) -> Self {
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
        let settled = hub
            .setting(&held_sends_key(session_id))
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();
        Self {
            row: field("row"),
            cmid: field("cmid"),
            settled,
            dispatched: None,
            awaits_target: field("reason").as_deref() == Some(TARGET_WAIT_REASON),
            waiting: wait.is_some(),
        }
    }

    fn settle(&mut self, cmid: String) {
        self.settled.retain(|settled| settled != &cmid);
        if self.settled.len() >= SETTLED_HELD_SENDS {
            self.settled.pop_front();
        }
        self.settled.push_back(cmid);
    }
}

fn held_sends_key(session_id: &str) -> String {
    format!("{}{session_id}", settings_keys::SESSION_HELD_SENDS_PREFIX)
}

impl Session {
    /// End the explicit send intent. Its held command id is remembered, so a
    /// client replaying that command after a reconnect sends nothing.
    pub(in crate::core) fn end_explicit_intent(&mut self) {
        self.explicit_wait = false;
        self.explicit.row = None;
        if let Some(cmid) = self.explicit.cmid.take() {
            self.explicit.settle(cmid);
        }
    }

    /// The head `row_id` leaves the queue as prompt `dispatch_cmid`; any
    /// dispatch consumes the intent.
    pub(in crate::core) fn dispatch_explicit_intent(&mut self, row_id: &str, dispatch_cmid: &str) {
        let held = (self.explicit_wait && self.explicit.row.as_deref() == Some(row_id))
            .then(|| self.explicit.cmid.clone())
            .flatten();
        self.end_explicit_intent();
        self.explicit.dispatched = held.map(|held| (dispatch_cmid.to_owned(), held));
    }

    /// A reorder, removal or move to drafts that took the held row off the
    /// head ends its intent (a later row may even reuse its id).
    pub(in crate::core) fn end_displaced_explicit_intent(&mut self) {
        if self.explicit_wait
            && self
                .explicit
                .row
                .as_ref()
                .is_some_and(|row| self.queue.first().is_none_or(|head| &head.id != row))
        {
            self.end_explicit_intent();
        }
    }

    /// Whether only a retry that checked the session's eligibility may take
    /// the head: the session waits for its execution Machine, or the head is
    /// a held Send now (an account park or offline target holds it). Any other
    /// automatic drain would take it out only for the dispatcher to refuse it
    /// and requeue it under a new id, which a terminal holding an edit of that
    /// row reads as a sent message and parks as a draft beside the original.
    pub(in crate::core) fn head_awaits_retry(&self) -> bool {
        self.explicit.awaits_target
            || (self.explicit.waiting
                && self.explicit_wait
                && self
                    .queue
                    .first()
                    .is_some_and(|head| self.explicit.row.as_deref() == Some(head.id.as_str())))
    }
}

impl Hub {
    /// Whether `cmid` is a held Send now this Hub already recorded: `Some(true)`
    /// while its intent still waits, `Some(false)` once that intent ended. A
    /// client replays a command whose reply it lost; the replay must not take
    /// the live send path, which would dispatch the row past the wait (and an
    /// edit of it) or send a row the user has since reordered.
    pub fn held_send_replay(&self, session_id: &str, cmid: &str) -> Option<bool> {
        let sessions = self.inner.sessions.lock();
        let s = sessions.get(session_id)?;
        if s.explicit_wait && s.explicit.cmid.as_deref() == Some(cmid) {
            return Some(true);
        }
        let in_flight = s
            .explicit
            .dispatched
            .as_ref()
            .is_some_and(|(_, held)| held == cmid);
        (in_flight || s.explicit.settled.iter().any(|settled| settled == cmid)).then_some(false)
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

    /// Record (or end) explicit send intent for the current head row.
    pub fn set_explicit_wait(&self, session_id: &str, explicit: bool) {
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            s.explicit.dispatched = None;
            s.end_explicit_intent();
            if explicit {
                s.explicit_wait = true;
                s.explicit.row = s.queue.first().map(|head| head.id.clone());
            }
        }
        self.persist_explicit_wait(session_id);
    }

    /// An explicit send `prompt` came back from a transient refusal. Its intent
    /// returns only if that prompt is the head now: another terminal may have
    /// removed or reordered it meanwhile, and the intent never moves to another
    /// row. A held send's command id comes back with it.
    pub fn retain_explicit_intent(&self, session_id: &str, prompt: Option<&str>) {
        {
            let mut sessions = self.inner.sessions.lock();
            let Some(s) = sessions.get_mut(session_id) else {
                return;
            };
            let head = s
                .queue
                .first()
                .filter(|head| prompt.is_some() && head.cmid.as_deref() == prompt)
                .map(|head| head.id.clone());
            let Some(head) = head else {
                return;
            };
            let held = s
                .explicit
                .dispatched
                .take_if(|(dispatched, _)| prompt == Some(dispatched.as_str()))
                .map(|(_, held)| held);
            s.end_explicit_intent();
            s.explicit_wait = true;
            s.explicit.row = Some(head);
            if let Some(held) = held {
                s.explicit.settled.retain(|settled| settled != &held);
                s.explicit.cmid = Some(held);
            }
        }
        self.persist_explicit_wait(session_id);
    }

    /// Whether a recorded explicit intent still waits for its queued row. A
    /// completed dispatch of another prompt must not end that wait.
    pub fn explicit_intent_pending(&self, session_id: &str) -> bool {
        self.inner.sessions.lock().get(session_id).is_some_and(|s| {
            s.explicit_wait
                && s.explicit
                    .row
                    .as_ref()
                    .is_some_and(|row| s.queue.iter().any(|queued| &queued.id == row))
        })
    }

    /// Write the session's explicit intent into its persisted wait, and its
    /// ended held commands beside it; both carry across a Controller restart.
    pub fn persist_explicit_wait(&self, session_id: &str) {
        self.edit_resource_wait(session_id, |wait| wait);
    }

    /// Replace the session's persisted resource wait with `edit(current)`
    /// (`None` ends it), with the intent binding taken from the Hub. Every
    /// writer goes through here, serialized: a reason or binding read before
    /// another writer's change is never written back over it.
    pub fn edit_resource_wait(
        &self,
        session_id: &str,
        edit: impl FnOnce(Option<serde_json::Value>) -> Option<serde_json::Value>,
    ) {
        static PERSIST: parking_lot::Mutex<()> = parking_lot::Mutex::new(());
        let _serial = PERSIST.lock();
        // A session not loaded (yet) has no intent; its wait is still written.
        let (explicit, row, cmid, settled) = self
            .inner
            .sessions
            .lock()
            .get(session_id)
            .map(|s| {
                (
                    s.explicit_wait,
                    s.explicit.row.clone(),
                    s.explicit.cmid.clone(),
                    s.explicit.settled.clone(),
                )
            })
            .unwrap_or_default();
        let settled = serde_json::json!(settled);
        let held_key = held_sends_key(session_id);
        if settled.as_array().is_some_and(|list| !list.is_empty())
            && self.setting(&held_key).as_ref() != Some(&settled)
        {
            self.set_setting(held_key, settled);
        }
        let key = format!(
            "{}{session_id}",
            settings_keys::SESSION_RESOURCE_WAIT_PREFIX
        );
        let current = self.setting(&key);
        let edited = edit(current.clone());
        let awaits_target = edited.as_ref().is_some_and(|wait| {
            wait.get("reason").and_then(serde_json::Value::as_str) == Some(TARGET_WAIT_REASON)
        });
        if let Some(s) = self.inner.sessions.lock().get_mut(session_id) {
            s.explicit.awaits_target = awaits_target;
            s.explicit.waiting = edited.is_some();
        }
        let Some(serde_json::Value::Object(mut wait)) = edited else {
            if current.is_some() {
                self.set_setting(key, serde_json::Value::Null);
            }
            return;
        };
        {
            wait.insert("explicit".to_owned(), serde_json::Value::Bool(explicit));
            for (field, value) in [("row", row), ("cmid", cmid)] {
                match value {
                    Some(value) => {
                        wait.insert(field.to_owned(), serde_json::Value::String(value));
                    }
                    None => {
                        wait.remove(field);
                    }
                }
            }
            let wait = serde_json::Value::Object(wait);
            if current.as_ref() != Some(&wait) {
                self.set_setting(key, wait);
            }
        }
    }
}
