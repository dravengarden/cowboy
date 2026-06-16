//! ACP client backend.
//!
//! cowboy is the ACP *client* (design §2): it drives each agent (the ACP
//! *server*) over stdio. This module is the only place that touches the
//! `agent-client-protocol` crate, so a crate bump is contained here.
//!
//! The crate (0.14) is built around role-typed connections
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

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use parking_lot::Mutex;

use agent_client_protocol::schema::{
    CancelNotification, ClientRequest, ContentBlock, ExtRequest, InitializeRequest,
    LoadSessionRequest, NewSessionRequest, PermissionOptionId, PermissionOptionKind, PromptRequest,
    ProtocolVersion, RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    SelectedPermissionOutcome, SessionConfigOption, SessionConfigSelectOption, SessionId,
    SessionModeId, SessionNotification, SessionUpdate, SetSessionModeRequest,
};
use agent_client_protocol::{Agent, ByteStreams, Client, ConnectionTo, Error};
use anyhow::{Context, Result};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use crate::cgroup;
use crate::core::{Event, Hub, Status};
use crate::provider::LaunchSpec;

/// How often the prompt watchdog re-checks a `Busy` turn for a wedge.
const WATCHDOG_TICK: std::time::Duration = std::time::Duration::from_secs(15);
/// Idle span (no agent-streamed update) past which a `Busy` turn with no open
/// tool and no pending permission is treated as wedged and force-ended. This is
/// the "agent stays alive but never returns the turn's `stop_reason`" case that
/// `d6ee0ca`'s subprocess-death recovery explicitly left out of scope. Generous
/// on purpose: the open-tool guard already excludes long silent tool calls, so
/// this only needs to outlast a non-streaming think on a provider that doesn't
/// stream partial messages — false-ending a live turn is worse than waiting.
const WATCHDOG_IDLE: std::time::Duration = std::time::Duration::from_secs(300);
/// Consecutive watchdog fires on one agent connection before escalating from a
/// soft turn-abandon to a HARD recycle (SIGKILL the agent's cgroup subtree, let
/// the death trip the `child.wait()` teardown → Crashed → revive). 2 = give the
/// first wedge a soft recovery; a second back-to-back wedge means the agent is
/// persistently broken, so recycle it (and reap its orphaned poll loops).
const WEDGE_RECYCLE_THRESHOLD: u32 = 2;

/// A command from a client, routed by the supervisor to an agent thread.
#[derive(Debug)]
pub enum AgentCommand {
    /// Send a user turn. The full ACP content array is forwarded to the
    /// upstream agent verbatim — image / audio / resource blocks make it
    /// through (subject to the upstream's own capabilities), not just text.
    /// The `Option<String>` is the originating client's cmid (chat send) used to
    /// tag the user-message echo for optimistic reconcile; None for none.
    Prompt(Vec<ContentBlock>, Option<String>),
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
    hub: Hub,
    session_id: String,
    /// Pending permission requests awaiting a client answer, keyed by request
    /// id. The connection's permission handler inserts a sender; the command
    /// loop resolves exactly one (first-response-wins).
    pending: Mutex<HashMap<String, oneshot::Sender<Option<String>>>>,
    /// Monotonic counter for synthesizing permission request ids.
    next_perm: AtomicU64,
    /// While `true`, incoming `session/update` notifications are dropped rather
    /// than pushed to the Hub. Set only around a `session/load` resume: the
    /// agent replays the whole prior conversation as updates, but cowboy's own
    /// persisted log is the source of truth and already holds that history —
    /// re-pushing it would duplicate every message. `load_session` is used
    /// purely to re-warm the agent's internal context, not to rebuild ours.
    suppress_updates: AtomicBool,
    /// This agent's containment cgroup (see [`crate::cgroup`]), or None when the
    /// host can't be contained (no `Delegate=yes`, cgroup-v1, …). The watchdog
    /// hard-recycle SIGKILLs this whole subtree; `agent_main` teardown reaps it.
    cgroup: Option<std::path::PathBuf>,
    /// Consecutive prompt-watchdog fires on THIS connection without an
    /// intervening clean turn end. Drives [`WEDGE_RECYCLE_THRESHOLD`]; reset to 0
    /// on any `Ok` prompt response. Per-connection, so a revive starts fresh.
    wedge_fires: AtomicU32,
}

