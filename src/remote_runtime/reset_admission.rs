//! A cross-release reset waits, with its stop, for its own MCP launch plan.
//!
//! `reset` declares the replacement adopt-only before the reset stop. During a
//! Provider update that declaration still carries the MCP plan of the release
//! it replaces: the plan for the new release is prepared asynchronously once
//! the session names it. Admission rejected the stale declaration but still
//! sent the stop, so the Machine relaunched the old release from its cached
//! declaration and the Controller declined that worker. Every MCP-bound
//! session stalled about four minutes per automatic update (five times on OVH
//! in a week). The declaration and its stop now wait together, bounded, and
//! leave with the plan of the release they declare.
//!
//! A same-release reload waits the same way. The plan owner is bound to the
//! worker it observed, and a resetting worker is no longer observable, so the
//! reload's own declaration resolves no plan until the recovery pass prepares
//! a fresh owner. Admission denied it while its stop still left, and the
//! Machine relaunched from a cached declaration that MCP launch refuses
//! (2026-11-20, sess-1795152728753 crashed on every reload). A declaration
//! that admission still denies now cancels its reset: the stop never leaves
//! and the running worker is kept.

use super::{CoreCommand, Duration, Frame, HashMap, HashSet, Mutex, Result, Shared, Status};

/// Longer than the MCP launch recovery pass needs (5-second ticks); after it
/// the declaration meets ordinary admission, as before.
const PLAN_WAIT: Duration = Duration::from_secs(60);
const RECHECK: Duration = Duration::from_secs(2);
const CLEARED_CANCEL_DETAIL: &str = "The conversation was cleared, but its fresh agent could not start yet; send a message or Retry to start it";

static WAITING: std::sync::LazyLock<Mutex<HashMap<String, std::time::Instant>>> =
    std::sync::LazyLock::new(Default::default);

/// The command to admit now, or `None` while it waits. `deferred` holds the
/// sessions whose reset declaration waits in this pass (it is ordered first).
pub(super) fn current(
    shared: &Shared,
    key: &str,
    version: u64,
    command: CoreCommand,
    deferred: &mut HashSet<String>,
) -> Option<CoreCommand> {
    // Everything ordered behind a waiting declaration (its reset stop, the
    // configuration replay) waits with it; a cancellation or an ordinary stop
    // still reaches the running worker.
    let urgent = match &command {
        CoreCommand::StopSession { command_id, .. } => !command_id.starts_with("reset-"),
        CoreCommand::Cancel { .. }
        | CoreCommand::CancelSubagent { .. }
        | CoreCommand::QuiesceAccountSession { .. } => true,
        _ => false,
    };
    if !urgent && command.session_id().is_some_and(|id| deferred.contains(id)) {
        return None;
    }
    let mut session = match command {
        CoreCommand::EnsureSession { session }
            if session.adopt_only && shared.resetting.lock().contains(&session.session_id) =>
        {
            session
        }
        command => return Some(command),
    };
    if shared.pending.lock().version(key) != Some(version) {
        deferred.insert(session.session_id);
        return None;
    }
    let id = session.session_id.clone();
    let Some(meta) = shared.hub.session_info(&id).map(|info| info.meta) else {
        return Some(CoreCommand::EnsureSession { session });
    };
    let Ok(plan) = crate::controller_hooks::launch_mcp(&shared.hub, &meta) else {
        let since = *WAITING
            .lock()
            .entry(id.clone())
            .or_insert_with(std::time::Instant::now);
        if since.elapsed() >= PLAN_WAIT {
            WAITING.lock().remove(&id);
            return Some(CoreCommand::EnsureSession { session });
        }
        if deferred.is_empty() {
            let notify = shared.notify.clone();
            tokio::spawn(async move {
                tokio::time::sleep(RECHECK).await;
                let _ = notify.send(());
            });
        }
        tracing::info!(session = %id, "reset declaration waits for its release's MCP launch plan");
        deferred.insert(id);
        return None;
    };
    WAITING.lock().remove(&id);
    // Only the plan of the very release this declaration names, and only if
    // this exact pending version has not been superseded since the snapshot.
    if plan != session.mcp_plan
        && meta.provider_version == session.provider_version
        && meta.provider_generation_digest == session.provider_generation_digest
        && meta.provider_auth_generation == session.provider_auth_generation
        && meta.provider_account_id == session.provider_account_id
    {
        session.mcp_plan = plan;
        if !shared.pending.lock().update_if_version(
            key,
            version,
            CoreCommand::EnsureSession {
                session: session.clone(),
            },
        ) {
            deferred.insert(id);
            return None;
        }
        if let Some(declared) = shared.declarations.lock().get_mut(&id).filter(|declared| {
            declared.provider_version == session.provider_version
                && declared.provider_generation_digest == session.provider_generation_digest
                && declared.provider_auth_generation == session.provider_auth_generation
                && declared.provider_account_id == session.provider_account_id
                && declared.cwd == session.cwd
        }) {
            declared.mcp_plan.clone_from(&session.mcp_plan);
        }
    }
    Some(CoreCommand::EnsureSession { session })
}

