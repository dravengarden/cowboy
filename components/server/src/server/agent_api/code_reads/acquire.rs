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
    scope: Option<crate::core::SessionCodeScope>,
    last_used: Instant,
    releasing: bool,
    opening: usize,
    possibly_open: bool,
}

type Key = (String, String);

static HELD: LazyLock<Mutex<HashMap<Key, Held>>> = LazyLock::new(Mutex::default);

/// Observations from this exact Session incarnation only. A summary never
/// refreshes idle time, acquires a new holding, or adopts a replaced scope.
pub(super) fn observed_paths(scope: &crate::core::SessionCodeScope) -> Vec<String> {
    let mut paths: Vec<_> = HELD
        .lock()
        .iter()
        .filter(|((session, _), held)| {
            session == scope.session_id() && held.scope.as_ref() == Some(scope)
        })
        .map(|((_, path), _)| path.clone())
        .collect();
    paths.sort();
    paths
}

struct Opening(Key, bool);

impl Drop for Opening {
    fn drop(&mut self) {
        let mut held = HELD.lock();
        if let Some(holding) = held.get_mut(&self.0) {
            holding.opening -= 1;
            holding.possibly_open |= self.1;
            holding.last_used = Instant::now();
            if holding.opening == 0 && !holding.possibly_open {
                held.remove(&self.0);
            }
        }
    }
}

