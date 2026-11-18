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

/// A worker event: a `TurnEnded` retires its prompt and is reported to the
/// listener.
pub(super) fn ended(shared: &Shared, session: &str, event: &RuntimeEvent) {
    if let RuntimeEvent::TurnEnded {
        turn_id,
        stop_reason,
        ..
    } = event
    {
        acknowledge_prompt_finish(shared, session, turn_id);
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
        let key = pending.iter().find_map(|(key, command)| match command {
            CoreCommand::Prompt {
                session_id,
                turn_id,
                ..
            } if session_id == session && turn_id == turn => Some(key.clone()),
            _ => None,
        });
        key.and_then(|key| pending.remove(&key).map(|command| (key, command)))
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
