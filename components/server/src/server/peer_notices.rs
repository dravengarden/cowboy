//! Session completion notices (docs/agent-api.md, "Session completion
//! notices"). A session that queues a prompt into another session with
//! `notify:true` gets one Cowboy notice when that prompt's own turn ends, so
//! it can end its turn instead of polling. A human-paced sender (Antigravity)
//! receives the notice as a draft plus a task notification; a person sends it.
//!
//! The watch is Controller-process state: a restart loses pending watches, and
//! `GET /api/sessions/{id}/prompts/{prompt}` remains the authority, as
//! `calls.inspect` is for managed calls.

use std::sync::LazyLock;
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
/// Longest server-side wait of one progress request, below the CLI's 60 s
/// HTTP timeout so `cowboy api` can carry it.
const MAX_LONG_POLL_MS: u64 = 50_000;

static WATCHES: AtomicUsize = AtomicUsize::new(0);
/// This Controller process. A replayed receipt from another process no
/// longer has its watch.
static GENERATION: LazyLock<String> = LazyLock::new(|| uuid::Uuid::new_v4().to_string());

/// Agent API commands that would start a turn in their target session.
const DISPATCHING: &[&str] = &[
    "queue.submit",
    "queue.send",
    "queue.force_send",
    "drafts.activate",
    "drafts.activate_all",
    "sessions.retry",
];

/// One admitted watch slot, released when its watch ends or is never started.
pub(super) struct Slot(());

impl Drop for Slot {
    fn drop(&mut self) {
        WATCHES.fetch_sub(1, Ordering::AcqRel);
    }
}

fn rejected(code: &str, next: &str) -> Value {
    json!({"status": "rejected", "error": code, "next": next})
}

/// Admit an Agent API command's dispatch and its `notify` option. `Err` is a
/// definite refusal receipt: nothing was submitted, and the same operation
/// cannot run later. An agent never starts a human-paced (Antigravity) turn:
/// a person sends it (docs/requirements.md, Antigravity boundaries).
pub(super) fn admit(
    hub: &crate::core::Hub,
    name: &str,
    input: &mut Value,
    sender: &str,
) -> Result<Option<Slot>, Value> {
    let target = input["session_id"].as_str().unwrap_or_default().to_owned();
    if DISPATCHING.contains(&name) && hub.is_human_paced(&target) {
        return Err(rejected(
            "human_paced_target",
            "Antigravity runs human-paced: add the message with drafts.add; a person sends it.",
        ));
    }
    if name != "queue.submit" {
        return Ok(None);
    }
    let Some(notify) = input.as_object_mut().and_then(|map| map.remove("notify")) else {
        return Ok(None);
    };
    let Some(notify) = notify.as_bool() else {
        return Err(rejected("invalid_argument", "notify is a boolean."));
    };
    if !notify {
        return Ok(None);
    }
    if target == sender {
        return Err(rejected(
            "notify_requires_another_session",
            "A prompt to this session runs after you end this turn.",
        ));
    }
    if input["force"] == true || input["delivery"].as_str().is_some_and(|d| d != "next_turn") {
        return Err(rejected(
            "notify_requires_next_turn",
            "Send with next_turn delivery to be notified.",
        ));
    }
    let empty = input["text"].as_str().is_none_or(|t| t.trim().is_empty())
        && input["content"].as_array().is_none_or(Vec::is_empty);
    if empty || hub.is_managed_child(&target) {
        return Err(rejected(
            "notify_unavailable",
            "This prompt cannot run as its own turn of that session.",
        ));
    }
    if WATCHES.fetch_add(1, Ordering::AcqRel) >= MAX_WATCHES {
        WATCHES.fetch_sub(1, Ordering::AcqRel);
        return Err(rejected(
            "capacity",
            "Too many pending notices; retry with a new operation later.",
        ));
    }
    Ok(Some(Slot(())))
}

