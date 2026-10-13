//! Bulk lanes: extra WebSockets beside a Machine's primary connection.
//!
//! One TCP connection across the Machine overlay carries about 1.2-1.8 MB/s
//! (one cross-border flow), while eight parallel flows carried 3-7 MB/s in
//! the same period. A lane is an authenticated auxiliary WebSocket that
//! carries only large execution requests and replies, which are independent
//! RPCs with no order relative to other frames. Small and ordered frames stay
//! on the primary connection, so latency-sensitive traffic never queues behind
//! bulk. The Machine opens lanes while bulk flows (up to [`MAX_LANES`]) and
//! closes a lane after [`LANE_IDLE`] without traffic; an idle Machine keeps
//! none. Requests are idempotent by operation ID, so after a lane fails the
//! Machine resends every request still awaiting a reply.

// The Controller uses the lane set and routing constants; the Machine also
// uses the opener and outstanding-request tracking. Each build has only one.
#![cfg_attr(not(all(feature = "full", feature = "machine-host")), allow(dead_code))]

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tokio::sync::mpsc::UnboundedSender;

/// Request header on the primary upgrade, answered with the same value.
pub(crate) const LANES_HEADER: &str = "x-cowboy-machine-lanes";
pub(crate) const LANES: &str = "lanes-v1";
/// Secret binding a lane to its authenticated primary connection: returned on
/// the primary upgrade, presented on each lane upgrade.
pub(crate) const LANE_TOKEN_HEADER: &str = "x-cowboy-machine-lane-token";
pub(crate) const LANE_PATH: &str = "/api/machine/lane";
/// Frames at least this large use a lane when one is open.
pub(crate) const LANE_MIN_BYTES: usize = 64 * 1024;
pub(crate) const MAX_LANES: usize = 4;
/// The Machine closes a lane after this long without traffic either way.
/// A remote Machine reaches the Controller over pooled cross-border streams:
/// every lane is a new stream, so a lane costs a pooled connection and a
/// replacement handshake over a lossy path, and a burst of four empties the
/// pool for the connection behind it. With a one-minute idle bound OVH opened
/// 471 lanes in two days, half of them living under 65 s; ten minutes keeps a
/// lane through a session's bursts at the cost of one idle socket.
pub(crate) const LANE_IDLE: Duration = Duration::from_secs(600);
/// The Controller sends only on lanes active more recently than this, so a
/// lane it writes to can never reach the Machine's idle deadline. It stays
/// below the previous one-minute `LANE_IDLE`, so a Controller is safe beside
/// a Machine of either release; the Machine's own sends refresh a lane.
pub(crate) const LANE_FRESH: Duration = Duration::from_secs(45);
/// At most one lane is being opened at a time, and not more often than this.
pub(crate) const LANE_OPEN_INTERVAL: Duration = Duration::from_millis(500);

static NEXT_LANE: AtomicU64 = AtomicU64::new(1);

pub(crate) fn next_lane_id() -> u64 {
    NEXT_LANE.fetch_add(1, Ordering::Relaxed)
}

struct Lane<M> {
    id: u64,
    tx: UnboundedSender<M>,
    active: Mutex<Instant>,
}

/// The open lanes of one primary connection.
pub(crate) struct LaneSet<M> {
    lanes: Mutex<Vec<Arc<Lane<M>>>>,
    next: AtomicUsize,
}

impl<M> Default for LaneSet<M> {
    fn default() -> Self {
        Self {
            lanes: Mutex::new(Vec::new()),
            next: AtomicUsize::new(0),
        }
    }
}

impl<M> LaneSet<M> {
    pub(crate) fn add(&self, id: u64, tx: UnboundedSender<M>) {
        self.lanes.lock().push(Arc::new(Lane {
            id,
            tx,
            active: Mutex::new(Instant::now()),
        }));
    }

    pub(crate) fn remove(&self, id: u64) {
        self.lanes.lock().retain(|lane| lane.id != id);
    }

    pub(crate) fn len(&self) -> usize {
        self.lanes.lock().len()
    }

    /// Record traffic received on a lane.
    pub(crate) fn touch(&self, id: u64) {
        if let Some(lane) = self.lanes.lock().iter().find(|lane| lane.id == id) {
            *lane.active.lock() = Instant::now();
        }
    }

    pub(crate) fn idle_for(&self, id: u64) -> Option<Duration> {
        self.lanes
            .lock()
            .iter()
            .find(|lane| lane.id == id)
            .map(|lane| lane.active.lock().elapsed())
    }

