//! Session completion notices (docs/agent-api.md, "Session completion
//! notices"). A session that queues a prompt into another session with
//! `notify:true` gets one Cowboy notice when that prompt's turn ends, so it
//! can end its own turn instead of polling. A human-paced sender (Antigravity)
//! receives the notice as a draft plus a task notification; a person sends it.
//!
//! The watch is Controller-process state: a restart loses pending watches, and
//! `GET /api/sessions/{id}/prompts/{prompt}` (`cowboy session wait`) remains
//! the authority, as `calls.inspect` is for managed calls.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::*;
use crate::core::PromptProgress;

const POLL: Duration = Duration::from_secs(2);
/// A watch older than this stops silently; the sender can still read state.
const MAX_WATCH: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_WATCHES: usize = 256;
const EXCERPT_CHARS: usize = 1200;

static WATCHES: AtomicUsize = AtomicUsize::new(0);

/// One admitted watch slot, released when its watch ends or is never started.
pub(super) struct Slot(());

impl Drop for Slot {
    fn drop(&mut self) {
        WATCHES.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Remove `notify` from a `queue.submit` input and admit it. Only a
/// next-turn prompt to another session can be watched: an interrupt or steer
/// has no single turn of its own, and a session cannot wait on itself.
pub(super) fn take_notify(
    name: &str,
    input: &mut Value,
    sender: &str,
) -> Result<Option<Slot>, &'static str> {
    if name != "queue.submit" {
        return Ok(None);
    }
    let Some(notify) = input.as_object_mut().and_then(|map| map.remove("notify")) else {
        return Ok(None);
    };
    if !notify.as_bool().ok_or("invalid_argument")? {
        return Ok(None);
    }
    if input["session_id"] == sender {
        return Err("notify_requires_another_session");
    }
    if input["force"] == true || input["delivery"].as_str().is_some_and(|d| d != "next_turn") {
        return Err("notify_requires_next_turn");
    }
    if WATCHES.fetch_add(1, Ordering::AcqRel) >= MAX_WATCHES {
        WATCHES.fetch_sub(1, Ordering::AcqRel);
        return Err("capacity");
    }
    Ok(Some(Slot(())))
}

/// After an accepted `queue.submit`, start watching `prompt` (its queue id)
/// and name it in the receipt for `cowboy session wait`.
pub(super) fn after_command(
    state: &Arc<AppState>,
    slot: Option<Slot>,
    (sender, name, input): (&str, &str, &Value),
    prompt: &str,
    mut receipt: Value,
) -> Value {
    let Some(target) = input["session_id"].as_str() else {
        return receipt;
    };
    if name != "queue.submit" || receipt["status"] != "accepted" {
        return receipt;
    }
    receipt["prompt"] = json!(prompt);
    let wait = format!("cowboy session wait {target} {prompt}");
    receipt["next"] = json!(next_step(
        state.hub.is_human_paced(sender),
        slot.is_some(),
        &wait
    ));
    if let Some(slot) = slot {
        receipt["notify"] = json!(true);
        tokio::spawn(watch(
            Arc::clone(state),
            slot,
            sender.to_owned(),
            target.to_owned(),
            prompt.to_owned(),
        ));
    }
    receipt
}

/// What the sender should do next. A human-paced sender cannot be woken
/// by Cowboy, so it keeps its own turn open with the blocking wait.
fn next_step(human_paced: bool, notify: bool, wait: &str) -> String {
    match (human_paced, notify) {
        (true, _) => format!(
            "Cowboy cannot start your next turn. To continue when it finishes, stay in this \
             turn: RunCommand `{wait}` with wait_seconds 600, repeating while state is \
             running.{}",
            if notify {
                " If you end your turn instead, the notice waits as a draft the owner sends."
            } else {
                ""
            }
        ),
        (false, true) => {
            "End your turn; Cowboy posts a notice into this session when that turn ends.".into()
        }
        (false, false) => format!(
            "To act on its result, run `{wait}`, or send with notify:true and end your turn."
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Ended(Status),
    Withdrawn,
    Gone,
}

async fn watch(state: Arc<AppState>, _slot: Slot, sender: String, target: String, prompt: String) {
    let started = Instant::now();
    let mut ran = false;
    while started.elapsed() < MAX_WATCH {
        tokio::time::sleep(POLL).await;
        if state.hub.status(&sender).is_none() {
            return;
        }
        let progress = state.hub.prompt_progress(&target, &prompt);
        let outcome = match settle(progress, ran, state.hub.status(&target)) {
            Ok(outcome) => outcome,
            Err(now_ran) => {
                ran = now_ran;
                continue;
            }
        };
        notify(&state, &sender, &target, &prompt, outcome);
        return;
    }
}

/// The watch's decision for one observation: an outcome, or keep waiting
/// (carrying whether the prompt is known to have reached the worker).
fn settle(progress: PromptProgress, ran: bool, status: Option<Status>) -> Result<Outcome, bool> {
    match progress {
        PromptProgress::Gone => Ok(Outcome::Gone),
        PromptProgress::Queued => Err(ran),
        PromptProgress::Running => Err(true),
        PromptProgress::Settled(status) => Ok(Outcome::Ended(status)),
        // Its evidence left the hot window during a long turn: the turn ends
        // when the session stops being busy.
        PromptProgress::Unknown if ran => match status {
            Some(Status::Busy | Status::Starting) => Err(true),
            Some(status) => Ok(Outcome::Ended(status)),
            None => Ok(Outcome::Gone),
        },
        PromptProgress::Unknown => Ok(Outcome::Withdrawn),
    }
}

fn notify(state: &AppState, sender: &str, target: &str, prompt: &str, outcome: Outcome) {
    let title = state
        .hub
        .session_info(target)
        .map(|info| format!(" ({})", info.meta.title.replace(['\n', '\r'], " ")))
        .unwrap_or_default();
    let turn = state
        .hub
        .prompt_echo_seq(target, prompt)
        .and_then(|seq| state.hub.managed_turn(target, seq));
    let text = notice_text(target, &title, prompt, outcome, turn.as_ref());
    let parked = state
        .hub
        .submit_peer_notice(sender, &format!("{target}:{prompt}"), text);
    if parked {
        let category = if outcome == Outcome::Ended(Status::Running) {
            crate::task_notifications::Category::Completion
        } else {
            crate::task_notifications::Category::Failure
        };
        state
            .hub
            .record_occurrence(crate::task_notifications::Occurrence::peer(
                sender, target, prompt, category,
            ));
    }
}

fn notice_text(
    target: &str,
    title: &str,
    prompt: &str,
    outcome: Outcome,
    turn: Option<&crate::core::ManagedTurn>,
) -> String {
    let read = format!(
        "Read it with cowboy_read: cowboy.sessions.history(\"{target}\",{{limit:6,chars:2000}})."
    );
    match outcome {
        Outcome::Ended(Status::Running) => {
            let stop = turn
                .and_then(|turn| turn.stop_reason.as_deref())
                .unwrap_or("unknown");
            let excerpt = turn
                .map(|turn| tail(&turn.final_text))
                .filter(|text| !text.is_empty())
                .map(|text| format!(" Its last reply ends:\n\n{text}\n\n"))
                .unwrap_or_else(|| " ".to_owned());
            format!(
                "[Cowboy] Session {target}{title} finished the turn for your message {prompt}: \
                 {stop}.{excerpt}{read}"
            )
        }
        Outcome::Ended(status) => format!(
            "[Cowboy] Session {target}{title} stopped before finishing your message {prompt}: \
             {}. {read}",
            status_code(status)
        ),
        Outcome::Withdrawn => format!(
            "[Cowboy] Your message {prompt} to session {target}{title} left its queue without \
             running (removed or moved to drafts). {read}"
        ),
        Outcome::Gone => format!(
            "[Cowboy] Session {target}{title} was closed before your message {prompt} finished."
        ),
    }
}

fn status_code(status: Status) -> &'static str {
    match status {
        Status::Starting => "starting",
        Status::Running => "idle",
        Status::Busy => "busy",
        Status::Exited => "exited",
        Status::Crashed => "crashed",
        Status::Interrupted => "interrupted",
    }
}

/// The last `EXCERPT_CHARS` characters, whole characters only.
fn tail(text: &str) -> String {
    let text = text.trim();
    let count = text.chars().count();
    if count <= EXCERPT_CHARS {
        return text.to_owned();
    }
    let tail: String = text.chars().skip(count - EXCERPT_CHARS).collect();
    format!("…{tail}")
}

pub(super) fn routes() -> Router<Arc<AppState>> {
    Router::new().route("/api/sessions/{id}/prompts/{prompt}", get(progress))
}

/// `GET /api/sessions/{id}/prompts/{prompt}`: where one queued prompt stands.
/// Read-only; it never wakes a worker.
pub(super) async fn progress(
    State(state): State<Arc<AppState>>,
    Path((session_id, prompt)): Path<(String, String)>,
) -> Response {
    let progress = state.hub.prompt_progress(&session_id, &prompt);
    if progress == PromptProgress::Gone {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    }
    let status = state.hub.status(&session_id).map(status_code);
    let mut body = json!({"session_id": session_id, "prompt": prompt,
        "state": progress.code(), "status": status});
    if let PromptProgress::Settled(_) = progress
        && let Some(turn) = state
            .hub
            .prompt_echo_seq(&session_id, &prompt)
            .and_then(|seq| state.hub.managed_turn(&session_id, seq))
    {
        body["stop_reason"] = json!(turn.stop_reason);
        body["reply_tail"] = json!(tail(&turn.final_text));
    }
    Json(body).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notify_is_admitted_only_for_a_next_turn_prompt_to_another_session() {
        let take = |input: Value| {
            let mut input = input;
            take_notify("queue.submit", &mut input, "me").map(|slot| (slot.is_some(), input))
        };
        let (watched, input) = take(json!({"session_id":"peer","text":"x","notify":true})).unwrap();
        assert!(watched);
        assert!(
            input.get("notify").is_none(),
            "the command never sees notify"
        );
        assert!(
            !take(json!({"session_id":"peer","text":"x","notify":false}))
                .unwrap()
                .0
        );
        assert!(!take(json!({"session_id":"peer","text":"x"})).unwrap().0);
        assert_eq!(
            take(json!({"session_id":"me","text":"x","notify":true})).err(),
            Some("notify_requires_another_session")
        );
        for refused in [
            json!({"session_id":"peer","text":"x","notify":true,"force":true}),
            json!({"session_id":"peer","text":"x","notify":true,"delivery":"interrupt"}),
            json!({"session_id":"peer","text":"x","notify":true,"delivery":"current_turn"}),
        ] {
            assert_eq!(take(refused).err(), Some("notify_requires_next_turn"));
        }
        assert_eq!(
            take(json!({"session_id":"peer","text":"x","notify":"yes"})).err(),
            Some("invalid_argument")
        );
        let mut other = json!({"session_id":"peer","notify":true});
        assert!(
            take_notify("drafts.add", &mut other, "me")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_human_paced_sender_is_told_to_wait_inside_its_turn() {
        let wait = "cowboy session wait sess-2 op-1";
        assert!(
            next_step(true, false, wait).contains("RunCommand `cowboy session wait sess-2 op-1`")
        );
        assert!(next_step(true, true, wait).contains("draft the owner sends"));
        assert!(next_step(false, true, wait).starts_with("End your turn"));
        assert!(next_step(false, false, wait).contains(wait));
    }

    #[test]
    fn a_watch_waits_for_the_turn_and_tells_how_it_ended() {
        use PromptProgress as P;
        assert_eq!(settle(P::Queued, false, Some(Status::Busy)), Err(false));
        assert_eq!(settle(P::Running, false, Some(Status::Busy)), Err(true));
        assert_eq!(
            settle(P::Settled(Status::Running), true, Some(Status::Running)),
            Ok(Outcome::Ended(Status::Running))
        );
        assert_eq!(
            settle(P::Settled(Status::Crashed), true, Some(Status::Crashed)),
            Ok(Outcome::Ended(Status::Crashed))
        );
        // A long turn can push the prompt's evidence out of the hot window.
        assert_eq!(settle(P::Unknown, true, Some(Status::Busy)), Err(true));
        assert_eq!(
            settle(P::Unknown, true, Some(Status::Running)),
            Ok(Outcome::Ended(Status::Running))
        );
        assert_eq!(
            settle(P::Unknown, false, Some(Status::Running)),
            Ok(Outcome::Withdrawn)
        );
        assert_eq!(settle(P::Gone, true, None), Ok(Outcome::Gone));
    }

    #[test]
    fn the_notice_names_the_session_the_prompt_and_how_to_read_it() {
        let turn = crate::core::ManagedTurn {
            prompted: true,
            stop_reason: Some("end_turn".into()),
            final_text: "x".repeat(EXCERPT_CHARS + 10),
            generated_images: Vec::new(),
            usage: None,
            status: Status::Running,
        };
        let text = notice_text(
            "sess-2",
            " (Land it)",
            "op-1",
            Outcome::Ended(Status::Running),
            Some(&turn),
        );
        assert!(text.starts_with(
            "[Cowboy] Session sess-2 (Land it) finished the turn for your message op-1: end_turn."
        ));
        assert!(text.contains(&format!("…{}", "x".repeat(EXCERPT_CHARS))));
        assert!(text.ends_with("cowboy.sessions.history(\"sess-2\",{limit:6,chars:2000})."));
        let crashed = notice_text("sess-2", "", "op-1", Outcome::Ended(Status::Crashed), None);
        assert!(crashed.contains("stopped before finishing your message op-1: crashed"));
    }
}
