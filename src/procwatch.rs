//! Detect agent-spawned **background processes** still alive *between* turns.
//!
//! ## The gap this closes
//!
//! When the agent backgrounds work (`run_in_background` bash, a build it polls
//! for) it ends the ACP turn (`stop_reason: EndTurn`) and yields — the process
//! keeps running, and the harness re-invokes the agent when it finishes. Between
//! those turns cowboy sees the session as non-busy with no in-flight ACP tool,
//! so the UI shows "Waiting for your reply" / "Queue paused" even though real
//! work is still running. ACP carries no signal for this (verified against the
//! spec + the just-released 1.0 SDK: no background/activity state, `StopReason`
//! has no "waiting"). The agent's own `run_in_background` task is invisible to
//! the protocol — but it IS in the agent's cgroup. The cgroup is the only
//! ground truth, so we read it directly.
//!
//! ## Why a heuristic, and why this one
//!
//! The cgroup also holds the agent's OWN infrastructure — the ACP launch chain
//! (`npm → node → claude`) plus any persistent MCP servers (`chrome-devtools-mcp`,
//! …). Naively counting processes is unreliable: that baseline varies per agent
//! (claude vs codex vs future) and per MCP config (servers come and go, lazily).
//! So we don't count — we **classify**, and learn the baseline from the running
//! system rather than hardcoding any agent/MCP knowledge:
//!
//! 1. **Seed** `infra` from a snapshot taken while the session is idle and
//!    BEFORE its first turn (the agent-ready edge): everything alive then is
//!    infrastructure — agent chain + startup MCP. No per-agent list.
//! 2. A cgroup process not in `infra` is a **candidate** (spawned by a tool).
//! 3. **Promotion** absorbs config drift: a candidate that stays CPU-quiescent
//!    (≈0 jiffies growth) for [`PROMOTE_QUIESCENT`] is folded into `infra` — a
//!    lazily-started MCP / idle helper just sits there, so it self-classifies as
//!    infrastructure. A background *task* burns CPU (or churns children) and is
//!    never promoted while it works.
//! 4. **A session is flagged** when a non-promoted candidate has outlived the
//!    [`SETTLE`] delay (which lets a tool's transient children exit first).
//!
//! Reliability details learned the hard way on hawk:
//! - Process **identity is `(pid, starttime)`**, not the bare pid — pids are
//!   reused; the start-time (an opaque, immutable token here) disambiguates.
//! - `/proc/<pid>/stat` MUST be parsed by splitting **after the last `)`** — the
//!   `comm` field is `(name)` and can contain spaces/parens (`(npm exec @agent)`),
//!   so a naive whitespace split shifts every later field. This bit us in
//!   testing (a bogus start-time of `11`).
//! - Detached strays are reaped by the agent at the next turn boundary, so the
//!   only things that survive between turns are infra + genuine harness-managed
//!   background tasks — which keeps the candidate set clean.
//!
//! Known corners (documented, accepted for v1): a background task that idles
//! CPU-quiescent for [`PROMOTE_QUIESCENT`] (e.g. blocked on a slow download) can
//! be mis-promoted; and on a daemon restart a task already running is baked into
//! the fresh seed. Both self-heal (the task exits) and are rare. Thread-level
//! stalls (a hung thread inside the agent's own process) are out of scope —
//! `cgroup.procs` is per-process.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::cgroup;
use crate::core::{Hub, Status};

/// Poll cadence. Cheap (one small file read per live session) so a tight-ish
/// interval keeps the widget responsive without meaningful cost.
const TICK: Duration = Duration::from_secs(2);
/// A candidate must persist this long before it counts — lets the short-lived
/// children a tool spawns (and zombies mid-reap) drain out, killing flicker.
const SETTLE: Duration = Duration::from_secs(2);
/// Sustained CPU-quiescence after which a candidate is deemed infrastructure
/// (a lazy MCP / idle helper) and promoted. Short enough that a lazy MCP only
/// briefly mis-flags; long enough that a working build doesn't dip under it.
const PROMOTE_QUIESCENT: Duration = Duration::from_secs(10);

/// One cgroup process we're tracking as not-yet-classified-as-infra.
struct Candidate {
    /// Identity tiebreaker (see module docs): if a pid reappears with a
    /// different start-time it's a different process — we reset.
    starttime: u64,
    /// First time we saw this exact `(pid, starttime)` — gates [`SETTLE`].
    first_seen: Instant,
    /// Last CPU sample (utime+stime jiffies) for the quiescence test.
    last_cpu: u64,
    /// When CPU growth first fell to ~0 in an unbroken run; reset to `None` on
    /// any activity. `Some(t)` past [`PROMOTE_QUIESCENT`] → promote.
    quiescent_since: Option<Instant>,
}

/// Per-session classifier state. Lives entirely in the watcher task — the Hub
/// only ever receives the resulting boolean.
#[derive(Default)]
struct SessionWatch {
    /// Known infrastructure, keyed `pid → starttime`. Seeded when empty (first
    /// observation, and again after a revival prunes it bare), grows by
    /// promotion, pruned as entries exit.
    infra: HashMap<u32, u64>,
    candidates: HashMap<u32, Candidate>,
    /// Last value pushed to the Hub — avoids redundant broadcasts.
    flagged: bool,
}

/// A `/proc/<pid>/stat` sample: the two fields we need, parsed robustly.
struct Stat {
    starttime: u64,
    /// utime + stime, in clock ticks (jiffies). Monotonic per process.
    cpu: u64,
}