    /// Send on the next open lane in turn, considering only lanes active
    /// within `fresh` when given. Returns the message if no lane took it.
    pub(crate) fn send(&self, message: M, fresh: Option<Duration>) -> Result<u64, M> {
        let lanes: Vec<_> = self
            .lanes
            .lock()
            .iter()
            .filter(|lane| fresh.is_none_or(|fresh| lane.active.lock().elapsed() < fresh))
            .cloned()
            .collect();
        if lanes.is_empty() {
            return Err(message);
        }
        let start = self.next.fetch_add(1, Ordering::Relaxed);
        let mut message = message;
        for offset in 0..lanes.len() {
            let lane = &lanes[(start + offset) % lanes.len()];
            match lane.tx.send(message) {
                Ok(()) => {
                    *lane.active.lock() = Instant::now();
                    return Ok(lane.id);
                }
                Err(returned) => message = returned.0,
            }
        }
        Err(message)
    }
}

/// Execution requests a Machine sent and has not yet seen answered, kept so
/// that they can be resent after a lane fails.
#[derive(Default)]
pub(crate) struct Outstanding {
    requests: HashMap<String, Arc<str>>,
}

/// Bound on remembered requests; workers bound their own pending calls well
/// below this, so it only guards against a leak.
const MAX_OUTSTANDING: usize = 16 * 1024;

impl Outstanding {
    pub(crate) fn insert(&mut self, request_id: &str, text: Arc<str>) {
        if self.requests.len() >= MAX_OUTSTANDING {
            self.requests.clear();
        }
        self.requests.insert(request_id.to_owned(), text);
    }

    pub(crate) fn answered(&mut self, request_id: &str) {
        self.requests.remove(request_id);
    }

    pub(crate) fn pending(&self) -> Vec<Arc<str>> {
        self.requests.values().cloned().collect()
    }
}

/// The Machine side's decision for a large frame: whether another lane should
/// be opened now.
#[derive(Default)]
pub(crate) struct Opener {
    opening: bool,
    last_open: Option<Instant>,
}

impl Opener {
    /// Whether to start opening a lane, given how many are open.
    pub(crate) fn should_open(&mut self, open: usize, now: Instant) -> bool {
        if self.opening
            || open >= MAX_LANES
            || self
                .last_open
                .is_some_and(|last| now.duration_since(last) < LANE_OPEN_INTERVAL)
        {
            return false;
        }
        self.opening = true;
        self.last_open = Some(now);
        true
    }

    pub(crate) fn opened(&mut self) {
        self.opening = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lanes_take_turns_and_skip_stale_or_closed_ones() {
        let set = LaneSet::<u32>::default();
        assert_eq!(set.send(1, None), Err(1));
        let (a_tx, mut a_rx) = tokio::sync::mpsc::unbounded_channel();
        let (b_tx, mut b_rx) = tokio::sync::mpsc::unbounded_channel();
        set.add(1, a_tx);
        set.add(2, b_tx);
        let used: Vec<_> = (0..4).map(|n| set.send(n, None).unwrap()).collect();
        assert_eq!(used.iter().filter(|id| **id == 1).count(), 2);
        assert_eq!(used.iter().filter(|id| **id == 2).count(), 2);
        assert!(a_rx.try_recv().is_ok());
        // A closed lane is skipped, not lost.
        let _ = b_rx.try_recv();
        drop(b_rx);
        for n in 0..3 {
            assert_eq!(set.send(n, None), Ok(1));
        }
        // Nothing fresh enough: the caller keeps the message.
        assert_eq!(set.send(9, Some(Duration::ZERO)), Err(9));
        set.remove(1);
        set.remove(2);
        assert_eq!(set.len(), 0);
    }

    #[test]
    fn outstanding_requests_are_forgotten_once_answered() {
        let mut outstanding = Outstanding::default();
        outstanding.insert("a", Arc::from("frame-a"));
        outstanding.insert("b", Arc::from("frame-b"));
        outstanding.answered("a");
        assert_eq!(outstanding.pending(), vec![Arc::<str>::from("frame-b")]);
    }

    #[test]
    fn lanes_open_one_at_a_time_up_to_the_limit() {
        let mut opener = Opener::default();
        let now = Instant::now();
        assert!(opener.should_open(0, now));
        assert!(!opener.should_open(0, now), "one opening at a time");
        opener.opened();
        assert!(!opener.should_open(1, now), "rate limited");
        let later = now + LANE_OPEN_INTERVAL;
        assert!(opener.should_open(1, later));
        opener.opened();
        assert!(!opener.should_open(MAX_LANES, later + LANE_OPEN_INTERVAL));
    }
}
