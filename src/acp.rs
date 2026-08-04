//! ACP client backend.
//!
//! cowboy is the ACP *client* (design §2): it drives each agent (the ACP
//! *server*) over stdio. This module is the only place that touches the
//! `agent-client-protocol` crate, so a crate bump is contained here.
//!
//! The crate is built around role-typed connections
//! ([`agent_client_protocol::Client`]/[`Agent`] markers + [`ConnectionTo`]).
//! `connect_with` runs the handshake + command loop in `run_session`; incoming
//! `session/update` notifications and permission requests are handled by the
//! `on_receive_*` closures, which translate each ACP `SessionUpdate` into a
//! normalized [`crate::core::Event`] on the shared [`Hub`]. Commands flow in
//! over a `Send` channel ([`AgentCommand`]).
//!
//! Everything here is `Send`: the crate dispatches handlers and `cx.spawn`ed
//! tasks on its own executor (driven by the `connect_with` future), and those
//! require `Send` futures. The shared `Hub` is already `Arc`-backed; the small
//! per-session client state ([`ClientState`]) uses `Arc` + `Mutex`/atomics.

#![warn(clippy::pedantic)]

use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

// ACP's stable wire schema lives under `schema::v1::`; SDK major versions do
// not change that protocol version. `ProtocolVersion` stays at the
// version-agnostic schema root and the `Agent`/`Client` traits at the crate root.
use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    CancelNotification, ContentBlock, InitializeRequest, LoadSessionRequest, NewSessionRequest,
    PermissionOptionId, PermissionOptionKind, PromptRequest, RequestPermissionOutcome,
    RequestPermissionRequest, RequestPermissionResponse, ResumeSessionRequest,
    SelectedPermissionOutcome, SessionConfigKind, SessionConfigOption, SessionConfigOptionValue,
    SessionConfigSelectOption, SessionConfigSelectOptions, SessionId, SessionModeId,
    SessionNotification, SessionUpdate, SetSessionConfigOptionRequest, SetSessionModeRequest,
};
use agent_client_protocol::{Agent, ByteStreams, Client, ConnectionTo, Error};
use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt as _, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use crate::agent_model::{AUTO_CONTINUE_PREFIX, Event, SCHED_PREFIX, Status, WAKEUP_PREFIX};
use crate::agent_sink::AgentSink;
use crate::cgroup;
use crate::provider::LaunchSpec;

/// Maximum time allowed for each distinct ACP startup phase. The watchdog
/// resets when the agent advances from initialize to session establishment and
/// then startup configuration, so a slow but progressing launch is not charged
/// against one opaque deadline.
pub(crate) const STARTUP_PHASE_TIMEOUT: Duration = Duration::from_mins(1);
const CODEX_FULL_ACCESS_CONFIG_ID: &str = "mode";
const CODEX_FULL_ACCESS_CONFIG_VALUE: &str = "agent-full-access";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StartupPhase {
    Initialize,
    Resume,
    Load,
    New,
    Configure,
    Ready,
}

impl StartupPhase {
    const fn method(self) -> &'static str {
        match self {
            Self::Initialize => "initialize",
            Self::Resume => "session/resume",
            Self::Load => "session/load",
            Self::New => "session/new",
            Self::Configure => "startup configuration",
            Self::Ready => "ready",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResumeMethod {
    Resume,
    Load,
}

const fn select_resume_method(
    agent_can_resume: bool,
    agent_can_load: bool,
) -> Option<ResumeMethod> {
    if agent_can_resume {
        Some(ResumeMethod::Resume)
    } else if agent_can_load {
        Some(ResumeMethod::Load)
    } else {
        None
    }
}

fn startup_full_access_mode(provider_id: &str) -> Option<&'static str> {
    match provider_id {
        "claude-code" => Some("bypassPermissions"),
        "gemini" => Some("yolo"),
        _ => None,
    }
}

#[cfg(test)]
mod startup_mode_tests {
    use super::{
        ResumeMethod, StartupPhase, StartupTimeout, codex_full_access_available,
        codex_full_access_selected, select_resume_method, session_config_value,
        startup_full_access_mode,
    };
    use agent_client_protocol::schema::v1::{
        SessionConfigOption, SessionConfigOptionValue, SessionConfigSelectOption,
    };

    #[test]
    fn providers_use_their_native_full_access_mode() {
        assert_eq!(
            startup_full_access_mode("claude-code"),
            Some("bypassPermissions")
        );
        assert_eq!(startup_full_access_mode("gemini"), Some("yolo"));
        assert_eq!(startup_full_access_mode("codex"), None);
    }

    #[test]
    fn codex_full_access_tracks_the_authoritative_mode() {
        let choices = vec![
            SessionConfigSelectOption::new("agent", "Agent"),
            SessionConfigSelectOption::new("agent-full-access", "Agent (full access)"),
        ];
        let restricted = vec![SessionConfigOption::select(
            "mode",
            "Mode",
            "agent",
            choices.clone(),
        )];
        let full_access = vec![SessionConfigOption::select(
            "mode",
            "Mode",
            "agent-full-access",
            choices,
        )];

        assert!(codex_full_access_available(&restricted));
        assert!(!codex_full_access_selected(&restricted));
        assert!(!codex_full_access_available(&full_access));
        assert!(codex_full_access_selected(&full_access));
    }

    #[test]
    fn config_values_preserve_typed_ids_and_booleans() {
        let selected = session_config_value(&serde_json::json!("agent")).expect("select value");
        assert_eq!(
            selected.as_value_id().map(|value| value.0.as_ref()),
            Some("agent")
        );

        let enabled = session_config_value(&serde_json::json!(true)).expect("boolean value");
        assert_eq!(enabled, SessionConfigOptionValue::boolean(true));
        assert!(session_config_value(&serde_json::json!(42)).is_err());
    }

    #[test]
    fn resume_prefers_no_replay_and_keeps_load_as_compatibility_fallback() {
        assert_eq!(select_resume_method(true, true), Some(ResumeMethod::Resume));
        assert_eq!(
            select_resume_method(true, false),
            Some(ResumeMethod::Resume)
        );
        assert_eq!(select_resume_method(false, true), Some(ResumeMethod::Load));
        assert_eq!(select_resume_method(false, false), None);
    }

