//! Agent acquisition: `code.diagnostics` opens a file of the session's own
//! worktree that no Code pane holds. The session's "Project language servers"
//! switch is the authority (owner decision, 2026-11-06); the Operator-only
//! owned-buffer API is not used. Each session holds its buffers under one
//! deterministic legacy owner, so a retry or a Controller restart re-adopts an
//! open instead of leaking another. Agent holdings are a small share of each
//! Machine, below interactive use, and are released after idle or once the
//! switch is off.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use super::super::super::{
    AppState, ZedAdapterResponse, session_code_context, zed_language_target, zed_session,
};

const MAX_PER_SESSION: usize = 4;
const MAX_PER_MACHINE: usize = 8;
/// Worktrees whose language servers agent reads may keep warm per Machine.
const MAX_WORKTREES_PER_MACHINE: usize = 2;
const IDLE: Duration = Duration::from_secs(300);

#[derive(Clone, Debug)]
struct Held {
    machine: String,
    worktree: String,
    last_used: Instant,
}

type Key = (String, String);

static HELD: LazyLock<Mutex<HashMap<Key, Held>>> = LazyLock::new(Mutex::default);

fn owner(session: &str) -> String {
    format!("cowboy-agent-{session}")
}

/// Whether one more holding fits the session's and the Machine's share.
fn fits(held: &HashMap<Key, Held>, session: &str, machine: &str, worktree: &str) -> bool {
    let on_machine: Vec<&Held> = held.values().filter(|h| h.machine == machine).collect();
    let worktrees: HashSet<&str> = on_machine.iter().map(|h| h.worktree.as_str()).collect();
    held.keys().filter(|(s, _)| s == session).count() < MAX_PER_SESSION
        && on_machine.len() < MAX_PER_MACHINE
        && (worktrees.contains(worktree) || worktrees.len() < MAX_WORKTREES_PER_MACHINE)
}

/// Open `path` for this session's agent, or refresh its holding.
pub(super) async fn acquire(
    state: &AppState,
    session: &str,
    path: &str,
) -> Result<(), &'static str> {
    release_stale(state).await;
    if !state.hub.language_servers_allowed(session) {
        return Err("restricted");
    }
    let context = session_code_context(state, session).ok_or("unavailable")?;
    let (worktree, target) = zed_language_target(&context.machine_id, &context.cwd, path)
        .filter(|(worktree, _)| {
            std::path::Path::new(worktree) == std::path::Path::new(&context.cwd)
        })
        .ok_or("refused")?;
    let key = (session.to_owned(), target.clone());
    {
        let mut held = HELD.lock();
        if let Some(holding) = held.get_mut(&key) {
            holding.last_used = Instant::now();
            return Ok(());
        }
        if !fits(&held, session, &context.machine_id, &worktree) {
            return Err("budget");
        }
        // Reserve before the await so concurrent reads cannot oversubscribe.
        held.insert(
            key.clone(),
            Held {
                machine: context.machine_id.clone(),
                worktree: worktree.clone(),
                last_used: Instant::now(),
            },
        );
    }
    let lease = owner(session);
    let opened = zed_session::buffer_request(
        &state.hub,
        &state.machine_control,
        state.zed_adapter_socket.as_deref(),
        &context.scope,
        zed_session::BufferRequest {
            worktree: &worktree,
            path: &target,
            lease_id: &lease,
            open: true,
        },
    )
    .await;
    if matches!(opened, Ok(ZedAdapterResponse::Buffer { .. })) {
        Ok(())
    } else {
        // Whatever the adapter kept is re-adopted by the same owner next time.
        HELD.lock().remove(&key);
        Err("unavailable")
    }
}

/// Release holdings that are idle, or whose session turned the switch off or
/// is gone.
pub(super) async fn release_stale(state: &AppState) {
    let stale: Vec<Key> = HELD
        .lock()
        .iter()
        .filter(|((session, _), holding)| {
            holding.last_used.elapsed() >= IDLE || !state.hub.language_servers_allowed(session)
        })
        .map(|(key, _)| key.clone())
        .collect();
    for key in stale {
        let Some(holding) = HELD.lock().remove(&key) else {
            continue;
        };
        let (session, path) = key;
        // A session that no longer exists keeps nothing to release through.
        let Some(context) = session_code_context(state, &session) else {
            continue;
        };
        let lease = owner(&session);
        if let Err(error) = zed_session::buffer_request(
            &state.hub,
            &state.machine_control,
            state.zed_adapter_socket.as_deref(),
            &context.scope,
            zed_session::BufferRequest {
                worktree: &holding.worktree,
                path: &path,
                lease_id: &lease,
                open: false,
            },
        )
        .await
        {
            tracing::warn!(%session, ?error, "agent code buffer release failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holding(machine: &str, worktree: &str) -> Held {
        Held {
            machine: machine.into(),
            worktree: worktree.into(),
            last_used: Instant::now(),
        }
    }

    #[test]
    fn agent_holdings_stay_within_the_session_machine_and_worktree_shares() {
        let mut held = HashMap::new();
        for i in 0..MAX_PER_SESSION {
            held.insert(("a".into(), format!("f{i}")), holding("m", "/a"));
        }
        assert!(!fits(&held, "a", "m", "/a"), "per-session share");
        assert!(fits(&held, "b", "m", "/b"));
        held.insert(("b".into(), "f".into()), holding("m", "/b"));
        // Two worktrees are warm on this Machine; a third waits.
        assert!(!fits(&held, "c", "m", "/c"));
        assert!(fits(&held, "b", "m", "/b"));
        assert!(fits(&held, "c", "other", "/c"));
        for i in 0..3 {
            held.insert(("b".into(), format!("g{i}")), holding("m", "/b"));
        }
        assert!(!fits(&held, "d", "m", "/a"), "per-Machine share");
        assert_eq!(owner("sess-1"), "cowboy-agent-sess-1");
    }
}