/// Admission denied `command` before writing it. Report it like a Machine
/// refusal; a reset's declaration also cancels that reset (see module docs).
pub(super) async fn denied<W: tokio::io::AsyncWrite + Unpin>(
    shared: &Shared,
    writer: &mut W,
    key: String,
    command: &CoreCommand,
) -> Result<()> {
    let Some(session_id) = command.session_id().map(str::to_owned) else {
        return Ok(());
    };
    let reset = matches!(command, CoreCommand::EnsureSession { session } if session.adopt_only)
        && cancel(shared, &session_id);
    let reason = if reset {
        "Worker launch admission denied; the reload was cancelled and the current worker keeps running"
    } else {
        "Worker launch admission denied"
    };
    super::handle_frame(
        shared,
        Frame::CommandAck {
            session_id,
            command_id: key,
            accepted: false,
            reason: Some(reason.into()),
        },
        writer,
    )
    .await
}

/// Withdraw a reset whose declaration was denied before its stop left.
/// Returns false when `session_id` has no such reset in flight.
///
/// Known limit: absence from `sent` is the evidence that the stop never
/// left, and a reconnect clears `sent`. A stop written before a disconnect,
/// whose ACK was lost and whose declaration is denied on replay, is treated
/// as unsent: the staged replacement is discarded and a Clear's session
/// reports Crashed from the cached old worker. That fails closed (Retry starts
/// a fresh worker). Closing it needs connection-independent attempt evidence,
/// such as an attempted-send record that survives reconnect.
fn cancel(shared: &Shared, session_id: &str) -> bool {
    if !shared.resetting.lock().contains(session_id) {
        return false;
    }
    let sent = shared.sent.lock().clone();
    let mut withdrawn_stops = Vec::new();
    shared.pending.lock().retain(|key, command| {
        if let CoreCommand::StopSession {
            session_id: id,
            command_id,
        } = command
            && id == session_id
            && command_id.starts_with("reset-")
            && !sent.contains(key)
        {
            withdrawn_stops.push(command_id.clone());
            false
        } else {
            true
        }
    });
    if withdrawn_stops.is_empty() {
        // Its stop is already with the Machine, which owns the outcome.
        return false;
    }
    WAITING.lock().remove(session_id);
    shared.resetting.lock().remove(session_id);
    shared.active_reset_commands.lock().remove(session_id);
    shared.reset_replacements.lock().remove(session_id);
    // Round 3 #7: if this cancelled reload had refreshed a recovery fence to
    // `AwaitingReplacement`, no CommandAck for the withdrawn stop will ever
    // arrive; transition that exact fence to `Failed` and return held prompts
    // through conditional requeue instead of stranding them forever.
    let cancel_reason = "Worker launch admission denied; the reload was cancelled and the current worker keeps running";
    for stop_id in &withdrawn_stops {
        let _ = super::notice_recovery::fail_rejected_recovery_command(
            shared,
            session_id,
            stop_id,
            cancel_reason,
        );
    }
    let kept = shared.workers.lock().get(session_id).cloned();
    // A Clear fenced the kept worker's context for good: it fails closed, and
    // the next prompt or Retry recycles the session into a fresh worker.
    let cleared = kept.as_ref().is_some_and(|worker| {
        !shared.hub.admit_runtime_epoch(
            session_id,
            &worker.worker_epoch,
            worker.agent_session_id.as_deref(),
        )
    });
    let (status, detail) = if cleared {
        (Status::Crashed, Some(CLEARED_CANCEL_DETAIL.to_owned()))
    } else {
        (
            kept.map_or(Status::Crashed, |worker| super::worker_status(worker.state)),
            None,
        )
    };
    tracing::warn!(session = %session_id, ?status, "reset declaration denied; reset cancelled");
    shared.hub.set_status(session_id, status, detail);
    true
}