/// OS-thread entry point: run one agent session to completion on a
/// current-thread runtime. A failure marks the session crashed.
///
/// `resume` carries the downstream agent's own session id when this is a
/// revive of a session whose prior agent process is gone: if the agent
/// supports it, the conversation is re-attached via `session/load` instead of
/// a blank `session/new`. `None` ⇒ a brand-new session.
pub fn run_agent(
    spec: &LaunchSpec,
    session_id: &str,
    cwd: PathBuf,
    resume: Option<String>,
    cmd_rx: mpsc::UnboundedReceiver<AgentCommand>,
    hub: &Hub,
) {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            hub.set_status(session_id, Status::Crashed, Some(format!("runtime: {e}")));
            return;
        }
    };
    // The whole connection (transport, handlers, `cx.spawn`ed tasks, command
    // loop) runs cooperatively inside the single `connect_with` future, so a
    // plain `block_on` suffices — no `LocalSet` needed now that the crate is
    // `Send`-based.
    let result = rt.block_on(agent_main(spec, session_id, cwd, resume, cmd_rx, hub));
    match result {
        Ok(()) => hub.set_status(session_id, Status::Exited, None),
        Err(e) => {
            tracing::error!(session = session_id, error = %e, "agent session ended with error");
            hub.set_status(session_id, Status::Crashed, Some(e.to_string()));
        }
    }
}