impl Held {
    fn begin_release(&mut self, allowed: bool) -> bool {
        if self.releasing || self.opening != 0 || (allowed && self.last_used.elapsed() < IDLE) {
            return false;
        }
        self.releasing = true;
        true
    }
}

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
            if holding.releasing
                || holding.scope.as_ref() != Some(&context.scope)
                || holding.machine != context.machine_id
                || holding.worktree != worktree
            {
                return Err("unavailable");
            }
            // Still sent: the open is idempotent for this owner and restores a
            // holding a replaced code runtime lost.
            holding.last_used = Instant::now();
            holding.opening += 1;
        } else if !fits(&held, session, &context.machine_id, &worktree) {
            return Err("budget");
        } else {
            // Reserve before the await so concurrent reads cannot oversubscribe.
            held.insert(
                key.clone(),
                Held {
                    machine: context.machine_id.clone(),
                    worktree: worktree.clone(),
                    scope: Some(context.scope.clone()),
                    last_used: Instant::now(),
                    releasing: false,
                    opening: 1,
                    possibly_open: false,
                },
            );
        }
    }
    let mut opening = Opening(key, true);
    let lease = owner(session);
    let opened = zed_session::agent_buffer_request(
        &state.hub,
        &state.machine_control,
        &context.scope,
        zed_session::BufferRequest {
            worktree: &worktree,
            path: &target,
            lease_id: &lease,
            open: true,
        },
    )
    .await;
    if let Ok(ZedAdapterResponse::AgentBufferBudget { state, .. }) = &opened {
        // This typed reply precedes every ensure/open effect. It cannot clear
        // an older unknown Open or another in-flight request's reservation.
        opening.1 = false;
        return Err(if state == "saturated" {
            "budget"
        } else {
            "unavailable"
        });
    }
    if matches!(opened, Ok(ZedAdapterResponse::Buffer { .. })) {
        Ok(())
    } else {
        // Whatever the adapter kept is re-adopted by the same owner next time.
        // An unknown Open may still own native resources. Keep its budget
        // reservation until the same owner's release is confirmed.
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
        let holding = {
            let mut held = HELD.lock();
            let Some(holding) = held.get_mut(&key) else {
                continue;
            };
            // A previous release may still execute. Do not send another close
            // or let an acquisition reuse this owner after an unknown outcome.
            // Acquisition may have refreshed it since the initial scan.
            if !holding.begin_release(state.hub.language_servers_allowed(&key.0)) {
                continue;
            }
            holding.clone()
        };
        let (session, path) = &key;
        // Missing or replaced context cannot authorize release of the old
        // native owner. Keep its reservation until lifecycle recovery.
        let Some(context) = session_code_context(state, session) else {
            continue;
        };
        if holding.scope.as_ref() != Some(&context.scope)
            || context.machine_id != holding.machine
            || context.cwd != holding.worktree
        {
            continue;
        }
        let lease = owner(session);
        let result = zed_session::buffer_request(
            &state.hub,
            &state.machine_control,
            state.zed_adapter_socket.as_deref(),
            &context.scope,
            zed_session::BufferRequest {
                worktree: &holding.worktree,
                path,
                lease_id: &lease,
                open: false,
            },
        )
        .await;
        if matches!(result, Ok(ZedAdapterResponse::Buffer { .. })) {
            HELD.lock().remove(&key);
        } else {
            tracing::warn!(%session, "agent code buffer release remains unconfirmed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_keeps_session_incarnations_separate_without_extending_idle_time() {
        use crate::core::{Hub, SessionOrigin};
        let hub = Hub::new();
        for id in ["summary-session", "summary-other"] {
            hub.create_local_session(
                id.into(),
                "codex".into(),
                "/work/a".into(),
                "test".into(),
                SessionOrigin::default(),
                false,
            );
        }
        let scope = hub.session_code_scope("summary-session").unwrap();
        let key = ("summary-session".into(), "a.rs".into());
        let last_used = Instant::now() - Duration::from_secs(90);
        let mut entry = holding("local", "/work/a");
        entry.scope = Some(scope.clone());
        entry.last_used = last_used;
        HELD.lock().insert(key.clone(), entry);
        assert_eq!(observed_paths(&scope), vec!["a.rs"]);
        assert!(observed_paths(&hub.session_code_scope("summary-other").unwrap()).is_empty());
        assert_eq!(HELD.lock()[&key].last_used, last_used);
        hub.update_session_cwd("summary-session", "/work/b".into())
            .unwrap();
        hub.update_session_cwd("summary-session", "/work/a".into())
            .unwrap();
        assert!(observed_paths(&hub.session_code_scope("summary-session").unwrap()).is_empty());
        HELD.lock().remove(&key);
    }

    fn holding(machine: &str, worktree: &str) -> Held {
        Held {
            machine: machine.into(),
            worktree: worktree.into(),
            scope: None,
            last_used: Instant::now(),
            releasing: false,
            opening: 0,
            possibly_open: true,
        }
    }

    #[test]
    fn definite_resource_refusal_frees_only_never_dispatched_holdings() {
        let key = (uuid::Uuid::new_v4().to_string(), "file.rs".into());
        let mut entry = holding("m", "/a");
        entry.possibly_open = false;
        entry.opening = 2;
        HELD.lock().insert(key.clone(), entry);
        drop(Opening(key.clone(), false));
        assert_eq!(
            HELD.lock()[&key].opening,
            1,
            "another caller is still pending"
        );
        drop(Opening(key.clone(), false));
        assert!(
            !HELD.lock().contains_key(&key),
            "both requests refused before effects"
        );
        let mut entry = holding("m", "/a");
        entry.possibly_open = false;
        entry.opening = 2;
        HELD.lock().insert(key.clone(), entry);
        drop(Opening(key.clone(), true));
        drop(Opening(key.clone(), false));
        assert!(
            HELD.lock()[&key].possibly_open,
            "a refusal cannot erase another unknown Open"
        );
        HELD.lock().remove(&key);
    }

    #[test]
    fn cleanup_rechecks_refresh_and_waits_for_in_flight_open() {
        let mut held = holding("m", "/a");
        assert!(
            !held.begin_release(true),
            "a refreshed holding survives an old scan"
        );
        held.last_used = Instant::now() - IDLE;
        held.opening = 1;
        assert!(
            !held.begin_release(true),
            "idle cleanup cannot overtake open"
        );
        assert!(
            !held.begin_release(false),
            "revocation cannot overtake open"
        );
        held.opening = 0;
        assert!(held.begin_release(true));
        assert!(
            !held.begin_release(false),
            "unknown close is never replayed"
        );
    }

    #[test]
    fn unknown_release_keeps_its_budget_and_revocation_releases_without_idle_wait() {
        let mut held = HashMap::new();
        for i in 0..MAX_PER_SESSION {
            let mut entry = holding("m", "/a");
            assert!(entry.begin_release(false));
            held.insert(("a".into(), format!("f{i}")), entry);
        }
        assert!(!fits(&held, "a", "m", "/a"));
        held.remove(&("a".into(), "f0".into()));
        assert!(
            fits(&held, "a", "m", "/a"),
            "only confirmed close frees capacity"
        );
    }

    #[tokio::test]
    async fn canceled_open_stops_blocking_cleanup_without_freeing_unknown_resources() {
        let key = (uuid::Uuid::new_v4().to_string(), "file.rs".into());
        let mut entry = holding("m", "/a");
        entry.opening = 1;
        HELD.lock().insert(key.clone(), entry);
        let (started, ready) = tokio::sync::oneshot::channel();
        let task_key = key.clone();
        let task = tokio::spawn(async move {
            let _opening = Opening(task_key, true);
            let _ = started.send(());
            std::future::pending::<()>().await;
        });
        ready.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        let mut held = HELD.lock();
        let entry = held
            .get_mut(&key)
            .expect("unknown open still consumes capacity");
        assert_eq!(entry.opening, 0);
        assert!(
            !entry.begin_release(true),
            "cancellation refreshes idle time"
        );
        assert!(
            entry.begin_release(false),
            "revocation can now attempt cleanup"
        );
        held.remove(&key);
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