/// After an accepted `queue.submit`, start watching `prompt` (its queue id)
/// and name it in the receipt with the next step.
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
    receipt["target"] = json!(target);
    let human_paced = state.hub.is_human_paced(sender);
    receipt["next"] = json!(next_step(
        human_paced,
        slot.is_some(),
        target == sender,
        &wait_command(target, prompt)
    ));
    if let Some(slot) = slot {
        receipt["notify"] = json!(true);
        receipt["watch"] = json!(*GENERATION);
        let owner = state
            .hub
            .session_info(sender)
            .and_then(|info| info.meta.owner_user_id);
        tokio::spawn(watch(
            Arc::clone(state),
            slot,
            Watch {
                sender: sender.to_owned(),
                owner,
                target: target.to_owned(),
                prompt: prompt.to_owned(),
            },
        ));
    }
    receipt
}

/// A replayed Agent API operation's receipt, or pending when its outcome was
/// never recorded. A `queue.submit` whose watch belonged to an earlier
/// Controller process no longer promises a notice.
pub(super) fn replayed(receipt: Option<Value>, domain_operation: &str) -> Value {
    let Some(mut receipt) = receipt else {
        return json!({"status":"pending","outcome":"unknown","domain_operation":domain_operation});
    };
    if receipt["notify"] == true && receipt["watch"].as_str() != Some(GENERATION.as_str()) {
        receipt["notify"] = json!("lost");
        receipt["next"] = json!(
            match (receipt["target"].as_str(), receipt["prompt"].as_str()) {
                (Some(target), Some(prompt)) => format!(
                    "Cowboy restarted, so no notice will come. Check with: {}",
                    wait_command(target, prompt)
                ),
                _ => "Cowboy restarted, so no notice will come. Read the session.".to_owned(),
            }
        );
    }
    receipt
}

/// A blocking wait that works with any installed `cowboy` CLI: each request
/// waits up to 50 s on the Controller, and the loop stops once the prompt's
/// state is no longer queued or running.
fn wait_command(target: &str, prompt: &str) -> String {
    format!(
        "for i in $(seq 11); do r=$(cowboy api GET '/api/sessions/{target}/prompts/{prompt}?wait_ms=50000') || break; \
         case \"$r\" in *'\"state\":\"queued\"'*|*'\"state\":\"running\"'*) ;; *) break;; esac; done; echo \"$r\""
    )
}

/// A human-paced parent (Antigravity) never receives a call notice as a
/// turn, so an admitted call's receipt tells it to wait inside this turn.
pub(super) fn call_next(hub: &crate::core::Hub, parent: &str, mut value: Value) -> Value {
    let Some(call) = value["call_id"].as_str().map(str::to_owned) else {
        return value;
    };
    if !hub.is_human_paced(parent) || !value["error"].is_null() {
        return value;
    }
    value["next"] = json!(format!(
        "Cowboy cannot start your next turn, so a notice would only reach the owner as a \
         draft. Stay in this turn: RunCommand with wait_seconds 600 and timeout_seconds 600: \
         cowboy call wait {call} --timeout-ms 580000 . Repeat while the state is not \
         terminal (if it reports context_unavailable, call cowboy_read \
         cowboy.calls.wait(\"{call}\",60000) instead), then read \
         cowboy.calls.result(\"{call}\",{{offset:0,chars:4000}}) and continue."
    ));
    value
}

/// What the sender should do next. A human-paced sender cannot be woken
/// by Cowboy, so it keeps its own turn open with the blocking wait.
fn next_step(human_paced: bool, notify: bool, own: bool, wait: &str) -> String {
    if own {
        return "This prompt runs after you end this turn.".into();
    }
    match (human_paced, notify) {
        (true, _) => format!(
            "Cowboy cannot start your next turn. To continue when it finishes, stay in this \
             turn: RunCommand with wait_seconds 600 and timeout_seconds 600: {wait} . Repeat \
             it while the printed state is queued or running.{}",
            if notify {
                " If you end your turn instead, the notice waits as a draft the owner sends."
            } else {
                ""
            }
        ),
        (false, true) => {
            "End your turn; Cowboy posts a notice into this session when that turn ends.".into()
        }
        (false, false) => {
            format!("To act on its result, send with notify:true and end your turn, or run: {wait}")
        }
    }
}