    #[test]
    fn only_an_initialize_timeout_is_safe_to_retry() {
        let initialize = StartupTimeout::new(StartupPhase::Initialize);
        let resume = StartupTimeout::new(StartupPhase::Resume);

        assert!(initialize.retryable());
        assert!(!resume.retryable());
        assert_eq!(
            resume.to_string(),
            "agent did not complete ACP session/resume within 60s"
        );
    }
}

/// One ACP startup phase did not complete within [`STARTUP_PHASE_TIMEOUT`].
/// Carried as an `anyhow` cause so [`run_agent_with_sink`] can retry only a
/// pre-initialize adapter stall, never an ambiguous session operation.
#[derive(Debug)]
struct StartupTimeout {
    phase: StartupPhase,
    seconds: u64,
}

impl StartupTimeout {
    const fn new(phase: StartupPhase) -> Self {
        Self {
            phase,
            seconds: STARTUP_PHASE_TIMEOUT.as_secs(),
        }
    }

    const fn retryable(&self) -> bool {
        matches!(self.phase, StartupPhase::Initialize)
    }
}

impl std::fmt::Display for StartupTimeout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "agent did not complete ACP {} within {}s",
            self.phase.method(),
            self.seconds
        )
    }
}

impl std::error::Error for StartupTimeout {}

async fn startup_watchdog(mut phases: watch::Receiver<StartupPhase>) -> StartupTimeout {
    loop {
        let phase = *phases.borrow_and_update();
        if phase == StartupPhase::Ready {
            std::future::pending::<()>().await;
        }

        tokio::select! {
            () = tokio::time::sleep(STARTUP_PHASE_TIMEOUT) => {
                return StartupTimeout::new(phase);
            }
            changed = phases.changed() => {
                if changed.is_err() {
                    std::future::pending::<()>().await;
                }
            }
        }
    }
}

fn codex_full_access_available(options: &[SessionConfigOption]) -> bool {
    options.iter().any(|option| {
        option.id.0.as_ref() == CODEX_FULL_ACCESS_CONFIG_ID
            && match &option.kind {
                SessionConfigKind::Select(select) => {
                    select.current_value.0.as_ref() != CODEX_FULL_ACCESS_CONFIG_VALUE
                        && config_select_options_contain(
                            &select.options,
                            CODEX_FULL_ACCESS_CONFIG_VALUE,
                        )
                }
                #[allow(unreachable_patterns)]
                _ => false,
            }
    })
}

fn codex_full_access_selected(options: &[SessionConfigOption]) -> bool {
    options.iter().any(|option| {
        option.id.0.as_ref() == CODEX_FULL_ACCESS_CONFIG_ID
            && match &option.kind {
                SessionConfigKind::Select(select) => {
                    select.current_value.0.as_ref() == CODEX_FULL_ACCESS_CONFIG_VALUE
                }
                #[allow(unreachable_patterns)]
                _ => false,
            }
    })
}

fn config_select_options_contain(options: &SessionConfigSelectOptions, value: &str) -> bool {
    match options {
        SessionConfigSelectOptions::Ungrouped(options) => options
            .iter()
            .any(|option| option.value.0.as_ref() == value),
        SessionConfigSelectOptions::Grouped(groups) => groups.iter().any(|group| {
            group
                .options
                .iter()
                .any(|option| option.value.0.as_ref() == value)
        }),
        #[allow(unreachable_patterns)]
        _ => false,
    }
}

fn session_config_value(
    value: &serde_json::Value,
) -> std::result::Result<SessionConfigOptionValue, &'static str> {
    match value {
        serde_json::Value::String(value) => Ok(SessionConfigOptionValue::value_id(value.clone())),
        serde_json::Value::Bool(value) => Ok(SessionConfigOptionValue::boolean(*value)),
        _ => Err("configuration values must be a string id or boolean"),
    }
}

async fn set_startup_config_option(
    cx: &ConnectionTo<Agent>,
    session_id: &str,
    acp_id: &SessionId,
    config_id: &str,
    value: &str,
) -> Option<serde_json::Value> {
    let req = SetSessionConfigOptionRequest::new(acp_id.clone(), config_id.to_owned(), value);
    match cx.send_request(req).block_task().await {
        Ok(resp) => match serde_json::to_value(&resp.config_options) {
            Ok(opts) => Some(opts),
            Err(e) => {
                tracing::warn!(
                    session = %session_id,
                    config_id,
                    value,
                    error = %e,
                    "serializing startup config options failed"
                );
                None
            }
        },
        Err(e) => {
            tracing::warn!(
                session = %session_id,
                config_id,
                value,
                error = ?e,
                "setting startup config option failed"
            );
            None
        }
    }
}

/// A command from a client, routed by the supervisor to an agent thread.
#[derive(Debug)]
pub enum AgentCommand {
    /// Send a user turn. The full ACP content array is forwarded to the
    /// upstream agent verbatim — image / audio / resource blocks make it
    /// through (subject to the upstream's own capabilities), not just text.
    /// The `Option<String>` is the originating client's cmid (chat send) used to
    /// tag the user-message echo for optimistic reconcile; None for none.
    Prompt(
        Vec<ContentBlock>,
        Option<String>,
        Option<oneshot::Sender<Result<String, String>>>,
    ),
    /// Cancel the current turn (ACP `session/cancel`).
    Cancel,
    /// Answer a pending permission request (`None` = cancelled / no choice).
    Permission {
        request_id: String,
        option_id: Option<String>,
    },
    /// Set one of the per-session config options the agent advertises
    /// (mode / model / effort / future). Forwarded to the upstream via the
    /// ACP `session/set_config_option` extension method. The agent's
    /// authoritative response carrying the refreshed options is pushed back
    /// into [`Hub`].
    SetConfigOption {
        config_id: String,
        value: serde_json::Value,
    },
}