async fn agent_main(
    spec: &LaunchSpec,
    session_id: &str,
    cwd: PathBuf,
    resume: Option<String>,
    cmd_rx: mpsc::UnboundedReceiver<AgentCommand>,
    hub: &Hub,
) -> Result<()> {
    let cwd =
        std::path::absolute(&cwd).with_context(|| format!("resolving cwd {}", cwd.display()))?;
    tracing::info!(provider = spec.id, session = session_id, cwd = %cwd.display(), "spawning agent");

    let mut child = Command::new(&spec.command)
        .args(&spec.args)
        .current_dir(&cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
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

    // Connect the crate directly to the child's pipes. The 0.4-era custom
    // stdio interceptors are gone: `config_option_update` now decodes natively
    // (handled in the notification closure), and ext methods are sent with
    // their wire name verbatim (no `_`-prefix mangling to undo), so there is
    // nothing left to rewrite on either stream.
    let transport = ByteStreams::new(child_stdin.compat_write(), child_stdout.compat());

    let state = Arc::new(ClientState {
        hub: hub.clone(),
        session_id: session_id.to_owned(),
        pending: Mutex::new(HashMap::new()),
        next_perm: AtomicU64::new(0),
        suppress_updates: AtomicBool::new(false),
        cgroup: agent_cgroup.clone(),
        wedge_fires: AtomicU32::new(0),
    });

    let notif_state = state.clone();
    let perm_state = state.clone();
    let main_state = state.clone();

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
                // System sessions (machine-driven, immutable, unattended — the
                // memory janitor and any future machine session) have no human
                // to answer an approval. Auto-approve tool calls the way the
                // try-agent path does; without this a system session hangs on
                // its first MCP tool approval forever (the per-tool approval a
                // provider like Codex requests just sits pending). The response
                // is instant, so unlike the human path it needn't be deferred.
                if perm_state.hub.session_is_system(&perm_state.session_id) {
                    let allow = req.options.iter().find(|o| {
                        matches!(
                            o.kind,
                            PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways
                        )
                    });
                    let outcome = match allow {
                        Some(opt) => {
                            tracing::info!(
                                option = %opt.name,
                                session = %perm_state.session_id,
                                "auto-approving permission (system session)"
                            );
                            RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
                                opt.option_id.clone(),
                            ))
                        }
                        None => RequestPermissionOutcome::Cancelled,
                    };
                    return responder.respond(RequestPermissionResponse::new(outcome));
                }
                let n = perm_state.next_perm.fetch_add(1, Ordering::Relaxed);
                let request_id = format!("perm-{n}");
                let tool_call =
                    serde_json::to_value(&req.tool_call).unwrap_or(serde_json::Value::Null);
                let options = serde_json::to_value(&req.options).unwrap_or(serde_json::Value::Null);

                let (tx, rx) = oneshot::channel::<Option<String>>();
                perm_state
                    .pending
                    .lock()
                    .insert(request_id.clone(), tx);
                perm_state.hub.push(
                    &perm_state.session_id,
                    Event::PermissionRequest {
                        request_id,
                        tool_call,
                        options,
                    },
                );

                // Defer the actual response: blocking the dispatch loop here
                // would stall every other incoming message (e.g. a concurrent
                // cancel) until the user answers. The crate keeps the request
                // open until the moved `responder` replies.
                cx.spawn(async move {
                    let chosen = rx.await.unwrap_or(None);
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
            run_session(&main_state, cx, resume, cwd, cmd_rx, spec.id).await
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
    result
}

/// Translate one incoming agent `SessionUpdate` into a Hub event.
///
/// `config_option_update` is special-cased: rather than surfacing it as a
/// generic timeline update, its `configOptions` array is pushed to the Hub's
/// dedicated config-options channel (which hydrates the composer dropdowns).
/// Every other variant — including the new `usage_update` — is passed through
/// as serialized JSON (design §5), so the UI renders message / thought chunks,
/// tool calls, plans, modes, and usage without per-variant re-modelling.
fn handle_session_notification(state: &ClientState, notif: &SessionNotification) {
    // During a `session/load` resume the agent replays prior turns; drop them
    // — cowboy already has this history persisted (see field docs).
    if state.suppress_updates.load(Ordering::SeqCst) {
        return;
    }
    if let SessionUpdate::ConfigOptionUpdate(ref update) = notif.update {
        match serde_json::to_value(&update.config_options) {
            Ok(opts) => state.hub.set_config_options(&state.session_id, opts),
            Err(e) => tracing::warn!(error = %e, "serializing config options"),
        }
        return;
    }
    match serde_json::to_value(&notif.update) {
        Ok(update) => {
            // Honor a ScheduleWakeup BEFORE pushing — the event is still stored
            // verbatim (timeline/UI unchanged); this just adds the side effect of
            // actually firing the wakeup, which the ACP runtime otherwise drops.
            maybe_arm_wakeup(state, &update);
            state.hub.push(&state.session_id, Event::Update { update });
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
    if let (Some(prompt), Some(delay)) = (prompt, delay) {
        if !prompt.trim().is_empty() {
            tracing::info!(session = %state.session_id, delay_s = delay, "scheduler: arming ScheduleWakeup");
            state
                .hub
                .schedule_wakeup(&state.session_id, delay, prompt.to_owned());
        }
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
    mut cmd_rx: mpsc::UnboundedReceiver<AgentCommand>,
    provider_id: &str,
) -> Result<(), Error> {
    let session_id = state.session_id.clone();

    let init = cx
        .send_request(InitializeRequest::new(ProtocolVersion::V1))
        .block_task()
        .await?;
    let agent_can_load = init.agent_capabilities.load_session;

    // Establish the agent session. Resume the agent's own memory via
    // `session/load` when (a) we were handed its prior id and (b) the agent
    // advertises load support; otherwise open a fresh `session/new`. On a
    // fresh start, persist the agent's assigned id so a later revive can
    // resume it. A failed load degrades gracefully to fresh (context lost, but
    // the session stays usable) — matching Zed's always-resumable thread.
    let mut acp_id: Option<SessionId> = None;
    let mut modes = None;
    // Agents may return their initial config options (mode / model / effort) IN the
    // session-creation response (codex does this) rather than only via a later
    // `config_option_update` notification (claude does that). We capture + surface
    // both, so codex's Model / approval chips render like claude's.
    let mut config_options = None;
    if let Some(resume_id) = resume.filter(|_| agent_can_load) {
        let load_id = SessionId::new(resume_id.as_str());
        state.suppress_updates.store(true, Ordering::SeqCst);
        let loaded = cx
            .send_request(LoadSessionRequest::new(load_id.clone(), cwd.clone()))
            .block_task()
            .await;
        state.suppress_updates.store(false, Ordering::SeqCst);
        match loaded {
            Ok(resp) => {
                tracing::info!(session = %session_id, acp_id = %resume_id, "session resumed via session/load");
                acp_id = Some(load_id);
                modes = resp.modes;
                config_options = resp.config_options;
            }
            Err(e) => {
                tracing::warn!(session = %session_id, error = ?e, "session/load failed; starting fresh");
            }
        }
    }
    let acp_id = if let Some(id) = acp_id {
        id
    } else {
        let session = cx
            .send_request(NewSessionRequest::new(cwd.clone()))
            .block_task()
            .await?;
        // Persist the agent's own id so a future revive can resume this exact
        // conversation rather than opening a blank one.
        state
            .hub
            .set_agent_session_id(&session_id, session.session_id.0.to_string());
        tracing::info!(session = %session_id, acp_id = %session.session_id.0, "session created");
        modes = session.modes;
        config_options = session.config_options;
        session.session_id
    };
    state.hub.set_status(&session_id, Status::Running, None);

    // Match Zed's claude-acp default UX: open at `bypassPermissions` if the
    // upstream advertises it. This is what most users want for an agent panel
    // — explicit permission prompts dominate the UX otherwise.
    if let Some(modes) = modes.as_ref() {
        let want = "bypassPermissions";
        let has = modes
            .available_modes
            .iter()
            .any(|m| m.id.0.as_ref() == want);
        if has && modes.current_mode_id.0.as_ref() != want {
            let req = SetSessionModeRequest::new(acp_id.clone(), SessionModeId::new(want));
            match cx.send_request(req).block_task().await {
                Ok(_) => {
                    tracing::info!(session = %session_id, "mode → bypassPermissions");
                    // Echo into the timeline so the UI mode chip is up to date
                    // without round-tripping through a session_update.
                    state.hub.push(
                        &session_id,
                        Event::Update {
                            update: serde_json::json!({
                                "sessionUpdate": "current_mode_update",
                                "currentModeId": want,
                            }),
                        },
                    );
                }
                Err(e) => tracing::warn!(error = ?e, "set_session_mode bypassPermissions failed"),
            }
        }
    }

    // Surface config options the agent returned IN the session response (codex
    // ships its Model + approval options this way; claude instead emits a later
    // `config_option_update` notification, handled separately). Without this codex
    // sessions showed NO Model/effort chips. The SET path is unchanged — the
    // composer's `set_config_option` already routes to `session/set_config_option`,
    // which codex implements (`set_session_config_option`).
    if let Some(opts) = config_options.filter(|o| !o.is_empty()) {
        match serde_json::to_value(&opts) {
            Ok(v) => state.hub.set_config_options(&session_id, v),
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
        let opt =
            SessionConfigOption::select("mode", "Mode", m.current_mode_id.0.to_string(), options.clone());
        match serde_json::to_value([opt]) {
            Ok(v) => state.hub.set_config_options(&session_id, v),
            Err(e) => tracing::warn!(error = %e, "serializing gemini mode chip"),
        }
    }

    // Command loop. Prompts and config changes run as concurrent tasks
    // (`cx.spawn`) so Cancel and Permission answers are still processed while a
    // turn is in flight.
    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            AgentCommand::Prompt(blocks, cmid) => {
                state.hub.set_status(&session_id, Status::Busy, None);
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
                    c.starts_with(crate::core::AUTO_CONTINUE_PREFIX)
                        || c.starts_with(crate::scheduler::WAKEUP_PREFIX)
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
                    state.hub.push_tagged(&session_id, Event::Update { update }, tag);
                }
                let cx = cx.clone();
                let hub = state.hub.clone();
                let sid = session_id.clone();
                let acp = acp_id.clone();
                // Cloned for the watchdog's pending-permission check below — a turn
                // legitimately parked on a human is silent indefinitely and must
                // NOT be force-ended. `pending` is connection-local, so this lives
                // in acp.rs, not behind a Hub method.
                let state = Arc::clone(state);
                cx.clone().spawn(async move {
                    let acp_for_cancel = acp.clone();
                    let prompt = cx.send_request(PromptRequest::new(acp, blocks)).block_task();
                    tokio::pin!(prompt);
                    loop {
                        tokio::select! {
                            // Prefer a real response when both are ready, so the
                            // watchdog never pre-empts a turn that just finished.
                            biased;
                            r = &mut prompt => {
                                match r {
                                    Ok(r) => {
                                        // A clean turn end → the agent isn't wedged; reset the
                                        // consecutive-wedge counter that drives hard recycle.
                                        state.wedge_fires.store(0, Ordering::Relaxed);
                                        // Turn completed normally — including a `Cancelled`
                                        // stop from a force-push (that's an Ok response, and
                                        // we WANT the queue to drain the promoted prompt).
                                        // Going Running lets the auto-drain send the next one.
                                        hub.push(&sid, Event::TurnEnd {
                                            stop_reason: format!("{:?}", r.stop_reason),
                                        });
                                        hub.set_status(&sid, Status::Running, None);
                                    }
                                    Err(e) => {
                                        // The prompt itself FAILED (agent/connection error) —
                                        // the task didn't finish. Do NOT go Running, or the
                                        // auto-drain would fire the next queued prompt into a
                                        // failed turn ("a task wasn't done but the queue
                                        // auto-sent"). Mark crashed so the queue holds; an
                                        // explicit resend / send-now revives and resumes.
                                        hub.push(&sid, Event::TurnEnd {
                                            stop_reason: format!("error: {e}"),
                                        });
                                        hub.set_status(&sid, Status::Crashed, Some(e.to_string()));
                                    }
                                }
                                break;
                            }
                            // Wedged-turn watchdog. The agent can stream a full reply
                            // (text + usage) then never return this turn's stop_reason
                            // while STAYING ALIVE (so d6ee0ca's `child.wait()` race never
                            // fires) — `prompt` then hangs forever and the session latches
                            // `Busy`: perpetual caret, queue never drains. If the turn has
                            // gone silent past WATCHDOG_IDLE with no open tool and no
                            // pending permission, close it exactly like a normal end
                            // (clears in_flight, runs the judge, drains the queue, clears
                            // the caret). The agent process is left alive — only this turn
                            // is abandoned — so the next prompt reuses the connection.
                            () = tokio::time::sleep(WATCHDOG_TICK) => {
                                if hub.turn_appears_stuck(&sid, WATCHDOG_IDLE)
                                    && state.pending.lock().is_empty()
                                {
                                    let fires = state.wedge_fires.fetch_add(1, Ordering::Relaxed) + 1;
                                    // Escalate: a second back-to-back wedge (no clean turn in
                                    // between) means the agent is persistently stuck, not just
                                    // having one bad turn. HARD-recycle — SIGKILL the whole
                                    // cgroup subtree (agent + leaked poll loops); the death
                                    // trips `child.wait()` in agent_main → Crashed → the
                                    // supervisor revives a fresh agent on the next prompt. Only
                                    // when contained; otherwise fall through to soft recovery.
                                    if fires >= WEDGE_RECYCLE_THRESHOLD {
                                        if let Some(dir) = &state.cgroup {
                                            tracing::warn!(
                                                session = %sid, fires,
                                                "prompt watchdog: recycling persistently-wedged agent (kill cgroup subtree)",
                                            );
                                            cgroup::kill(dir);
                                            break;
                                        }
                                    }
                                    // Soft: abandon just this turn, leave the agent alive. Cancel
                                    // so a late reply is a no-op; dropping `prompt` on break
                                    // tears the orphaned request down.
                                    let _ = cx.send_notification(
                                        CancelNotification::new(acp_for_cancel.clone()),
                                    );
                                    hub.push(&sid, Event::TurnEnd {
                                        stop_reason: "watchdog: agent idle, no turn response".to_owned(),
                                    });
                                    hub.set_status(&sid, Status::Running, None);
                                    tracing::warn!(
                                        session = %sid, fires,
                                        "prompt watchdog: closed a wedged turn (agent alive, no stop_reason)",
                                    );
                                    break;
                                }
                            }
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
                state.hub.push(
                    &session_id,
                    Event::PermissionResolved {
                        request_id,
                        option_id,
                    },
                );
            }
            AgentCommand::SetConfigOption { config_id, value } if config_id == "mode" && mode_select.is_some() => {
                // gemini's synthesized "mode" chip maps to ACP `session/set_mode` —
                // it implements no `session/set_config_option` (it never advertised
                // config options; cowboy built this chip from its session modes).
                let Some(mode_id) = value.as_str().map(str::to_owned) else {
                    tracing::warn!(?value, "set mode: non-string value");
                    continue;
                };
                let cx = cx.clone();
                let hub = state.hub.clone();
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
                                Ok(v) => hub.set_config_options(&sid, v),
                                Err(e) => tracing::warn!(error = %e, "re-serializing gemini mode chip"),
                            }
                        }
                        Err(e) => hub.broadcast_error(Some(sid.clone()), format!("set mode: {e}")),
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
                let hub = state.hub.clone();
                let sid = session_id.clone();
                let acp = acp_id.clone();
                cx.clone().spawn(async move {
                    let params = serde_json::json!({
                        "sessionId": acp.0,
                        "configId": config_id,
                        "value": value,
                    });
                    let params_raw = match serde_json::value::to_raw_value(&params) {
                        Ok(r) => Arc::from(r),
                        Err(e) => {
                            hub.broadcast_error(
                                Some(sid.clone()),
                                format!("encoding setConfigOption params: {e}"),
                            );
                            return Ok(());
                        }
                    };
                    let req = ClientRequest::ExtMethodRequest(ExtRequest::new(
                        "session/set_config_option",
                        params_raw,
                    ));
                    match cx.send_request(req).block_task().await {
                        Ok(val) => {
                            // Response carries `{ configOptions: [...] }`.
                            if let Some(opts) = val.get("configOptions").cloned() {
                                hub.set_config_options(&sid, opts);
                            }
                        }
                        Err(e) => {
                            hub.broadcast_error(Some(sid.clone()), format!("set {config_id}: {e}"));
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