struct Watch {
    sender: String,
    owner: Option<String>,
    target: String,
    prompt: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    Finished {
        stop_reason: String,
        echo_seq: u64,
    },
    Stopped(Status),
    /// It ran, but its result left the bounded hot window.
    Ended,
    Withdrawn,
    Gone,
}

async fn watch(state: Arc<AppState>, _slot: Slot, watch: Watch) {
    let started = Instant::now();
    let mut ran = false;
    while started.elapsed() < MAX_WATCH {
        tokio::time::sleep(POLL).await;
        if state.hub.status(&watch.sender).is_none() {
            return;
        }
        let progress = state.hub.prompt_progress(&watch.target, &watch.prompt);
        match settle(progress, ran, state.hub.status(&watch.target)) {
            Ok(outcome) => return notify(&state, &watch, &outcome),
            Err(now_ran) => ran = now_ran,
        }
    }
}

/// The watch's decision for one observation: an outcome, or keep waiting
/// (carrying whether the current dispatch attempt reached the worker). A
/// requeue starts a new attempt.
fn settle(progress: PromptProgress, ran: bool, status: Option<Status>) -> Result<Outcome, bool> {
    match progress {
        PromptProgress::Gone => Ok(Outcome::Gone),
        PromptProgress::Queued => Err(false),
        PromptProgress::Running => Err(true),
        PromptProgress::Finished {
            stop_reason,
            echo_seq,
        } => Ok(Outcome::Finished {
            stop_reason,
            echo_seq,
        }),
        PromptProgress::Stopped(status) => Ok(Outcome::Stopped(status)),
        // It ran, and its evidence left the hot window during a long turn:
        // the turn ends when the session stops being busy.
        PromptProgress::Unknown if ran => match status {
            Some(Status::Busy | Status::Starting) => Err(true),
            Some(_) => Ok(Outcome::Ended),
            None => Ok(Outcome::Gone),
        },
        // The watch saw it queued (it starts right after admission), so it
        // left the queue without reaching the worker.
        PromptProgress::Unknown => Ok(Outcome::Withdrawn),
    }
}

/// Deliver once, rechecking that the sender still exists under the owner
/// that asked and that the target still belongs to that owner.
fn notify(state: &AppState, watch: &Watch, outcome: &Outcome) {
    let Some(sender) = state.hub.session_info(&watch.sender) else {
        return;
    };
    if sender.meta.closing || sender.meta.owner_user_id != watch.owner {
        return;
    }
    let target = state.hub.session_info(&watch.target);
    if target
        .as_ref()
        .is_some_and(|info| info.meta.owner_user_id != watch.owner)
    {
        return;
    }
    let title = target
        .map(|info| format!(" ({})", info.meta.title.replace(['\n', '\r'], " ")))
        .unwrap_or_default();
    let reply = match outcome {
        Outcome::Finished { echo_seq, .. } => state
            .hub
            .managed_turn(&watch.target, *echo_seq)
            .map(|turn| tail(&turn.final_text)),
        _ => None,
    };
    let text = notice_text(
        &watch.target,
        &title,
        &watch.prompt,
        outcome,
        reply.as_deref(),
    );
    let key = format!("{}:{}", watch.target, watch.prompt);
    if state.hub.submit_peer_notice(&watch.sender, &key, text)
        && let Some(category) = category(outcome)
    {
        state
            .hub
            .record_occurrence(crate::task_notifications::Occurrence::peer(
                &watch.sender,
                &watch.target,
                &watch.prompt,
                category,
            ));
    }
}