/// Per-session client state shared by the connection's handler closures and the
/// command loop. All inhabit the crate's single executor, but the crate
/// requires `Send`, so this is `Arc` + `Mutex`/atomics (not `Rc`/`RefCell`).
struct ClientState {
    sink: Arc<dyn AgentSink>,
    session_id: String,
    /// Pending permission requests awaiting a client answer, keyed by request
    /// id. The connection's permission handler inserts a sender; the command
    /// loop resolves exactly one (first-response-wins).
    pending: Mutex<HashMap<String, oneshot::Sender<Option<String>>>>,
    /// Assistant text captured for an internal prompt that requested a direct
    /// completion result. Ordinary UI prompts leave it off.
    capture: Mutex<Option<String>>,
    /// The Codex adapter's authoritative session mode is Full Access. Codex has
    /// occasionally emitted permission requests after `session/load` despite
    /// that mode (`approval_policy=never`); keep those upstream regressions
    /// from blocking an explicitly unrestricted Cowboy session. This is never
    /// enabled for a restricted mode or another provider.
    codex_full_access: AtomicBool,
    /// While `true`, incoming `session/update` notifications are dropped rather
    /// than pushed to the Hub. Set only around a `session/load` resume: the
    /// agent replays the whole prior conversation as updates, but cowboy's own
    /// persisted log is the source of truth and already holds that history —
    /// re-pushing it would duplicate every message. `load_session` is used
    /// purely to re-warm the agent's internal context, not to rebuild ours.
    suppress_updates: AtomicBool,
}

/// Detached-worker entry point. The ACP connection and all pending request
/// futures remain in this process; output crosses only the [`AgentSink`]
/// boundary, which can survive a Cowboy control-plane restart.
pub fn run_agent_with_sink(
    spec: &LaunchSpec,
    session_id: &str,
    cwd: PathBuf,
    resume: Option<String>,
    mut cmd_rx: mpsc::UnboundedReceiver<AgentCommand>,
    sink: &Arc<dyn AgentSink>,
) {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            sink.set_status(session_id, Status::Crashed, Some(format!("runtime: {e}")));
            return;
        }
    };
    // The whole connection (transport, handlers, `cx.spawn`ed tasks, command
    // loop) runs cooperatively inside the single `connect_with` future, so a
    // plain `block_on` suffices — no `LocalSet` needed now that the crate is
    // `Send`-based.
    //
    // Auto-retry once only when the adapter does not answer ACP initialize.
    // This covers a transient launch/runtime stall without replaying an
    // ambiguous session/resume, session/load, or session/new request. The queued
    // prompts are safe across the retry: they live in the Hub queue (pg) until
    // the session reaches `Running`, never in `cmd_rx`, so `cmd_rx` is empty here
    // — we keep it alive across both attempts only so the supervisor's sender
    // stays valid. A SECOND stall is a persistent failure → `Crashed` + the UI's
    // manual Retry (which routes back through revive).
    let result = rt.block_on(async {
        let mut result = agent_main(
            spec,
            session_id,
            cwd.clone(),
            resume.clone(),
            &mut cmd_rx,
            Arc::clone(sink),
        )
        .await;
        let retryable_startup_stall = result
            .as_ref()
            .err()
            .and_then(|e| e.downcast_ref::<StartupTimeout>())
            .is_some_and(StartupTimeout::retryable);
        if retryable_startup_stall {
            tracing::warn!(
                session = session_id,
                "ACP initialize stalled; auto-retrying spawn once"
            );
            // Stay in `Starting` (a spinner), not `Crashed`: this blip is
            // expected to self-heal, so don't flash an error for it.
            sink.set_status(
                session_id,
                Status::Starting,
                Some("agent slow to start — retrying…".to_owned()),
            );
            result = agent_main(spec, session_id, cwd, resume, &mut cmd_rx, Arc::clone(sink)).await;
        }
        result
    });
    match result {
        Ok(()) => sink.set_status(session_id, Status::Exited, None),
        Err(e) => {
            let raw_error = e.to_string();
            let detail = if spec.id == "gemini" {
                crate::provider::gemini::user_facing_startup_error(&raw_error)
                    .unwrap_or(&raw_error)
                    .to_owned()
            } else {
                raw_error.clone()
            };
            tracing::error!(session = session_id, error = %raw_error, "agent session ended with error");
            // Salvage un-consumed prompts. A cold-start / handshake failure returns
            // BEFORE the command loop (`while let Some(cmd) = cmd_rx.recv()`) drains
            // cmd_rx, so a prompt the dispatcher delivered to this (revived) agent is
            // still sitting here un-logged. Without this it dies with the thread —
            // the user's message vanishes from every surface (see
            // `Hub::requeue_prompt`). Put it back on the durable queue so it's
            // visible and re-drains once the session recovers. Cancel/Permission
            // commands are transient and intentionally dropped.
            while let Ok(cmd) = cmd_rx.try_recv() {
                if let AgentCommand::Prompt(blocks, cmid, completion) = cmd {
                    if let Some(tx) = completion {
                        let _ = tx.send(Err(format!("agent failed before prompt: {detail}")));
                        continue;
                    }
                    let content: Vec<serde_json::Value> = blocks
                        .iter()
                        .map(|b| serde_json::to_value(b).unwrap_or(serde_json::Value::Null))
                        .collect();
                    let text = content
                        .iter()
                        .filter_map(|v| v.get("text").and_then(serde_json::Value::as_str))
                        .collect::<Vec<_>>()
                        .join("\n");
                    sink.requeue_prompt(session_id, text, content, cmid);
                }
            }
            sink.set_status(session_id, Status::Crashed, Some(detail));
        }
    }
}