#[cfg(test)]
mod tests {
    use super::super::{Frame, Hub, RemoteRuntime, read_frame, send_pending};
    use super::*;

    async fn wire(runtime: &RemoteRuntime) -> Vec<CoreCommand> {
        let mut bytes = Vec::new();
        send_pending(&runtime.shared, &mut bytes).await.unwrap();
        let mut input = bytes.as_slice();
        let mut commands = Vec::new();
        while let Some(Frame::CoreCommand { command }) = read_frame(&mut input).await.unwrap() {
            commands.push(command);
        }
        commands
    }

    #[tokio::test]
    async fn a_reset_declaration_and_its_stop_wait_together_for_the_launch_plan() {
        let hub = Hub::new();
        hub.create_local_session(
            "s".to_owned(),
            "codex".to_owned(),
            "/tmp".to_owned(),
            "test".to_owned(),
            crate::model::SessionOrigin::Web,
            false,
        );
        let bindings = format!(
            "{}s",
            crate::core::settings_keys::SESSION_MCP_BINDINGS_PREFIX
        );
        // A bound session whose launch plan is not prepared yet.
        hub.set_setting(bindings.clone(), serde_json::json!({"bindings": ["x"]}));
        let worker = super::super::tests::snapshot("s");
        let runtime = RemoteRuntime::for_test(hub.clone(), vec![worker.clone()]);
        runtime.connect_for_test();
        let mut update = worker.launch.clone().unwrap();
        update.provider_version = "2".to_owned();
        runtime.reset(update);
        // Neither the declaration nor the stop leaves, and neither is refused:
        // the old worker keeps running instead of relaunching its old release.
        let early = wire(&runtime).await;
        assert!(early.is_empty(), "{early:?}");
        let reset = |command: &CoreCommand| match command {
            CoreCommand::EnsureSession { session } => session.adopt_only,
            CoreCommand::StopSession { command_id, .. } => command_id.starts_with("reset-"),
            _ => false,
        };
        assert_eq!(
            runtime
                .pending_for_test()
                .iter()
                .filter(|c| reset(c))
                .count(),
            2
        );
        // Once the plan is current, both leave in order.
        hub.set_setting(bindings, serde_json::Value::Null);
        let commands: Vec<_> = wire(&runtime).await.into_iter().filter(reset).collect();
        assert!(
            matches!(
                commands.as_slice(),
                [
                    CoreCommand::EnsureSession { session },
                    CoreCommand::StopSession { command_id, .. },
                ] if session.adopt_only && command_id.starts_with("reset-")
            ),
            "{commands:?}"
        );
        assert!(WAITING.lock().get("s").is_none());
    }

    fn bound_session(hub: &Hub, id: &str) -> String {
        hub.create_local_session(
            id.to_owned(),
            "codex".to_owned(),
            "/tmp".to_owned(),
            "test".to_owned(),
            crate::model::SessionOrigin::Web,
            false,
        );
        let bindings = format!(
            "{}{id}",
            crate::core::settings_keys::SESSION_MCP_BINDINGS_PREFIX
        );
        hub.set_setting(bindings.clone(), serde_json::json!({"bindings": ["x"]}));
        bindings
    }

