//! The daemon's cheap write path: any agent's add/update/delete is a fast
//! `enqueue` (no model), and a debouncer coalesces a burst into a single batched
//! wake (which, in prod, hands the batch to the cowboy memory session for
//! judgment).
//!
//! Ported faithfully from mnemosyne's `internal/queue/queue.go`. The Go uses
//! `time.AfterFunc` (a runtime-managed background timer); here a self-contained
//! background thread implements the same quiesce timer, so the `Queue` stays a
//! pure data type with no async runtime requirement (Phase C can drive it from
//! tokio or threads alike). The fire rules are byte-for-byte the Go's: fire on
//! `Cap`, on `MaxWait` since the first item, else (re)arm the `Quiesce` timer.

use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::store::Memory;

/// The mutation kind. Mirrors Go's `queue.Op`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Add,
    Update,
    Delete,
}

impl Op {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Op::Add => "add",
            Op::Update => "update",
            Op::Delete => "delete",
        }
    }
}

/// One queued change. `slug` is the caller's project slug, or `""` for the
/// machine tier. `cmid` is an optional client id for reconciliation/dedup of
/// retries. Mirrors Go's `queue.Mutation`.
#[derive(Debug, Clone)]
pub struct Mutation {
    pub op: Op,
    pub memory: Memory,
    pub slug: String,
    pub cmid: String,
}

/// Tunes the debounce: fire after `quiesce` of silence, OR when the batch reaches
/// `cap`, OR when `max_wait` has elapsed since the first item. Mirrors Go's
/// `queue.Config`.
#[derive(Debug, Clone, Copy)]
pub struct Config {
    pub quiesce: Duration,
    pub cap: usize,
    pub max_wait: Duration,
}

impl Config {
    /// A sane starting point for low-volume memory traffic. Mirrors Go's
    /// `queue.DefaultConfig`.
    #[must_use]
    pub fn default_config() -> Config {
        Config {
            quiesce: Duration::from_secs(2),
            cap: 64,
            max_wait: Duration::from_secs(30),
        }
    }
}

impl Default for Config {
    fn default() -> Config {
        Config::default_config()
    }
}

type Wake = Arc<dyn Fn(Vec<Mutation>) + Send + Sync>;

struct Inner {
    cfg: Config,
    wake: Wake,
    batch: Vec<Mutation>,
    first: Instant,
    /// Generation counter: bumped each time the quiesce timer (re)arms or the
    /// batch fires, so a stale timer thread's wakeup is ignored — the equivalent
    /// of Go's `timer.Stop()` before re-arming.
    timer_gen: u64,
}

/// Coalesces mutations into batched wakes. Mirrors Go's `queue.Queue`.
pub struct Queue {
    inner: Arc<Mutex<Inner>>,
    cv: Arc<Condvar>,
    _timer: JoinHandle<()>,
}

impl Queue {
    /// Returns a Queue that calls `wake(batch)` once per coalesced burst. Mirrors
    /// Go's `queue.New`.
    #[must_use]
    pub fn new<F>(cfg: Config, wake: F) -> Queue
    where
        F: Fn(Vec<Mutation>) + Send + Sync + 'static,
    {
        let inner = Arc::new(Mutex::new(Inner {
            cfg,
            wake: Arc::new(wake),
            batch: Vec::new(),
            first: Instant::now(),
            timer_gen: 0,
        }));
        let cv = Arc::new(Condvar::new());

        // Background quiesce timer. It sleeps until either the armed deadline
        // elapses (→ fire) or it is re-notified (→ re-read the deadline). A
        // generation check makes a superseded arming a no-op, matching the Go
        // `timer.Stop()` + fresh `AfterFunc` pattern.
        let timer_inner = Arc::clone(&inner);
        let timer_cv = Arc::clone(&cv);
        let timer = std::thread::spawn(move || loop {
            let mut guard = timer_inner.lock().unwrap();
            // Wait until there is an armed timer (a non-empty batch with a live
            // generation). When the batch is empty there is nothing to fire.
            if guard.batch.is_empty() {
                guard = timer_cv.wait(guard).unwrap();
                continue;
            }
            let gen_at_arm = guard.timer_gen;
            let deadline = guard.first + guard.cfg.quiesce;
            let now = Instant::now();
            let wait = deadline.saturating_duration_since(now);
            if wait.is_zero() {
                // Deadline already passed → fire (if still the live generation).
                fire_locked(&mut guard);
                continue;
            }
            let (g, timeout) = timer_cv.wait_timeout(guard, wait).unwrap();
            guard = g;
            if timeout.timed_out() && guard.timer_gen == gen_at_arm && !guard.batch.is_empty() {
                fire_locked(&mut guard);
            }
            // Otherwise the generation moved (re-armed/fired/Enqueue) → loop and
            // re-read the new deadline.
        });

        Queue {
            inner,
            cv,
            _timer: timer,
        }
    }