#[allow(clippy::too_many_lines)] // one cohesive spawn + connection + watchdog
async fn agent_main(
    spec: &LaunchSpec,
    session_id: &str,
    cwd: PathBuf,
    resume: Option<String>,
    cmd_rx: &mut mpsc::UnboundedReceiver<AgentCommand>,
    sink: Arc<dyn AgentSink>,
) -> Result<()> {
    let cwd =
        std::path::absolute(&cwd).with_context(|| format!("resolving cwd {}", cwd.display()))?;
    tracing::info!(provider = spec.id, session = session_id, cwd = %cwd.display(), "spawning agent");

    let mut child = Command::new(&spec.command)
        .args(&spec.args)
        .current_dir(&cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawning provider {} ({})", spec.id, spec.command))?;

    // Contain the agent + everything it forks in its own cgroup so a wedged turn's
    // orphaned subprocesses (e.g. a detached `until …; do sleep; done` poll loop)
    // can be reaped wholesale on teardown / recycle. Best-effort: None ⇒ the agent
    // runs uncontained (see crate::cgroup). Done before the agent forks anything.
    let agent_cgroup = child.id().and_then(|pid| {
        let dir = cgroup::create(session_id)?;
        cgroup::add_pid(&dir, pid);
        Some(dir)
    });

    let child_stdin = child.stdin.take().context("child stdin")?;
    let child_stdout = child.stdout.take().context("child stdout")?;
    let child_stderr = child.stderr.take().context("child stderr")?;
    let stderr_session = session_id.to_owned();
    let stderr_provider = spec.id.to_owned();
    let stderr_task = tokio::spawn(async move {
        let mut lines = BufReader::new(child_stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::warn!(
                session = %stderr_session,
                provider = %stderr_provider,
                child = true,
                message = %line,
                "agent stderr"
            );
        }
    });

    // Connect the crate directly to the child's pipes. The 0.4-era custom
    // stdio interceptors are gone: `config_option_update` now decodes natively
    // (handled in the notification closure), and ext methods are sent with
    // their wire name verbatim (no `_`-prefix mangling to undo), so there is
    // nothing left to rewrite on either stream.
    let transport = ByteStreams::new(child_stdin.compat_write(), child_stdout.compat());

    let state = Arc::new(ClientState {
        sink,
        session_id: session_id.to_owned(),
        pending: Mutex::new(HashMap::new()),
        capture: Mutex::new(None),
        codex_full_access: AtomicBool::new(false),
        suppress_updates: AtomicBool::new(false),
    });

    let notif_state = state.clone();
    let perm_state = state.clone();
    let main_state = state.clone();

    // `run_session` advances this marker before every startup request. The
    // watchdog resets its deadline at each transition and pends permanently
    // once the session is Running.
    let (startup_phase, startup_progress) = watch::channel(StartupPhase::Initialize);
    let run_progress = startup_phase.clone();

    let conn = Client
        .builder()
        .name("cowboy")
        .on_receive_notification(
            async move |notif: SessionNotification,
                        _cx: ConnectionTo<Agent>|
                        -> Result<(), Error> {
                handle_session_notification(&notif_state, &notif);
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |req: RequestPermissionRequest,
                        responder,
                        cx: ConnectionTo<Agent>|
                        -> Result<(), Error> {
                // System sessions have no human to answer. Also absorb Codex's
                // known resume regression where it asks despite its
                // authoritative Full Access mode. Restricted user sessions
                // continue through the human permission path below.
                let system_session = perm_state.sink.session_is_system(&perm_state.session_id);
                let codex_full_access = perm_state.codex_full_access.load(Ordering::SeqCst);
                if system_session || codex_full_access {
                    let allow = req
                        .options
                        .iter()
                        .find(|o| matches!(o.kind, PermissionOptionKind::AllowAlways))
                        .or_else(|| {
                            req.options
                                .iter()
                                .find(|o| matches!(o.kind, PermissionOptionKind::AllowOnce))
                        });
                    let outcome = match allow {
                        Some(opt) => {
                            tracing::info!(
                                option = %opt.name,
                                session = %perm_state.session_id,
                                system_session,
                                codex_full_access,
                                "auto-approving permission"
                            );
                            RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
                                opt.option_id.clone(),
                            ))
                        }
                        None => RequestPermissionOutcome::Cancelled,
                    };
                    return responder.respond(RequestPermissionResponse::new(outcome));
                }
                // The SDK exposes the actual JSON-RPC request id. Keep its JSON
                // representation as Cowboy's opaque UI key so numeric `1` and
                // string `"1"` cannot collide and cancellation/response
                // correlation remains tied to the wire request.
                let request_id = serde_json::to_string(responder.id())
                    .unwrap_or_else(|_| responder.id().to_string());
                let tool_call =
                    serde_json::to_value(&req.tool_call).unwrap_or(serde_json::Value::Null);
                let options = serde_json::to_value(&req.options).unwrap_or(serde_json::Value::Null);

                let (tx, rx) = oneshot::channel::<Option<String>>();
                perm_state.pending.lock().insert(request_id.clone(), tx);
                perm_state.sink.push(
                    &perm_state.session_id,
                    Event::PermissionRequest {
                        request_id: request_id.clone(),
                        tool_call,
                        options,
                    },
                );

                // Defer the actual response: blocking the dispatch loop here
                // would stall every other incoming message (e.g. a concurrent
                // cancel) until the user answers. SDK 2 exposes JSON-RPC
                // request cancellation directly, so an upstream cancellation
                // also clears the pending Cowboy prompt immediately.
                let cancellation = responder.cancellation();
                let cancelled_state = Arc::clone(&perm_state);
                let cancelled_request_id = request_id.clone();
                cx.spawn(async move {
                    let chosen = tokio::select! {
                        chosen = rx => chosen.unwrap_or(None),
                        () = cancellation.cancelled() => {
                            cancelled_state.pending.lock().remove(&cancelled_request_id);
                            cancelled_state.sink.push(
                                &cancelled_state.session_id,
                                Event::PermissionResolved {
                                    request_id: cancelled_request_id,
                                    option_id: None,
                                },
                            );
                            responder.respond_with_error(Error::request_cancelled())?;
                            return Ok(());
                        }
                    };
                    let outcome = match chosen {
                        Some(option_id) => RequestPermissionOutcome::Selected(
                            SelectedPermissionOutcome::new(PermissionOptionId::new(option_id)),
                        ),
                        None => RequestPermissionOutcome::Cancelled,
                    };
                    responder.respond(RequestPermissionResponse::new(outcome))?;
                    Ok(())
                })?;
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(transport, async move |cx: ConnectionTo<Agent>| {
            run_session(&main_state, cx, resume, cwd, cmd_rx, spec.id, &run_progress).await
        });

    // Race the connection against the subprocess's OWN exit. The connection
    // future only resolves when `run_session` returns (its cmd channel closed),
    // so it cannot see the agent dying underneath it: if the agent streams a full
    // reply then exits BEFORE returning the turn's stop_reason, the in-flight
    // `prompt()` awaits a response that will never arrive — and a dead pipe does
    // not reliably surface as a request error — so the future hangs forever and
    // the session latches at `Busy`: a perpetual streaming caret + a queue that
    // never drains (the confirmed sess-stuck bug). `child.wait()` is the ground
    // truth the connection can miss; whichever finishes first ends the session,
    // and the hung turn is torn down with the dropped connection future. The Err
    // here lands as `Status::Crashed` in `run_agent` (queue holds; resend
    // revives). `biased` prefers a clean `run_session` return when both are ready.
    //
    // A THIRD ground truth `child.wait()` cannot catch: a live but wedged
    // adapter. The phase-aware watchdog also reports which request actually
    // stalled instead of attributing every startup timeout to process launch.
    let watchdog = startup_watchdog(startup_progress);
    let result = tokio::select! {
        biased;
        r = conn => r.map_err(|e| anyhow::anyhow!("acp connection: {e}")),
        status = child.wait() => Err(anyhow::anyhow!(
            "agent subprocess exited mid-session ({})",
            match status {
                Ok(s) => s.to_string(),
                Err(e) => format!("wait failed: {e}"),
            }
        )),
        timeout = watchdog => Err(anyhow::Error::new(timeout)),
    };

    // Keep the child alive for the whole connection; dropping it here lets the
    // agent see stdin EOF and exit (a no-op if it already exited above).
    drop(child);
    // Reap the whole agent subtree (the agent + any setsid-detached children it
    // leaked) and remove the leaf. Covers EVERY exit path — clean teardown, a
    // crashed/Exited race, or a watchdog hard-recycle that already SIGKILLed it.
    if let Some(dir) = &agent_cgroup {
        cgroup::kill_and_remove(dir);
    }
    stderr_task.abort();
    result
}

/// Translate one incoming agent `SessionUpdate` into a Hub event.
///
/// `config_option_update` is special-cased: rather than surfacing it as a
/// generic timeline update, its `configOptions` array is pushed to the Hub's
/// dedicated config-options channel (which hydrates the composer dropdowns).
/// Usage is kept as ephemeral session metadata. Every remaining variant is
/// passed through as serialized JSON (design §5), so the UI renders message /
/// thought chunks, tool calls, plans, and modes without per-variant re-modelling.
fn handle_session_notification(state: &ClientState, notif: &SessionNotification) {
    // During a `session/load` resume the agent replays prior turns; drop them
    // — cowboy already has this history persisted (see field docs).
    if state.suppress_updates.load(Ordering::SeqCst) {
        return;
    }
    if let SessionUpdate::ConfigOptionUpdate(ref update) = notif.update {
        match serde_json::to_value(&update.config_options) {
            Ok(opts) => state.sink.set_config_options(&state.session_id, opts),
            Err(e) => tracing::warn!(error = %e, "serializing config options"),
        }
        return;
    }
    if let SessionUpdate::AgentMessageChunk(ref chunk) = notif.update
        && let ContentBlock::Text(text) = &chunk.content
        && let Some(capture) = state.capture.lock().as_mut()
    {
        capture.push_str(&text.text);
    }
    if let SessionUpdate::UsageUpdate(ref usage) = notif.update {
        let raw = serde_json::to_value(&notif.update).unwrap_or(serde_json::Value::Null);
        state.sink.set_session_usage(
            &state.session_id,
            crate::agent_model::SessionUsage {
                used: usage.used,
                size: usage.size,
                raw,
                observed_at_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX)),
            },
        );
        return;
    }
    match serde_json::to_value(&notif.update) {
        Ok(update) => {
            // Honor a ScheduleWakeup BEFORE pushing — the event is still stored
            // verbatim (timeline/UI unchanged); this just adds the side effect of
            // actually firing the wakeup, which the ACP runtime otherwise drops.
            maybe_arm_wakeup(state, &update);
            state.sink.push(&state.session_id, Event::Update { update });
        }
        Err(e) => tracing::warn!(error = %e, "serializing session update"),
    }
}

