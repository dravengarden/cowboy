//! Turn endings, observed for account controls: the Controller maps a
//! named-only session's ending to its account's state
//! (`docs/antigravity-real-accounts-design.md`, "Revision 5 simplification").
//! An ending is a signal about the account only; nothing is correlated to a
//! delivery, reserved or settled by it.
use super::{RemoteRuntime, Shared};
use crate::runtime_wire::{CoreCommand, RuntimeEvent};
use std::sync::Arc;

/// One turn ending of a session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnEnd {
    pub session_id: String,
    pub stop_reason: String,
}

/// Observes every turn ending before it is projected into the Hub.
pub type Listener = Arc<dyn Fn(TurnEnd) + Send + Sync>;

impl RemoteRuntime {
    pub fn set_turn_end_listener(&self, listener: Listener) {
        *self.shared.turn_ends.lock() = Some(listener);
    }
}

/// A projected worker event: a `TurnEnded` retires its prompt.
pub(super) fn ended(shared: &Shared, session: &str, event: &RuntimeEvent) {
    if let RuntimeEvent::TurnEnded { turn_id, .. } = event {
        acknowledge_prompt_finish(shared, session, turn_id);
    }
}

/// Report a turn ending of an admitted worker epoch to the listener (account
/// signals), whether or not the session projects it now (a reset does not).
pub(super) fn signal(shared: &Shared, session: &str, event: &RuntimeEvent) {
    if let RuntimeEvent::TurnEnded { stop_reason, .. } = event {
        let listener = shared.turn_ends.lock().clone();
        if let Some(listener) = listener {
            listener(TurnEnd {
                session_id: session.to_owned(),
                stop_reason: stop_reason.clone(),
            });
        }
    }
}

// Completion proves admission when the separate CommandAck was lost. Start
// events and active snapshots precede the fallible ACP send and cannot prove it.
fn acknowledge_prompt_finish(shared: &Shared, session: &str, turn: &str) {
    let admitted = {
        let mut pending = shared.pending.lock();
        let found =
            pending
                .iter_with_versions()
                .find_map(|(key, version, command)| match command {
                    CoreCommand::Prompt {
                        session_id,
                        turn_id,
                        ..
                    } if session_id == session && turn_id == turn => Some((key.clone(), version)),
                    _ => None,
                });
        if let Some((key, version)) = found {
            if shared.fenced_commands.lock().contains(&key) {
                shared.wrap_up.record_late_park_resolution(
                    key,
                    version,
                    super::wrap_up::LateParkResolution::Admitted,
                );
                return;
            }
            pending.remove(&key).map(|command| (key, command))
        } else {
            None
        }
    };
    if let Some((key, command)) = admitted {
        shared.sent.lock().remove(&key);
        if let CoreCommand::Prompt {
            trace: Some(trace), ..
        } = command
            && let Some((_, telemetry)) = shared.telemetry.lock().as_ref()
        {
            telemetry.finish_delivery_trace(&trace, true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Hub;

    #[tokio::test]
    async fn a_turn_end_retires_its_prompt_and_reaches_the_listener() {
        let hub = Hub::new();
        let runtime = RemoteRuntime::for_test(hub, Vec::new());
        let seen = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        runtime.set_turn_end_listener(Arc::new(move |end| sink.lock().push(end)));
        runtime.prompt_traced("s", Vec::new(), Some("c1".into()), None);
        let turn = runtime
            .pending_for_test()
            .into_iter()
            .find_map(|command| match command {
                CoreCommand::Prompt { turn_id, .. } => Some(turn_id),
                _ => None,
            })
            .unwrap();
        let event = RuntimeEvent::TurnEnded {
            turn_id: turn,
            stop_reason: "EndTurn".into(),
            usage: None,
        };
        // The listener is told even when the ending is not projected (a
        // reset); retiring the prompt belongs to the projection.
        signal(&runtime.shared, "s", &event);
        assert!(!runtime.pending_for_test().is_empty());
        ended(&runtime.shared, "s", &event);
        assert!(runtime.pending_for_test().is_empty());
        assert_eq!(
            *seen.lock(),
            [TurnEnd {
                session_id: "s".into(),
                stop_reason: "EndTurn".into(),
            }]
        );
    }
}