    fn reset_stop(command: &CoreCommand) -> bool {
        matches!(command, CoreCommand::StopSession { command_id, .. } if command_id.starts_with("reset-"))
    }

    #[tokio::test]
    async fn a_same_release_reload_waits_for_its_launch_plan() {
        // The plan owner was bound to the worker being reset, so a reload's
        // declaration resolves no plan until a fresh owner is prepared. Its
        // stop must wait instead of relaunching from the Machine's cache
        // (2026-11-20, sess-1795152728753).
        let hub = Hub::new();
        let bindings = bound_session(&hub, "same-release");
        let worker = super::super::tests::snapshot("same-release");
        let runtime = RemoteRuntime::for_test(hub.clone(), vec![worker.clone()]);
        runtime.connect_for_test();
        runtime.reset(worker.launch.clone().unwrap());
        let early = wire(&runtime).await;
        assert!(early.is_empty(), "{early:?}");
        assert!(runtime.pending_for_test().iter().any(reset_stop));
        hub.set_setting(bindings, serde_json::Value::Null);
        let commands = wire(&runtime).await;
        assert!(commands.iter().any(reset_stop), "{commands:?}");
        assert!(WAITING.lock().get("same-release").is_none());
    }

    #[tokio::test]
    async fn a_denied_reset_declaration_cancels_its_stop_and_keeps_the_worker() {
        let hub = Hub::new();
        bound_session(&hub, "denied");
        let worker = super::super::tests::snapshot("denied");
        let runtime = RemoteRuntime::for_test(hub.clone(), vec![worker.clone()]);
        runtime.connect_for_test();
        runtime.reset(worker.launch.clone().unwrap());
        // The plan wait is exhausted: the declaration meets admission, which
        // denies it because no current plan exists.
        let expired = std::time::Instant::now()
            .checked_sub(PLAN_WAIT + Duration::from_secs(1))
            .unwrap();
        WAITING.lock().insert("denied".to_owned(), expired);
        let commands = wire(&runtime).await;
        assert!(!commands.iter().any(reset_stop), "{commands:?}");
        assert!(!runtime.pending_for_test().iter().any(reset_stop));
        assert!(!runtime.shared.resetting.lock().contains("denied"));
        assert!(runtime.shared.workers.lock().contains_key("denied"));
        assert_eq!(hub.status("denied"), Some(super::super::Status::Busy));
        assert!(WAITING.lock().get("denied").is_none());
    }