/// If `update` is a `ScheduleWakeup` tool call carrying `rawInput.{prompt,
/// delaySeconds}`, arm cowboy's scheduler. The agent expects its `/loop` runtime
/// to re-invoke it at the scheduled time; under ACP cowboy IS that runtime, so we
/// honor it here (see [`crate::scheduler`]). Only the event that carries the
/// input arms — the bare `tool_call` and the result `tool_call_update` lack it,
/// so this is effectively once per `ScheduleWakeup` call.
fn maybe_arm_wakeup(state: &ClientState, update: &serde_json::Value) {
    if update
        .pointer("/_meta/claudeCode/toolName")
        .and_then(serde_json::Value::as_str)
        != Some("ScheduleWakeup")
    {
        return;
    }
    let prompt = update
        .pointer("/rawInput/prompt")
        .and_then(serde_json::Value::as_str);
    let delay = update
        .pointer("/rawInput/delaySeconds")
        .and_then(serde_json::Value::as_i64);
    if let (Some(prompt), Some(delay)) = (prompt, delay)
        && !prompt.trim().is_empty()
    {
        tracing::info!(session = %state.session_id, delay_s = delay, "scheduler: arming ScheduleWakeup");
        state
            .sink
            .schedule_wakeup(&state.session_id, delay, prompt.to_owned());
    }
}

