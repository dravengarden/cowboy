//! Agent acquisition: `code.diagnostics` opens a file of the session's own
//! worktree that no Code pane holds. The session's "Project language servers"
//! switch is the authority (owner decision, 2026-11-06); the Operator-only
//! owned-buffer API is not used. Each holding has a unique legacy owner,
//! retained across retries. A later acquisition never reuses an owner whose
//! delayed Close could still arrive. Holdings are a small share of each
//! Machine, below interactive use, and are released after idle or once the
//! switch is off.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use sha2::{Digest as _, Sha256};

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
    lease: String,
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

struct Opening {
    key: Key,
    lease: String,
    possibly_open: bool,
    retire: bool,
    touch: bool,
}

impl Opening {
    fn new(key: Key, lease: String, possibly_open: bool) -> Self {
        Self {
            key,
            lease,
            possibly_open,
            retire: false,
            touch: possibly_open,
        }
    }
}

impl Drop for Opening {
    fn drop(&mut self) {
        let mut held = HELD.lock();
        if let Some(holding) = held.get_mut(&self.key)
            && holding.lease == self.lease
        {
            holding.opening -= 1;
            holding.possibly_open |= self.possibly_open;
            holding.releasing |= self.retire;
            if self.touch {
                holding.last_used = Instant::now();
            }
            if holding.opening == 0 && (holding.releasing || !holding.possibly_open) {
                held.remove(&self.key);
            }
        }
    }
}

/// Controller observations are not Machine resource ownership. Once this
/// one-use cleanup ends (including cancellation), retire only its observation.
/// Unknown native resources remain charged to the Machine's original owner.
struct RetiringObservation(Key, String);

