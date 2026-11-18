//! Terminal records of prompts, correlated to their delivery: the worker's
//! `TurnEnded` names the Controller-issued turn of the `CoreCommand::Prompt`
//! it ends, whose `cmid` is the prompt's delivery ID. The Controller's
//! account controls settle a reserved turn only from the terminal record of
//! exactly its delivery and worker epoch.
use super::{RemoteRuntime, Shared};
use crate::runtime_wire::{CoreCommand, RuntimeEvent};
use std::sync::Arc;

/// One terminal record of a delivered prompt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnTerminal {
    pub session_id: String,
    /// The worker incarnation that ended the turn.
    pub worker_epoch: String,
    /// The prompt's delivery ID; `None` when this runtime holds no pending
    /// prompt of the turn (the record is then not attributable).
    pub cmid: Option<String>,
    pub stop_reason: String,
}

/// Observes every terminal record before it is projected into the Hub.
pub type Listener = Arc<dyn Fn(TurnTerminal) + Send + Sync>;

impl RemoteRuntime {
    pub fn set_turn_terminal_listener(&self, listener: Listener) {
        *self.shared.turn_terminals.lock() = Some(listener);
    }
}

/// A worker event of `worker_epoch`: a `TurnEnded` retires its prompt and is
/// reported to the listener with the prompt's delivery ID.
pub(super) fn ended(shared: &Shared, session: &str, worker_epoch: &str, event: &RuntimeEvent) {
    if let RuntimeEvent::TurnEnded {
        turn_id,
        stop_reason,
        ..
    } = event
    {
        let cmid = finish(shared, session, turn_id);
        notify(shared, session, worker_epoch, cmid, stop_reason);
    }
}

/// Retire the finished prompt's pending command and return its `cmid`.
pub(super) fn finish(shared: &Shared, session: &str, turn: &str) -> Option<String> {
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
    let (key, command) = admitted?;
    shared.sent.lock().remove(&key);
    let CoreCommand::Prompt { trace, cmid, .. } = command else {
        return None;
    };
    if let Some(trace) = trace
        && let Some((_, telemetry)) = shared.telemetry.lock().as_ref()
    {
        telemetry.finish_delivery_trace(&trace, true);
    }
    cmid
}

pub(super) fn notify(
    shared: &Shared,
    session: &str,
    worker_epoch: &str,
    cmid: Option<String>,
    stop_reason: &str,
) {
    let listener = shared.turn_terminals.lock().clone();
    if let Some(listener) = listener {
        listener(TurnTerminal {
            session_id: session.to_owned(),
            worker_epoch: worker_epoch.to_owned(),
            cmid,
            stop_reason: stop_reason.to_owned(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::Hub;

    #[tokio::test]
    async fn a_terminal_record_names_the_delivery_and_worker_epoch_it_ends() {
        let hub = Hub::new();
        let runtime = RemoteRuntime::for_test(hub, Vec::new());
        let seen = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let sink = Arc::clone(&seen);
        runtime.set_turn_terminal_listener(Arc::new(move |terminal| sink.lock().push(terminal)));
        runtime.prompt_traced("s", Vec::new(), Some("c1".into()), None);
        let turn = runtime
            .pending_for_test()
            .into_iter()
            .find_map(|command| match command {
                CoreCommand::Prompt { turn_id, .. } => Some(turn_id),
                _ => None,
            })
            .unwrap();
        // Another session's or turn's end names no delivery.
        assert_eq!(finish(&runtime.shared, "other", &turn), None);
        assert_eq!(finish(&runtime.shared, "s", "turn-unknown"), None);
        let cmid = finish(&runtime.shared, "s", &turn);
        assert_eq!(cmid.as_deref(), Some("c1"));
        // Retired once: a replayed end is no longer attributable.
        assert_eq!(finish(&runtime.shared, "s", &turn), None);
        notify(&runtime.shared, "s", "epoch-1", cmid, "EndTurn");
        assert_eq!(
            *seen.lock(),
            [TurnTerminal {
                session_id: "s".into(),
                worker_epoch: "epoch-1".into(),
                cmid: Some("c1".into()),
                stop_reason: "EndTurn".into(),
            }]
        );
    }
}