/// The connection's `main_fn`: run the ACP handshake, then a command loop that
/// drives prompts/cancels/permissions/config changes until the command channel
/// closes (the supervisor dropped the agent).
#[allow(clippy::too_many_lines)] // one cohesive handshake + command loop
async fn run_session(
    state: &Arc<ClientState>,
    cx: ConnectionTo<Agent>,
    resume: Option<String>,
    cwd: PathBuf,
    cmd_rx: &mut mpsc::UnboundedReceiver<AgentCommand>,
    provider_id: &str,
    startup_phase: &watch::Sender<StartupPhase>,
) -> Result<(), Error> {
    let session_id = state.session_id.clone();

    let init = cx
        .send_request(InitializeRequest::new(ProtocolVersion::V1))
        .block_task()
        .await?;
    let agent_can_load = init.agent_capabilities.load_session;
    let agent_can_resume = init
        .agent_capabilities
        .session_capabilities
        .resume
        .is_some();
    let resume_method = select_resume_method(agent_can_resume, agent_can_load);

    // Establish the agent session. Prefer `session/resume`: unlike
    // `session/load`, ACP defines it to restore native state without replaying
    // prior messages that Cowboy already persists. Retain `session/load` only
    // for older agents. A requested resume is strict: silently falling back to
    // session/new would preserve the Cowboy row while losing the native thread.
    let mut acp_id: Option<SessionId> = None;
    let mut modes = None;
    // Agents may return their initial config options (mode / model / effort) IN the
    // session-creation response (codex does this) rather than only via a later
    // `config_option_update` notification (claude does that). We capture + surface
    // both, so codex's Model / approval chips render like claude's.
    let mut config_options = None;
    if resume.is_some() && resume_method.is_none() {
        return Err(anyhow::anyhow!(
            "agent supports neither session/resume nor session/load; refusing to replace the existing native thread"
        )
        .into());
    }
    if let Some(resume_id) = resume {
        let resume_id = SessionId::new(resume_id.as_str());
        match resume_method.expect("resume support checked above") {
            ResumeMethod::Resume => {
                startup_phase.send_replace(StartupPhase::Resume);
                match cx
                    .send_request(ResumeSessionRequest::new(resume_id.clone(), cwd.clone()))
                    .block_task()
                    .await
                {
                    Ok(resp) => {
                        tracing::info!(session = %session_id, acp_id = %resume_id.0, "session resumed via session/resume");
                        acp_id = Some(resume_id);
                        modes = resp.modes;
                        config_options = resp.config_options;
                    }
                    Err(e) => {
                        tracing::error!(session = %session_id, error = ?e, "session/resume failed; preserving native thread identity");
                        return Err(e);
                    }
                }
            }
            ResumeMethod::Load => {
                startup_phase.send_replace(StartupPhase::Load);
                state.suppress_updates.store(true, Ordering::SeqCst);
                let loaded = cx
                    .send_request(LoadSessionRequest::new(resume_id.clone(), cwd.clone()))
                    .block_task()
                    .await;
                state.suppress_updates.store(false, Ordering::SeqCst);
                match loaded {
                    Ok(resp) => {
                        tracing::info!(session = %session_id, acp_id = %resume_id.0, "session resumed via session/load compatibility fallback");
                        acp_id = Some(resume_id);
                        modes = resp.modes;
                        config_options = resp.config_options;
                    }
                    Err(e) => {
                        tracing::error!(session = %session_id, error = ?e, "session/load failed; preserving native thread identity");
                        return Err(e);
                    }
                }
            }
        }
    }
    let acp_id = if let Some(id) = acp_id {
        id
    } else {
        startup_phase.send_replace(StartupPhase::New);
        let session = cx
            .send_request(NewSessionRequest::new(cwd.clone()))
            .block_task()
            .await?;
        // Persist the agent's own id so a future revive can resume this exact
        // conversation rather than opening a blank one.
        state
            .sink
            .set_agent_session_id(&session_id, session.session_id.0.to_string());
        tracing::info!(session = %session_id, acp_id = %session.session_id.0, "session created");
        modes = session.modes;
        config_options = session.config_options;
        session.session_id
    };
    startup_phase.send_replace(StartupPhase::Configure);
    if provider_id == "codex" {
        state.codex_full_access.store(
            config_options
                .as_ref()
                .is_some_and(|opts| codex_full_access_selected(opts)),
            Ordering::SeqCst,
        );
    }

    // Codex ACP exposes its approval preset as a config option instead of a
    // session mode. Default new/revived Codex panels to Full Access when the
    // adapter advertises it; a failed set falls back to the adapter default.
    if provider_id == "codex"
        && config_options
            .as_ref()
            .is_some_and(|opts| codex_full_access_available(opts))
        && let Some(updated_options) = set_startup_config_option(
            &cx,
            &session_id,
            &acp_id,
            CODEX_FULL_ACCESS_CONFIG_ID,
            CODEX_FULL_ACCESS_CONFIG_VALUE,
        )
        .await
    {
        tracing::info!(session = %session_id, "codex approval preset -> full access");
        state.codex_full_access.store(true, Ordering::SeqCst);
        state.sink.set_config_options(&session_id, updated_options);
        config_options = None;
    }

    // Open every provider at its own full-access session mode when advertised.
    // Codex exposes this as the config option handled above; Claude calls it
    // `bypassPermissions`, while Gemini calls the equivalent mode `yolo`.
    if let (Some(modes), Some(want)) = (modes.as_ref(), startup_full_access_mode(provider_id)) {
        let has = modes
            .available_modes
            .iter()
            .any(|m| m.id.0.as_ref() == want);
        if has && modes.current_mode_id.0.as_ref() != want {
            let req = SetSessionModeRequest::new(acp_id.clone(), SessionModeId::new(want));
            match cx.send_request(req).block_task().await {
                Ok(_) => {
                    tracing::info!(session = %session_id, mode = want, "startup mode -> full access");
                    // Echo into the timeline so the UI mode chip is up to date
                    // without round-tripping through a session_update.
                    state.sink.push(
                        &session_id,
                        Event::Update {
                            update: serde_json::json!({
                                "sessionUpdate": "current_mode_update",
                                "currentModeId": want,
                            }),
                        },
                    );
                }
                Err(e) => {
                    tracing::warn!(mode = want, error = ?e, "setting full-access startup mode failed");
                }
            }
        }
    }

    // Do not expose Running (which lets the broker drain queued prompts) until
    // the startup permission mode is authoritative.
    state.sink.set_status(&session_id, Status::Running, None);
    // Startup landed — disarm the phase watchdog (see `agent_main`).
    startup_phase.send_replace(StartupPhase::Ready);

    // Surface config options the agent returned IN the session response (codex
    // ships its Model + approval options this way; claude instead emits a later
    // `config_option_update` notification, handled separately). Without this codex
    // sessions showed NO Model/effort chips. The SET path is unchanged — the
    // composer's `set_config_option` already routes to `session/set_config_option`,
    // which codex implements (`set_session_config_option`).
    if let Some(opts) = config_options.filter(|o| !o.is_empty()) {
        match serde_json::to_value(&opts) {
            Ok(v) => state.sink.set_config_options(&session_id, v),
            Err(e) => tracing::warn!(error = %e, "serializing session config_options"),
        }
    }

    // gemini (unlike codex) exposes its APPROVAL options as session MODES
    // (`availableModes` + `session/set_mode`), NOT config_options — so the codex
    // push above renders nothing for it. Translate those modes into a synthetic
    // "mode" select chip (matching Zed, which surfaces `session_modes` as its own
    // selector) so gemini gets an Approval dropdown like the others. Gated to
    // gemini: claude ALSO ships session modes, but advertises its mode as a real
    // `config_option` (via a later notification) that this must not shadow. The
    // chip's SET is routed to `session/set_mode` in the command loop below —
    // gemini implements no `session/set_config_option`. `Some` here also marks the
    // session as mode-via-session-modes for that routing.
    let mode_select: Option<Vec<SessionConfigSelectOption>> = if provider_id == "gemini" {
        modes
            .as_ref()
            .filter(|m| !m.available_modes.is_empty())
            .map(|m| {
                m.available_modes
                    .iter()
                    .map(|md| SessionConfigSelectOption::new(md.id.0.to_string(), md.name.clone()))
                    .collect()
            })
    } else {
        None
    };
    if let (Some(options), Some(m)) = (mode_select.as_ref(), modes.as_ref()) {
        let opt = SessionConfigOption::select(
            "mode",
            "Mode",
            m.current_mode_id.0.to_string(),
            options.clone(),
        );
        match serde_json::to_value([opt]) {
            Ok(v) => state.sink.set_config_options(&session_id, v),
            Err(e) => tracing::warn!(error = %e, "serializing gemini mode chip"),
        }
    }

    // Command loop. Prompts and config changes run as concurrent tasks
    // (`cx.spawn`) so Cancel and Permission answers are still processed while a
    // turn is in flight.
    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            AgentCommand::Prompt(blocks, cmid, completion) => {
                if completion.is_some() {
                    *state.capture.lock() = Some(String::new());
                }
                state.sink.set_status(&session_id, Status::Busy, None);
                // Echo each user content block into the timeline so every
                // client (Web UI, phone, native shell) sees it — the upstream
                // agent may not stream a user_message_chunk back. One Hub event
                // per block so each renders as its own bubble. The FIRST echo
                // carries the originating client's cmid so that client reconciles
                // its optimistic chat bubble by id (the rest are untagged).
                // A daemon-originated turn — an auto-resume continuation (cmid
                // "__cont__…") or a fired ScheduleWakeup ("__wake__…") — is flagged
                // on the echo (persisted in the payload) so the UI renders it as a
                // distinct "↻ resumed turn" note: it isn't something the user
                // typed, so it must never look like a user bubble (e.g. a wakeup
                // re-issues a self-check prompt the user never sent).
                let auto_resumed = cmid.as_deref().is_some_and(|c| {
                    c.starts_with(AUTO_CONTINUE_PREFIX)
                        || c.starts_with(WAKEUP_PREFIX)
                        || c.starts_with(SCHED_PREFIX)
                });
                for (i, block) in blocks.iter().enumerate() {
                    let content = serde_json::to_value(block).unwrap_or(serde_json::Value::Null);
                    let tag = if i == 0 { cmid.clone() } else { None };
                    let mut update = serde_json::json!({
                        "sessionUpdate": "user_message_chunk",
                        "content": content,
                    });
                    if auto_resumed {
                        update["autoResumed"] = serde_json::Value::Bool(true);
                    }
                    state
                        .sink
                        .push_tagged(&session_id, Event::Update { update }, tag);
                }
                let cx = cx.clone();
                let sink = Arc::clone(&state.sink);
                let sid = session_id.clone();
                let acp = acp_id.clone();
                let state = Arc::clone(state);
                cx.clone().spawn(async move {
                    match cx
                        .send_request(PromptRequest::new(acp, blocks))
                        .block_task()
                        .await
                    {
                        Ok(r) => {
                            if let Some(tx) = completion {
                                let text = state.capture.lock().take().unwrap_or_default();
                                let _ = tx.send(Ok(text));
                            }
                            // Turn completed — including a `Cancelled` from the user's manual
                            // Stop or a force-push (an Ok we WANT to drain). Going Running lets
                            // the auto-drain send the next queued prompt.
                            sink.push(
                                &sid,
                                Event::TurnEnd {
                                    stop_reason: format!("{:?}", r.stop_reason),
                                },
                            );
                            sink.set_status(&sid, Status::Running, None);
                        }
                        Err(e) => {
                            if let Some(tx) = completion {
                                state.capture.lock().take();
                                let _ = tx.send(Err(e.to_string()));
                            }
                            // The prompt FAILED — agent/connection error, INCLUDING the agent
                            // subprocess dying mid-turn (surfaced by agent_main's `child.wait()`
                            // race, the one auto-recovery we keep: process death is unambiguous).
                            // Mark Crashed so the queue holds; a resend/open revives.
                            //
                            // We deliberately do NOT auto-detect a live-but-silent wedge: idle
                            // time can't tell a slow turn from a stuck one (Zed, the ACP author,
                            // reaches the same conclusion). The UI surfaces silence as a
                            // "waiting Xm" indicator and the user recovers MANUALLY via Stop
                            // (→ Cancel → the agent yields here as an Ok). No auto-kill.
                            sink.push(
                                &sid,
                                Event::TurnEnd {
                                    stop_reason: format!("error: {e}"),
                                },
                            );
                            sink.set_status(&sid, Status::Crashed, Some(e.to_string()));
                        }
                    }
                    Ok(())
                })?;
            }
            AgentCommand::Cancel => {
                let _ = cx.send_notification(CancelNotification::new(acp_id.clone()));
            }
            AgentCommand::Permission {
                request_id,
                option_id,
            } => {
                if let Some(tx) = state.pending.lock().remove(&request_id) {
                    let _ = tx.send(option_id.clone());
                }
                state.sink.push(
                    &session_id,
                    Event::PermissionResolved {
                        request_id,
                        option_id,
                    },
                );
            }
            AgentCommand::SetConfigOption { config_id, value }
                if config_id == "mode" && mode_select.is_some() =>
            {
                // gemini's synthesized "mode" chip maps to ACP `session/set_mode` —
                // it implements no `session/set_config_option` (it never advertised
                // config options; cowboy built this chip from its session modes).
                let Some(mode_id) = value.as_str().map(str::to_owned) else {
                    tracing::warn!(?value, "set mode: non-string value");
                    continue;
                };
                let cx = cx.clone();
                let sink = Arc::clone(&state.sink);
                let sid = session_id.clone();
                let acp = acp_id.clone();
                let options = mode_select.clone().unwrap_or_default();
                cx.clone().spawn(async move {
                    let req = SetSessionModeRequest::new(acp, SessionModeId::new(mode_id.clone()));
                    match cx.send_request(req).block_task().await {
                        Ok(_) => {
                            // Re-push the chip with the new current so the dropdown
                            // sticks (gemini emits no current_mode_update for an
                            // explicit set).
                            let opt = SessionConfigOption::select("mode", "Mode", mode_id, options);
                            match serde_json::to_value([opt]) {
                                Ok(v) => sink.set_config_options(&sid, v),
                                Err(e) => {
                                    tracing::warn!(error = %e, "re-serializing gemini mode chip");
                                }
                            }
                        }
                        Err(e) => sink.broadcast_error(Some(sid.clone()), format!("set mode: {e}")),
                    }
                    Ok(())
                })?;
            }
            AgentCommand::SetConfigOption { config_id, value } => {
                // claude-agent-acp ≥ 0.31 handles mode / model / effort all
                // through the same `session/set_config_option` request. The
                // agent acks with the refreshed `configOptions` array; pushing
                // it back into Hub keeps the composer dropdowns in sync even
                // when the upstream chose a different value than we asked for
                // (e.g. `model=default` resets effort to its model's default).
                let cx = cx.clone();
                let sink = Arc::clone(&state.sink);
                let sid = session_id.clone();
                let acp = acp_id.clone();
                let state = Arc::clone(state);
                cx.clone().spawn(async move {
                    let config_value = match session_config_value(&value) {
                        Ok(value) => value,
                        Err(e) => {
                            sink.broadcast_error(
                                Some(sid.clone()),
                                format!("set {config_id}: {e}"),
                            );
                            return Ok(());
                        }
                    };
                    let req =
                        SetSessionConfigOptionRequest::new(acp, config_id.clone(), config_value);
                    match cx.send_request(req).block_task().await {
                        Ok(response) => {
                            if config_id == CODEX_FULL_ACCESS_CONFIG_ID {
                                let selected = codex_full_access_selected(&response.config_options);
                                state.codex_full_access.store(selected, Ordering::SeqCst);
                            }
                            match serde_json::to_value(response.config_options) {
                                Ok(options) => sink.set_config_options(&sid, options),
                                Err(e) => {
                                    tracing::warn!(
                                        error = %e,
                                        "serializing set config response"
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            sink.broadcast_error(
                                Some(sid.clone()),
                                format!("set {config_id}: {e}"),
                            );
                        }
                    }
                    Ok(())
                })?;
            }
        }
    }
    Ok(())
}

/// Spawn `spec`'s adapter, run the full ACP handshake, send one `prompt` in a
/// fresh session under `cwd`, and stream updates to stdout. Used by the
/// `try-agent` debug command to verify a provider end-to-end. Auto-approves the
/// first allow-style permission option.
pub async fn run_oneshot(spec: &LaunchSpec, cwd: PathBuf, prompt: String) -> Result<()> {
    let cwd =
        std::path::absolute(&cwd).with_context(|| format!("resolving cwd {}", cwd.display()))?;
    tracing::info!(provider = spec.id, cwd = %cwd.display(), "spawning agent");

    let mut child = Command::new(&spec.command)
        .args(&spec.args)
        .current_dir(&cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("spawning provider {} ({})", spec.id, spec.command))?;

    let child_stdin = child.stdin.take().context("child stdin")?;
    let child_stdout = child.stdout.take().context("child stdout")?;
    let transport = ByteStreams::new(child_stdin.compat_write(), child_stdout.compat());

    let result = Client
        .builder()
        .name("cowboy-oneshot")
        .on_receive_notification(
            async move |notif: SessionNotification,
                        _cx: ConnectionTo<Agent>|
                        -> Result<(), Error> {
                use std::io::Write as _;
                match notif.update {
                    SessionUpdate::AgentMessageChunk(chunk)
                    | SessionUpdate::AgentThoughtChunk(chunk) => {
                        if let ContentBlock::Text(t) = chunk.content {
                            print!("{}", t.text);
                            let _ = std::io::stdout().flush();
                        }
                    }
                    SessionUpdate::ToolCall(tc) => eprintln!("\n[tool-call] {}", tc.title),
                    other => tracing::debug!(?other, "session update"),
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |req: RequestPermissionRequest,
                        responder,
                        _cx: ConnectionTo<Agent>|
                        -> Result<(), Error> {
                let allow = req.options.iter().find(|o| {
                    matches!(
                        o.kind,
                        PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways
                    )
                });
                let outcome = match allow {
                    Some(opt) => {
                        tracing::info!(option = %opt.name, "auto-approving permission (try-agent)");
                        RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
                            opt.option_id.clone(),
                        ))
                    }
                    None => RequestPermissionOutcome::Cancelled,
                };
                responder.respond(RequestPermissionResponse::new(outcome))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(transport, async move |cx: ConnectionTo<Agent>| {
            cx.send_request(InitializeRequest::new(ProtocolVersion::V1))
                .block_task()
                .await?;

            let session = cx
                .send_request(NewSessionRequest::new(cwd.clone()))
                .block_task()
                .await?;
            tracing::info!(session_id = %session.session_id.0, "session created");

            let resp = cx
                .send_request(PromptRequest::new(
                    session.session_id,
                    vec![ContentBlock::from(prompt)],
                ))
                .block_task()
                .await?;

            println!("\n--- stop: {:?} ---", resp.stop_reason);
            Ok(())
        })
        .await;

    drop(child);
    result.map_err(|e| anyhow::anyhow!("acp connection: {e}"))
}