    /// Add a mutation (cheap; no model). May trigger an immediate wake if the
    /// batch hits `cap` or `max_wait`, otherwise it (re)arms the quiesce timer.
    /// Mirrors Go's `queue.Enqueue`.
    pub fn enqueue(&self, m: Mutation) {
        let mut g = self.inner.lock().unwrap();
        if g.batch.is_empty() {
            g.first = Instant::now();
        }
        g.batch.push(m);

        let cap = g.cfg.cap;
        let max_wait = g.cfg.max_wait;
        if g.batch.len() >= cap || g.first.elapsed() >= max_wait {
            fire_locked(&mut g);
            return;
        }
        // (Re)arm the quiesce timer: bump the generation and wake the timer
        // thread so it re-reads the deadline. Equivalent to Go stopping the old
        // timer and starting a fresh AfterFunc.
        g.timer_gen = g.timer_gen.wrapping_add(1);
        drop(g);
        self.cv.notify_all();
    }

    /// The current pending batch size (for tests/metrics). Mirrors Go's `Depth`.
    #[must_use]
    pub fn depth(&self) -> usize {
        self.inner.lock().unwrap().batch.len()
    }
}

/// Drain the batch and fire the wake. Caller holds the lock. Bumps the generation
/// so any in-flight timer wakeup is ignored. Mirrors Go's `fireLocked`.
fn fire_locked(g: &mut std::sync::MutexGuard<'_, Inner>) {
    if g.batch.is_empty() {
        return;
    }
    g.timer_gen = g.timer_gen.wrapping_add(1);
    let batch = std::mem::take(&mut g.batch);
    // Fire while holding the lock (as Go's `fireLocked` does under `q.mu`). The
    // wake callback must not re-enter the queue — same contract as the Go.
    let wake = Arc::clone(&g.wake);
    wake(batch);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::store::MemoryType;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn mut_add(name: &str, slug: &str) -> Mutation {
        Mutation {
            op: Op::Add,
            memory: Memory {
                name: name.to_string(),
                description: "d".to_string(),
                mem_type: MemoryType::Project,
                body: "b".to_string(),
            },
            slug: slug.to_string(),
            cmid: String::new(),
        }
    }

    #[test]
    fn burst_coalesces_to_one_wake() {
        let wakes = Arc::new(AtomicUsize::new(0));
        let last_batch = Arc::new(AtomicUsize::new(0));
        let w = Arc::clone(&wakes);
        let lb = Arc::clone(&last_batch);

        let q = Queue::new(
            Config {
                quiesce: Duration::from_millis(100),
                cap: 64,
                max_wait: Duration::from_secs(5),
            },
            move |b| {
                w.fetch_add(1, Ordering::SeqCst);
                lb.store(b.len(), Ordering::SeqCst);
            },
        );

        for _ in 0..5 {
            q.enqueue(mut_add("m", "-home-draven-columbus"));
            std::thread::sleep(Duration::from_millis(15)); // < quiesce → coalesce
        }

        // Wait for the wake (quiesce elapses after the last enqueue).
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(wakes.load(Ordering::SeqCst), 1, "want exactly 1 wake");
        assert_eq!(last_batch.load(Ordering::SeqCst), 5, "want batch of 5");

        // Prove no spurious second wake.
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(wakes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cap_fires_immediately() {
        let fired = Arc::new(AtomicUsize::new(0));
        let got = Arc::new(AtomicUsize::new(0));
        let f = Arc::clone(&fired);
        let g = Arc::clone(&got);
        let q = Queue::new(
            Config {
                quiesce: Duration::from_secs(3600),
                cap: 3,
                max_wait: Duration::from_secs(3600),
            },
            move |b| {
                g.store(b.len(), Ordering::SeqCst);
                f.fetch_add(1, Ordering::SeqCst);
            },
        );
        for _ in 0..3 {
            q.enqueue(mut_add("m", ""));
        }
        // Cap-triggered: fires synchronously inside the 3rd enqueue.
        assert_eq!(fired.load(Ordering::SeqCst), 1, "cap must fire a wake");
        assert_eq!(got.load(Ordering::SeqCst), 3, "want batch of 3");
    }
}
