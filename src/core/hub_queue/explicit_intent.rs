//! The explicit send intent a held queue row carries through a resource wait.
use super::*;

/// The resource wait of an admitted prompt whose execution Machine is offline
/// or booting. While it is recorded, only the maintenance retry, which first
/// checks that Machine, delivers the session's queue head.
pub const TARGET_WAIT_REASON: &str = "the execution Machine is not ready";

/// How many ended held-send command ids a session remembers for late replays.
const SETTLED_HELD_SENDS: usize = 16;

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
    /// removed). A late replay of one sends nothing. Bounded, in memory.
    settled: VecDeque<String>,
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
        Self {
            row: field("row"),
            cmid: field("cmid"),
            settled: VecDeque::new(),
        }
    }

    pub(in crate::core) fn settled(&self, cmid: &str) -> bool {
        self.settled.iter().any(|settled| settled == cmid)
    }
}

impl Session {
    /// End the explicit send intent. Its held command id is remembered, so a
    /// client replaying that command after a reconnect sends nothing.
    pub(in crate::core) fn end_explicit_intent(&mut self) {
        self.explicit_wait = false;
        self.explicit.row = None;
        if let Some(cmid) = self.explicit.cmid.take() {
            if self.explicit.settled.len() >= SETTLED_HELD_SENDS {
                self.explicit.settled.pop_front();
            }
            self.explicit.settled.push_back(cmid);
        }
    }
}