impl Drop for RetiringObservation {
    fn drop(&mut self) {
        let mut held = HELD.lock();
        if held
            .get(&self.0)
            .is_some_and(|h| h.lease == self.1 && h.opening == 0)
        {
            held.remove(&self.0);
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
    format!(
        "cowboy-agent-{:x}-{}",
        Sha256::digest(session.as_bytes()),
        uuid::Uuid::new_v4().simple()
    )
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
    let (lease, refresh_only) = {
        let mut held = HELD.lock();
        if let Some(holding) = held.get_mut(&key) {
            if holding.releasing
                || holding.scope.as_ref() != Some(&context.scope)
                || holding.machine != context.machine_id
                || holding.worktree != worktree
            {
                return Err("unavailable");
            }
            // Never recreate an owner that Machine idle maintenance may have
            // released while a delayed Controller Close still names it.
            holding.opening += 1;
            (holding.lease.clone(), true)
        } else if !fits(&held, session, &context.machine_id, &worktree) {
            return Err("budget");
        } else {
            let lease = owner(session);
            // Reserve before the await so concurrent reads cannot oversubscribe.
            held.insert(
                key.clone(),
                Held {
                    lease: lease.clone(),
                    machine: context.machine_id.clone(),
                    worktree: worktree.clone(),
                    scope: Some(context.scope.clone()),
                    last_used: Instant::now(),
                    releasing: false,
                    opening: 1,
                    possibly_open: false,
                },
            );
            (lease, false)
        }
    };
    let mut opening = Opening::new(key, lease.clone(), !refresh_only);
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
        refresh_only,
    )
    .await;
    if refresh_only {
        if matches!(&opened, Ok(ZedAdapterResponse::AgentBufferRefreshed { lease_id, .. }) if lease_id == &lease)
        {
            opening.touch = true;
            return Ok(());
        }
        // No fallback Open, even on an older Machine or a lost refresh reply.
        // A future acquisition gets a new nonce and remains subject to Machine
        // quotas, including every old uncertain owner still retained there.
        opening.retire = matches!(opened, Ok(ZedAdapterResponse::AgentBufferBudget { .. }));
        // A transport failure is not an ownership observation. Retain this
        // nonce for harmless refresh retries, without extending its idle time.
        return Err("unavailable");
    }
    if let Ok(ZedAdapterResponse::AgentBufferBudget { state, .. }) = &opened {
        // This typed reply precedes every ensure/open effect. It cannot clear
        // an older unknown Open or another in-flight request's reservation.
        opening.possibly_open = false;
        return Err(if state == "saturated" {
            "budget"
        } else {
            "unavailable"
        });
    }
    if matches!(opened, Ok(ZedAdapterResponse::Buffer { .. })) {
        Ok(())
    } else {
        // An unknown Open remains owned and charged by the Machine. A later
        // refresh only observes a confirmed holding; it cannot replay Open.
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
        let _retiring = RetiringObservation(key.clone(), holding.lease.clone());
        let (session, path) = &key;
        // Missing or replaced context cannot authorize a native Close. The
        // original Machine retains its owner, quota and idle cleanup anyway.
        let Some(context) = session_code_context(state, session) else {
            continue;
        };
        if holding.scope.as_ref() != Some(&context.scope)
            || context.machine_id != holding.machine
            || context.cwd != holding.worktree
        {
            continue;
        }
        let lease = &holding.lease;
        let result = zed_session::buffer_request(
            &state.hub,
            &state.machine_control,
            state.zed_adapter_socket.as_deref(),
            &context.scope,
            zed_session::BufferRequest {
                worktree: &holding.worktree,
                path,
                lease_id: lease,
                open: false,
            },
        )
        .await;
        if !matches!(result, Ok(ZedAdapterResponse::Buffer { .. })) {
            tracing::warn!(%session, "Machine-owned agent code release remains unconfirmed; retiring Controller observation");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opening(key: Key, possibly_open: bool) -> Opening {
        let lease = HELD.lock()[&key].lease.clone();
        Opening::new(key, lease, possibly_open)
    }

    #[test]
    fn refused_refresh_retires_observation_after_all_original_openers_leave() {
        let key = (uuid::Uuid::new_v4().to_string(), "file.rs".into());
        let mut entry = holding("m", "/a");
        entry.opening = 2;
        HELD.lock().insert(key.clone(), entry);
        let original = opening(key.clone(), true);
        let mut refresh = opening(key.clone(), false);
        refresh.retire = true;
        drop(refresh);
        assert!(HELD.lock()[&key].releasing);
        assert_eq!(HELD.lock()[&key].opening, 1);
        drop(original);
        assert!(!HELD.lock().contains_key(&key));
    }

    #[test]
    fn uncertain_refresh_preserves_nonce_without_extending_idle_time() {
        let key = (uuid::Uuid::new_v4().to_string(), "file.rs".into());
        let mut entry = holding("m", "/a");
        entry.last_used = Instant::now() - IDLE;
        let last_used = entry.last_used;
        entry.opening = 1;
        HELD.lock().insert(key.clone(), entry);
        drop(opening(key.clone(), false));
        let mut held = HELD.lock();
        let original = held.get_mut(&key).unwrap();
        assert_eq!(original.last_used, last_used);
        assert!(
            original.begin_release(true),
            "retry errors cannot defer cleanup forever"
        );
        held.remove(&key);
    }

    #[test]
    fn cleanup_observation_never_removes_a_replacement_nonce() {
        let key = (uuid::Uuid::new_v4().to_string(), "file.rs".into());
        let old = holding("m", "/a");
        let old_lease = old.lease.clone();
        HELD.lock().insert(key.clone(), old);
        drop(RetiringObservation(key.clone(), old_lease.clone()));
        assert!(
            !HELD.lock().contains_key(&key),
            "unknown native outcome is Machine-owned"
        );
        let new = holding("m", "/a");
        let new_lease = new.lease.clone();
        HELD.lock().insert(key.clone(), new);
        drop(RetiringObservation(key.clone(), old_lease.clone()));
        drop(Opening::new(key.clone(), old_lease, true));
        assert_eq!(HELD.lock()[&key].lease, new_lease);
        assert_eq!(HELD.lock()[&key].opening, 0);
        HELD.lock().remove(&key);
    }

    #[tokio::test]
    async fn canceled_close_retires_only_controller_observation() {
        let key = (uuid::Uuid::new_v4().to_string(), "file.rs".into());
        let mut entry = holding("m", "/a");
        assert!(entry.begin_release(false));
        let lease = entry.lease.clone();
        HELD.lock().insert(key.clone(), entry);
        let (started, ready) = tokio::sync::oneshot::channel();
        let task_key = key.clone();
        let task = tokio::spawn(async move {
            let _retiring = RetiringObservation(task_key, lease);
            started.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        ready.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(!HELD.lock().contains_key(&key));
    }

    #[test]
    fn new_acquisition_never_reuses_a_delayed_close_owner() {
        let first = owner("sess-test");
        let next = owner("sess-test");
        assert_ne!(first, next);
        assert_eq!(
            first.rsplit_once('-').unwrap().0,
            next.rsplit_once('-').unwrap().0
        );
        assert_ne!(
            first.rsplit_once('-').unwrap().0,
            owner("another-session").rsplit_once('-').unwrap().0
        );
        assert!(first.len() <= 128);
        assert!(owner(&"long-session".repeat(100)).len() <= 128);
    }

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
            lease: owner("fixture"),
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
        drop(opening(key.clone(), false));
        assert_eq!(
            HELD.lock()[&key].opening,
            1,
            "another caller is still pending"
        );
        drop(opening(key.clone(), false));
        assert!(
            !HELD.lock().contains_key(&key),
            "both requests refused before effects"
        );
        let mut entry = holding("m", "/a");
        entry.possibly_open = false;
        entry.opening = 2;
        HELD.lock().insert(key.clone(), entry);
        drop(opening(key.clone(), true));
        drop(opening(key.clone(), false));
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
            let _opening = opening(task_key, true);
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
        assert!(owner("sess-1").starts_with("cowboy-agent-"));
    }
}