    #[tokio::test]
    async fn a_cancelled_clear_fails_closed_and_keeps_the_old_context_fenced() {
        let hub = Hub::new();
        bound_session(&hub, "kept");
        let worker = super::super::tests::snapshot("kept");
        let runtime = RemoteRuntime::for_test(hub.clone(), vec![worker.clone()]);
        runtime.connect_for_test();
        assert!(super::super::apply_snapshot(&runtime.shared, &worker));
        hub.prepare_context_reset("kept");
        runtime.reset(worker.launch.clone().unwrap());
        let expired = std::time::Instant::now()
            .checked_sub(PLAN_WAIT + Duration::from_secs(1))
            .unwrap();
        WAITING.lock().insert("kept".to_owned(), expired);
        let commands = wire(&runtime).await;
        assert!(!commands.iter().any(reset_stop), "{commands:?}");
        // No fresh agent yet: the session needs a restart instead of
        // silently running the cleared context.
        assert_eq!(hub.status("kept"), Some(super::super::Status::Crashed));
        super::super::handle_frame(
            &runtime.shared,
            Frame::WorkerEvent {
                session_id: "kept".into(),
                worker_epoch: worker.worker_epoch.clone(),
                runtime_seq: 5,
                event: super::super::RuntimeEvent::ContextUsage {
                    used: 7,
                    size: 10,
                    raw: serde_json::Value::Null,
                    observed_at_ms: 1,
                },
                diagnostics: Vec::new(),
            },
            &mut tokio::io::sink(),
        )
        .await
        .unwrap();
        let meta = hub.session_info("kept").unwrap().meta;
        assert_eq!(meta.context_used, 0);
        assert_eq!(meta.agent_session_id, None);
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn stale_reset_declaration_cannot_overwrite_newer_pending_version() {
        let hub = Hub::new();
        hub.create_local_session(
            "superseded".to_owned(),
            "codex".to_owned(),
            "/tmp".to_owned(),
            "test".to_owned(),
            crate::model::SessionOrigin::Web,
            false,
        );
        let worker = super::super::tests::snapshot("superseded");
        let runtime = RemoteRuntime::for_test(hub.clone(), vec![worker.clone()]);
        runtime.connect_for_test();

        let mut stale = worker.launch.clone().unwrap();
        stale.adopt_only = true;
        stale.cwd = "/tmp/old-cwd".to_owned();
        stale.mcp_plan = Some(Box::new(crate::mcp_binding::NativeLaunchPlan {
            schema: 1,
            command: "/bin/stale-plan".to_owned(),
            bindings: Vec::new(),
        }));
        runtime.reset(stale.clone());

        let key = "ensure:superseded";
        let v1 = runtime
            .shared
            .pending
            .lock()
            .version(key)
            .expect("initial ensure version");

        // Interleave a newer reset declaration (version v2) with a different cwd.
        let mut newer = worker.launch.clone().unwrap();
        newer.adopt_only = true;
        newer.cwd = "/tmp/new-cwd".to_owned();
        newer.mcp_plan = Some(Box::new(crate::mcp_binding::NativeLaunchPlan {
            schema: 1,
            command: "/bin/newer-plan".to_owned(),
            bindings: Vec::new(),
        }));
        runtime
            .shared
            .declarations
            .lock()
            .insert("superseded".to_owned(), newer.clone());
        let v2 = runtime.shared.pending.lock().insert(
            key.to_owned(),
            CoreCommand::EnsureSession {
                session: newer.clone(),
            },
        );
        assert!(v2 > v1);

        // Passing the stale v1 snapshot into `current()` must be skipped and
        // must NOT overwrite either `pending` (at v2) or `declarations`.
        let mut deferred = HashSet::new();
        assert!(
            !runtime.shared.pending.lock().update_if_version(
                key,
                v1,
                CoreCommand::EnsureSession {
                    session: stale.clone(),
                },
            ),
            "update_if_version must refuse stale version v1 once v2 is installed"
        );
        assert!(
            current(
                &runtime.shared,
                key,
                v1,
                CoreCommand::EnsureSession { session: stale },
                &mut deferred,
            )
            .is_none(),
            "stale snapshot must be skipped once a newer version exists"
        );
        assert!(
            deferred.contains("superseded"),
            "skipping a stale reset declaration must defer the session for this pass"
        );
        // Any reset stop ordered behind the skipped declaration in the same pass
        // must also wait rather than going out ahead of v2's EnsureSession.
        assert!(
            current(
                &runtime.shared,
                "reset-1",
                1,
                CoreCommand::StopSession {
                    session_id: "superseded".to_owned(),
                    command_id: "reset-1".to_owned(),
                },
                &mut deferred,
            )
            .is_none(),
            "reset stop must wait in the same pass when its stale declaration was skipped"
        );
        assert_eq!(runtime.shared.pending.lock().version(key), Some(v2));
        match runtime.shared.pending.lock().get(key) {
            Some(CoreCommand::EnsureSession { session }) => {
                assert_eq!(session.cwd, "/tmp/new-cwd");
                assert_eq!(
                    session.mcp_plan.as_ref().map(|p| p.command.as_str()),
                    Some("/bin/newer-plan")
                );
            }
            other => panic!("expected newer EnsureSession in pending, got {other:?}"),
        }
        assert_eq!(
            runtime
                .shared
                .declarations
                .lock()
                .get("superseded")
                .and_then(|d| d.mcp_plan.as_ref())
                .map(|p| p.command.as_str()),
            Some("/bin/newer-plan")
        );
    }
}