/// The task notification for a parked notice, by the prompt's own stop
/// reason. A cancellation, a withdrawal or a deleted peer notify nobody.
fn category(outcome: &Outcome) -> Option<crate::task_notifications::Category> {
    use crate::task_notifications::Category;
    // Workers report ACP (`end_turn`) or SDK (`EndTurn`) spellings.
    let stop = |reason: &str| reason.replace('_', "").to_ascii_lowercase();
    match outcome {
        Outcome::Finished { stop_reason, .. } if stop(stop_reason) == "endturn" => {
            Some(Category::Completion)
        }
        Outcome::Finished { stop_reason, .. } if stop(stop_reason) == "cancelled" => None,
        Outcome::Finished { .. } | Outcome::Stopped(_) => Some(Category::Failure),
        Outcome::Ended => Some(Category::Completion),
        Outcome::Withdrawn | Outcome::Gone => None,
    }
}

fn notice_text(
    target: &str,
    title: &str,
    prompt: &str,
    outcome: &Outcome,
    reply: Option<&str>,
) -> String {
    let read = format!(
        "Read it with cowboy_read: cowboy.sessions.history(\"{target}\",{{limit:6,chars:2000}})."
    );
    match outcome {
        Outcome::Finished { stop_reason, .. } => {
            let excerpt = reply
                .filter(|text| !text.is_empty())
                .map(|text| format!(" Its last reply ends:\n\n{text}\n\n"))
                .unwrap_or_else(|| " ".to_owned());
            format!(
                "[Cowboy] Session {target}{title} finished the turn for your message {prompt}: \
                 {stop_reason}.{excerpt}{read}"
            )
        }
        Outcome::Stopped(status) => format!(
            "[Cowboy] Session {target}{title} stopped before finishing your message {prompt}: \
             {}. {read}",
            status_code(*status)
        ),
        Outcome::Ended => format!(
            "[Cowboy] Session {target}{title} ended the turn for your message {prompt}; its \
             result is no longer in memory. {read}"
        ),
        Outcome::Withdrawn => format!(
            "[Cowboy] Your message {prompt} to session {target}{title} left its queue without \
             running (removed or moved to drafts). {read}"
        ),
        Outcome::Gone => format!(
            "[Cowboy] Session {target}{title} was deleted before your message {prompt} finished."
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

#[derive(serde::Deserialize)]
struct ProgressQuery {
    #[serde(default)]
    wait_ms: u64,
}

/// `GET /api/sessions/{id}/prompts/{prompt}[?wait_ms=N]`: where one queued
/// prompt stands, bound to its own turn. With `wait_ms` (at most 50 s) it
/// answers as soon as the prompt is no longer queued or running. Read-only;
/// it never wakes a worker.
async fn progress(
    State(state): State<Arc<AppState>>,
    Path((session_id, prompt)): Path<(String, String)>,
    Query(query): Query<ProgressQuery>,
) -> Response {
    let deadline = Instant::now() + Duration::from_millis(query.wait_ms.min(MAX_LONG_POLL_MS));
    let mut progress = state.hub.prompt_progress(&session_id, &prompt);
    while progress.pending() && Instant::now() + POLL / 2 < deadline {
        tokio::time::sleep(POLL / 2).await;
        progress = state.hub.prompt_progress(&session_id, &prompt);
    }
    if progress == PromptProgress::Gone {
        return (StatusCode::NOT_FOUND, "unknown session").into_response();
    }
    let status = state.hub.status(&session_id).map(status_code);
    let mut body = json!({"session_id": session_id, "prompt": prompt,
        "state": progress.code(), "status": status});
    match progress {
        PromptProgress::Finished {
            stop_reason,
            echo_seq,
        } => {
            body["stop_reason"] = json!(stop_reason);
            if let Some(turn) = state.hub.managed_turn(&session_id, echo_seq) {
                body["reply_tail"] = json!(tail(&turn.final_text));
            }
        }
        PromptProgress::Stopped(status) => body["stopped"] = json!(status_code(status)),
        _ => {}
    }
    Json(body).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hub() -> crate::core::Hub {
        let hub = crate::core::Hub::new();
        for (id, provider) in [("me", "codex"), ("peer", "codex"), ("agy", "antigravity")] {
            hub.create_local_session(
                id.to_owned(),
                provider.to_owned(),
                "/tmp".to_owned(),
                "t".to_owned(),
                crate::core::SessionOrigin::Web,
                false,
            );
        }
        hub
    }

    #[test]
    fn notify_is_admitted_only_for_a_next_turn_prompt_to_another_session() {
        let hub = hub();
        let take = |name: &str, input: Value| {
            let mut input = input;
            admit(&hub, name, &mut input, "me")
                .map(|slot| (slot.is_some(), input))
                .map_err(|refusal| refusal["error"].as_str().unwrap().to_owned())
        };
        let (watched, input) = take(
            "queue.submit",
            json!({"session_id":"peer","text":"x","notify":true}),
        )
        .unwrap();
        assert!(watched);
        assert!(
            input.get("notify").is_none(),
            "the command never sees notify"
        );
        let plain = json!({"session_id":"peer","text":"x","notify":false});
        assert!(!take("queue.submit", plain).unwrap().0);
        assert!(
            !take("queue.submit", json!({"session_id":"peer","text":"x"}))
                .unwrap()
                .0
        );
        let refused = |name: &str, input: Value| take(name, input).err().unwrap();
        assert_eq!(
            refused(
                "queue.submit",
                json!({"session_id":"me","text":"x","notify":true})
            ),
            "notify_requires_another_session"
        );
        for input in [
            json!({"session_id":"peer","text":"x","notify":true,"force":true}),
            json!({"session_id":"peer","text":"x","notify":true,"delivery":"interrupt"}),
            json!({"session_id":"peer","text":"x","notify":true,"delivery":"current_turn"}),
        ] {
            assert_eq!(refused("queue.submit", input), "notify_requires_next_turn");
        }
        assert_eq!(
            refused(
                "queue.submit",
                json!({"session_id":"peer","text":" ","notify":true})
            ),
            "notify_unavailable"
        );
        assert_eq!(
            refused(
                "queue.submit",
                json!({"session_id":"peer","text":"x","notify":"yes"})
            ),
            "invalid_argument"
        );
        assert!(
            !take("drafts.add", json!({"session_id":"peer","notify":true}))
                .unwrap()
                .0
        );
    }

    #[test]
    fn an_agent_never_starts_a_human_paced_turn() {
        let hub = hub();
        for name in DISPATCHING {
            let mut input = json!({"session_id":"agy","text":"x","id":"d"});
            let refusal = admit(&hub, name, &mut input, "me").err().unwrap();
            assert_eq!(refusal["error"], "human_paced_target", "{name}");
        }
        let mut draft = json!({"session_id":"agy","text":"x"});
        assert!(
            admit(&hub, "drafts.add", &mut draft, "me")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn the_next_step_fits_the_sender() {
        let wait = wait_command("sess-2", "op-1");
        assert!(wait.contains("cowboy api GET '/api/sessions/sess-2/prompts/op-1?wait_ms=50000'"));
        let agy = next_step(true, false, false, &wait);
        assert!(agy.contains("wait_seconds 600 and timeout_seconds 600"));
        assert!(agy.contains("queued or running"));
        assert!(next_step(true, true, false, &wait).contains("draft the owner sends"));
        assert!(next_step(false, true, false, &wait).starts_with("End your turn"));
        assert!(next_step(false, false, false, &wait).contains(&wait));
        assert!(next_step(false, false, true, &wait).contains("after you end this turn"));
    }

    #[test]
    fn a_human_paced_parent_waits_for_its_call_inside_the_turn() {
        let hub = hub();
        let admitted = json!({"state":"queued","call_id":"call-1","error":null});
        let agy = call_next(&hub, "agy", admitted.clone());
        let next = agy["next"].as_str().unwrap();
        assert!(next.contains("cowboy call wait call-1 --timeout-ms 580000"));
        assert!(next.contains("timeout_seconds 600"));
        assert!(next.contains("cowboy.calls.result(\"call-1\""));
        assert_eq!(call_next(&hub, "me", admitted.clone()), admitted);
        let refused = json!({"state":"error","call_id":"call-2","error":{"code":"x"}});
        assert_eq!(call_next(&hub, "agy", refused.clone()), refused);
    }

    #[test]
    fn a_replay_from_another_process_admits_the_watch_is_lost() {
        let current = json!({"status":"accepted","notify":true,"watch":*GENERATION});
        assert_eq!(replayed(Some(current.clone()), "d"), current);
        assert_eq!(replayed(None, "d")["status"], "pending");
        let old = replayed(
            Some(json!({"status":"accepted","notify":true,"watch":"old",
            "target":"sess-2","prompt":"op-1"})),
            "d",
        );
        assert_eq!(old["notify"], "lost");
        assert!(old["next"].as_str().unwrap().contains("prompts/op-1"));
    }

    #[test]
    fn a_watch_follows_one_dispatch_attempt_and_its_own_turn() {
        use PromptProgress as P;
        let finished = || P::Finished {
            stop_reason: "end_turn".into(),
            echo_seq: 4,
        };
        assert_eq!(
            settle(P::Queued, true, Some(Status::Busy)),
            Err(false),
            "a requeue resets"
        );
        assert_eq!(settle(P::Running, false, Some(Status::Busy)), Err(true));
        assert_eq!(
            settle(finished(), true, Some(Status::Crashed)),
            Ok(Outcome::Finished {
                stop_reason: "end_turn".into(),
                echo_seq: 4
            })
        );
        assert_eq!(
            settle(P::Stopped(Status::Crashed), true, Some(Status::Crashed)),
            Ok(Outcome::Stopped(Status::Crashed))
        );
        assert_eq!(settle(P::Unknown, true, Some(Status::Busy)), Err(true));
        assert_eq!(
            settle(P::Unknown, true, Some(Status::Running)),
            Ok(Outcome::Ended)
        );
        assert_eq!(
            settle(P::Unknown, false, Some(Status::Running)),
            Ok(Outcome::Withdrawn)
        );
        assert_eq!(settle(P::Gone, true, None), Ok(Outcome::Gone));
    }

    #[test]
    fn a_parked_notice_notifies_by_the_prompts_own_stop_reason() {
        use crate::task_notifications::Category;
        let finished = |stop: &str| Outcome::Finished {
            stop_reason: stop.into(),
            echo_seq: 1,
        };
        assert_eq!(category(&finished("end_turn")), Some(Category::Completion));
        assert_eq!(category(&finished("EndTurn")), Some(Category::Completion));
        assert_eq!(category(&finished("Cancelled")), None);
        assert_eq!(category(&finished("cancelled")), None);
        assert_eq!(category(&finished("refusal")), Some(Category::Failure));
        assert_eq!(
            category(&Outcome::Stopped(Status::Crashed)),
            Some(Category::Failure)
        );
        assert_eq!(category(&Outcome::Withdrawn), None);
    }

    #[test]
    fn the_notice_names_the_session_the_prompt_and_how_to_read_it() {
        let reply = tail(&"x".repeat(EXCERPT_CHARS + 10));
        let outcome = Outcome::Finished {
            stop_reason: "end_turn".into(),
            echo_seq: 3,
        };
        let text = notice_text("sess-2", " (Land it)", "op-1", &outcome, Some(&reply));
        assert!(text.starts_with(
            "[Cowboy] Session sess-2 (Land it) finished the turn for your message op-1: end_turn."
        ));
        assert!(text.contains(&format!("…{}", "x".repeat(EXCERPT_CHARS))));
        assert!(text.ends_with("cowboy.sessions.history(\"sess-2\",{limit:6,chars:2000})."));
        let crashed = notice_text(
            "sess-2",
            "",
            "op-1",
            &Outcome::Stopped(Status::Crashed),
            None,
        );
        assert!(crashed.contains("stopped before finishing your message op-1: crashed"));
    }
}
