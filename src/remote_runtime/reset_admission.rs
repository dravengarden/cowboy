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

use super::{CoreCommand, Duration, HashMap, HashSet, Mutex, Shared};

/// Longer than the MCP launch recovery pass needs (5-second ticks); after it
/// the declaration meets ordinary admission, as before.
const PLAN_WAIT: Duration = Duration::from_secs(60);
const RECHECK: Duration = Duration::from_secs(2);

static WAITING: std::sync::LazyLock<Mutex<HashMap<String, std::time::Instant>>> =
    std::sync::LazyLock::new(Default::default);

/// The command to admit now, or `None` while it waits. `deferred` holds the
/// sessions whose reset declaration waits in this pass (it is ordered first).
pub(super) fn current(
    shared: &Shared,
    key: &str,
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
            if session.adopt_only
                && shared.resetting.lock().contains(&session.session_id)
                && cross_release(shared, &session) =>
        {
            session
        }
        command => return Some(command),
    };
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
    // Only the plan of the very release this declaration names.
    if plan != session.mcp_plan
        && meta.provider_version == session.provider_version
        && meta.provider_generation_digest == session.provider_generation_digest
        && meta.provider_auth_generation == session.provider_auth_generation
        && meta.provider_account_id == session.provider_account_id
    {
        session.mcp_plan = plan;
        if let Some(declared) = shared.declarations.lock().get_mut(&id) {
            declared.mcp_plan.clone_from(&session.mcp_plan);
        }
        shared.pending.lock().insert(
            key.to_owned(),
            CoreCommand::EnsureSession {
                session: session.clone(),
            },
        );
    }
    Some(CoreCommand::EnsureSession { session })
}

/// A same-release reset (a stuck or reconnecting worker) keeps its previous
/// admission: its cached Machine declaration already names this release.
fn cross_release(shared: &Shared, session: &super::StartSession) -> bool {
    shared
        .workers
        .lock()
        .get(&session.session_id)
        .and_then(|worker| worker.launch.as_ref())
        .is_none_or(|running| {
            running.provider_version != session.provider_version
                || running.provider_generation_digest != session.provider_generation_digest
        })
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
}