/// Parse `(starttime, utime+stime)` from `/proc/<pid>/stat`. Returns None if the
/// process is gone or the line is malformed. Splits AFTER the last `)` so a
/// `comm` containing spaces/parens can't shift the field offsets (see module
/// docs). Post-`comm` fields (1-based): state=1, ppid=2, …, utime=12, stime=13,
/// …, starttime=20 — i.e. the kernel's stat field N maps to N-2 here.
fn read_stat(pid: u32) -> Option<Stat> {
    let raw = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = &raw[raw.rfind(')')? + 1..];
    let f: Vec<&str> = rest.split_whitespace().collect();
    // Need through starttime (index 19, 0-based).
    if f.len() <= 19 {
        return None;
    }
    let utime: u64 = f[11].parse().ok()?;
    let stime: u64 = f[12].parse().ok()?;
    let starttime: u64 = f[19].parse().ok()?;
    Some(Stat {
        starttime,
        cpu: utime + stime,
    })
}

/// Background-process watcher. Spawned once at startup; runs for the daemon's
/// life. Mirrors `run_scheduler`'s shape (a `Hub` clone + a loop).
pub async fn run_proc_watch(hub: Hub) {
    let mut watches: HashMap<String, SessionWatch> = HashMap::new();
    let mut ticker = tokio::time::interval(TICK);
    loop {
        ticker.tick().await;
        let now = Instant::now();
        let live = hub.session_list();
        // Drop watches for sessions that have gone away entirely.
        let alive: std::collections::HashSet<&str> =
            live.iter().map(|m| m.id.as_str()).collect();
        watches.retain(|id, _| alive.contains(id.as_str()));

        for meta in &live {
            // A turn in flight (Busy) or a dead agent (Exited/Crashed/…) owns the
            // status slot already — the spinner / terminal overlay shows. We only
            // care about the BETWEEN-turns window, so clear any stale flag and
            // keep the classifier state warm for the next idle period.
            if meta.status != Status::Running {
                if let Some(w) = watches.get_mut(&meta.id) {
                    if w.flagged {
                        w.flagged = false;
                        hub.set_background_task(&meta.id, false);
                    }
                }
                continue;
            }
            let Some(dir) = cgroup::agent_dir(&meta.id) else {
                continue; // cgroup-v1 host / unresolved — fail-open, no widget.
            };
            let pids = cgroup::read_procs(&dir);
            if pids.is_empty() {
                continue; // cgroup gone (teardown race) — leave state as-is.
            }
            let w = watches.entry(meta.id.clone()).or_default();
            let flag = w.classify(&pids, now);
            if flag != w.flagged {
                w.flagged = flag;
                hub.set_background_task(&meta.id, flag);
            }
        }
    }
}

impl SessionWatch {
    /// Run one classification pass over the cgroup's current pids; returns
    /// whether the session has a live background task.
    fn classify(&mut self, pids: &[u32], now: Instant) -> bool {
        // Snapshot stats once per pid this tick.
        let stats: HashMap<u32, Stat> = pids
            .iter()
            .filter_map(|&p| read_stat(p).map(|s| (p, s)))
            .collect();

        // Prune infra entries that exited or whose pid was reused (start-time
        // changed) — a reused pid is a different process and must re-classify.
        self.infra
            .retain(|p, st| stats.get(p).is_some_and(|s| s.starttime == *st));

        // Seed when infra is bare: the FIRST idle observation (agent chain +
        // startup MCP, before any turn → no background task possible yet), AND
        // again after a session REVIVAL prunes the old chain away (the restarted
        // agent has fresh pids — without re-seeding they'd misread as background
        // tasks). `status == Running` with an empty infra can only mean "the agent
        // chain is here under new pids", since a dead agent isn't Running. Seed
        // and emit nothing this tick.
        if self.infra.is_empty() {
            self.infra = stats.iter().map(|(&p, s)| (p, s.starttime)).collect();
            self.candidates.clear();
            return false;
        }

        for (&pid, s) in &stats {
            if self.infra.get(&pid) == Some(&s.starttime) {
                continue; // known infrastructure.
            }
            // Candidate. Reset if the pid was reused under a new start-time.
            let c = match self.candidates.get_mut(&pid) {
                Some(c) if c.starttime == s.starttime => c,
                _ => {
                    self.candidates.insert(
                        pid,
                        Candidate {
                            starttime: s.starttime,
                            first_seen: now,
                            last_cpu: s.cpu,
                            quiescent_since: Some(now),
                        },
                    );
                    continue;
                }
            };
            // Quiescence track: any CPU growth resets the idle clock.
            if s.cpu > c.last_cpu {
                c.quiescent_since = None;
            } else if c.quiescent_since.is_none() {
                c.quiescent_since = Some(now);
            }
            c.last_cpu = s.cpu;
            // Promote a sustained-quiescent candidate to infrastructure.
            if c
                .quiescent_since
                .is_some_and(|t| now.duration_since(t) >= PROMOTE_QUIESCENT)
            {
                self.infra.insert(pid, s.starttime);
                self.candidates.remove(&pid);
            }
        }

        // Drop candidates that have exited.
        self.candidates
            .retain(|p, c| stats.get(p).is_some_and(|s| s.starttime == c.starttime));

        // Flagged iff a settled, non-promoted candidate remains.
        self.candidates
            .values()
            .any(|c| now.duration_since(c.first_seen) >= SETTLE)
    }
}
