//! Heartbeat-gap attribution (`docs/host-isolation-design.md`, L0).
//!
//! WHY: workers have been isolated after 48-58 s without a heartbeat while the
//! host ran at load above 400, and the mechanism is not established. Shared
//! `MemoryHigh` throttling, CPU, I/O, one large frame and a broker stall each
//! call for a different remedy. This module records what the broker saw, and
//! the cgroup pressure around each gap, without changing any decision.
//!
//! The connection reader and the heartbeat monitor only update atomics and
//! `try_send` a small notice. Every cgroup read, user-manager query and log
//! record happens in one sampler task with a bounded queue, deduplicated scopes
//! and blocking reads moved off the runtime threads.

use crate::runtime_wire::{Frame, MAX_FRAME_BYTES};
use parking_lot::Mutex;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io,
    pin::Pin,
    sync::{
        Arc, OnceLock, Weak,
        atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncReadExt as _, ReadBuf},
    sync::mpsc,
};

/// A heartbeat later than this after the previous one is recorded. Workers
/// beat every 10 s; the broker isolates after 45 s of frame silence.
pub(super) const GAP_THRESHOLD: Duration = Duration::from_secs(20);
const NOTICE_CAPACITY: usize = 64;
const SAMPLE_PERIOD: Duration = Duration::from_secs(10);
/// Samples kept per scope: 100 s covers the 45-60 s isolation window with the
/// baseline sample before it.
const SAMPLE_HISTORY: usize = 10;
const MAX_WORKER_SCOPES: usize = 256;
const MAX_SHARED_SCOPES: usize = 64;
const BIND_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_COUNTER_FILE: usize = 4096;
const CGROUP2_SUPER_MAGIC: i64 = 0x6367_7270;
const PROC_SUPER_MAGIC: i64 = 0x9fa0;
/// Logged in place of a value that could not be read.
const UNAVAILABLE: i64 = -1;

static DEVICE_CONFIG: OnceLock<crate::config::Handle> = OnceLock::new();

/// Starts the Machine's live Device-file watch and gives the sampler the same
/// handle, so the `diagnostics.heartbeat_gap_sampler` kill switch applies
/// within seconds.
pub fn watch(
    device_config: &crate::config::Handle,
    path: std::path::PathBuf,
) -> tokio::task::JoinHandle<()> {
    let _ = DEVICE_CONFIG.set(device_config.clone());
    device_config.watch(crate::config::Scope::Device, path)
}

fn sampler_enabled() -> bool {
    DEVICE_CONFIG
        .get()
        .is_none_or(|config| config.get(&crate::config::schema::DEVICE_HEARTBEAT_GAP_SAMPLER))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(super) enum Phase {
    Header = 0,
    Payload = 1,
    Decode = 2,
    Handling = 3,
}

impl Phase {
    fn name(raw: u8) -> &'static str {
        match raw {
            0 => "header",
            1 => "payload",
            2 => "decode",
            _ => "handling",
        }
    }
}

/// Clocks and frame progress of one worker connection. Written only by its
/// reader task (and the heartbeat arm), read by the sampler.
pub(super) struct Connection {
    session: String,
    connection: u64,
    epoch: String,
    unit: Option<String>,
    peer_pid: Option<u32>,
    origin: Instant,
    opened_ms: u64,
    /// Milliseconds since `origin`, plus one; zero means never.
    last_heartbeat: AtomicU64,
    last_frame: AtomicU64,
    last_progress: AtomicU64,
    /// Frame-clock value already covered by a deferred-isolation record.
    deferred_reported: AtomicU64,
    phase: AtomicU8,
    declared: AtomicU32,
    read: AtomicU64,
    notices: mpsc::Sender<Notice>,
    dropped: Arc<AtomicU64>,
}

impl Connection {
    fn now_ms(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX - 1)
    }

    fn stamp(clock: &AtomicU64, now: u64) -> u64 {
        clock.swap(now.saturating_add(1), Ordering::Relaxed)
    }

    fn set_phase(&self, phase: Phase, declared: usize) {
        self.declared.store(
            u32::try_from(declared).unwrap_or(u32::MAX),
            Ordering::Relaxed,
        );
        self.read.store(0, Ordering::Relaxed);
        self.phase.store(phase as u8, Ordering::Relaxed);
    }

    fn progressed(&self, bytes: usize) {
        self.read.fetch_add(bytes as u64, Ordering::Relaxed);
        Self::stamp(&self.last_progress, self.now_ms());
    }

    fn frame_done(&self) {
        self.phase.store(Phase::Handling as u8, Ordering::Relaxed);
        Self::stamp(&self.last_frame, self.now_ms());
    }

    /// Records a received heartbeat. A heartbeat that closes a gap above
    /// [`GAP_THRESHOLD`] produces one notice; nothing else leaves this call.
    pub(super) fn heartbeat(self: &Arc<Self>) {
        let now = self.now_ms();
        let previous = Self::stamp(&self.last_heartbeat, now);
        let since = if previous == 0 {
            self.opened_ms
        } else {
            previous - 1
        };
        if let Some(gap) = gap_ms(since, now) {
            self.notify(Notice::Gap(GapNotice {
                connection: Arc::clone(self),
                kind: GapKind::Heartbeat,
                gap_ms: gap,
                since_ms: since,
                at_ms: now,
                unread_bytes: None,
                detail: None,
            }));
        }
    }

    fn notify(&self, notice: Notice) {
        if self.notices.try_send(notice).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn snapshot(&self, now: u64) -> FrameState {
        let age = |clock: &AtomicU64| match clock.load(Ordering::Relaxed) {
            0 => None,
            stamp => Some(now.saturating_sub(stamp - 1)),
        };
        FrameState {
            heartbeat_age_ms: age(&self.last_heartbeat),
            frame_age_ms: age(&self.last_frame),
            progress_age_ms: age(&self.last_progress),
            phase: Phase::name(self.phase.load(Ordering::Relaxed)),
            declared: self.declared.load(Ordering::Relaxed),
            read: self.read.load(Ordering::Relaxed),
        }
    }
}

/// A gap only when strictly above the threshold.
pub(super) fn gap_ms(since_ms: u64, now_ms: u64) -> Option<u64> {
    let gap = now_ms.saturating_sub(since_ms);
    (u128::from(gap) > GAP_THRESHOLD.as_millis()).then_some(gap)
}

struct FrameState {
    heartbeat_age_ms: Option<u64>,
    frame_age_ms: Option<u64>,
    progress_age_ms: Option<u64>,
    phase: &'static str,
    declared: u32,
    read: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GapKind {
    /// A heartbeat arrived after a gap.
    Heartbeat,
    /// The sweep extended isolation because worker output was still queued.
    Deferred,
    /// The sweep isolated the worker; no closing heartbeat arrived.
    Isolated,
}

impl GapKind {
    fn name(self) -> &'static str {
        match self {
            Self::Heartbeat => "heartbeat",
            Self::Deferred => "backlog_extension",
            Self::Isolated => "isolated",
        }
    }
}

struct GapNotice {
    connection: Arc<Connection>,
    kind: GapKind,
    gap_ms: u64,
    since_ms: u64,
    at_ms: u64,
    unread_bytes: Option<u64>,
    detail: Option<String>,
}

enum Notice {
    Bind(Arc<Connection>),
    Gap(GapNotice),
}

/// Broker-owned entry point: registers connections and forwards sweep
/// decisions. Nothing here blocks or allocates per byte.
pub(super) struct Hub {
    origin: Instant,
    tx: mpsc::Sender<Notice>,
    rx: Mutex<Option<mpsc::Receiver<Notice>>>,
    dropped: Arc<AtomicU64>,
    connections: Mutex<HashMap<String, Weak<Connection>>>,
}

impl Default for Hub {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel(NOTICE_CAPACITY);
        Self {
            origin: Instant::now(),
            tx,
            rx: Mutex::new(Some(rx)),
            dropped: Arc::new(AtomicU64::new(0)),
            connections: Mutex::new(HashMap::new()),
        }
    }
}

impl Hub {
    /// Registers an admitted worker connection. `unit` is its systemd user
    /// unit (absent in Direct mode); `peer_pid` comes from `SO_PEERCRED`.
    pub(super) fn connect(
        &self,
        session: &str,
        connection: u64,
        epoch: &str,
        unit: Option<String>,
        peer_pid: Option<u32>,
    ) -> Arc<Connection> {
        let opened_ms = u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(0);
        let observed = Arc::new(Connection {
            session: session.to_owned(),
            connection,
            epoch: epoch.to_owned(),
            unit,
            peer_pid,
            origin: self.origin,
            opened_ms,
            last_heartbeat: AtomicU64::new(0),
            last_frame: AtomicU64::new(0),
            last_progress: AtomicU64::new(0),
            deferred_reported: AtomicU64::new(0),
            phase: AtomicU8::new(Phase::Handling as u8),
            declared: AtomicU32::new(0),
            read: AtomicU64::new(0),
            notices: self.tx.clone(),
            dropped: Arc::clone(&self.dropped),
        });
        let mut connections = self.connections.lock();
        connections.retain(|_, connection| connection.strong_count() > 0);
        connections.insert(session.to_owned(), Arc::downgrade(&observed));
        drop(connections);
        observed.notify(Notice::Bind(Arc::clone(&observed)));
        observed
    }

    fn current(&self, session: &str, connection: Option<u64>) -> Option<Arc<Connection>> {
        self.connections
            .lock()
            .get(session)
            .and_then(Weak::upgrade)
            .filter(|observed| connection.is_none_or(|id| observed.connection == id))
    }

    /// The sweep kept a silent worker because its output is still queued.
    /// Recorded once per frame-silence episode.
    pub(super) fn deferred(&self, session: &str, unread_bytes: u64, silent: Duration) {
        let Some(observed) = self.current(session, None) else {
            return;
        };
        let frame = observed.last_frame.load(Ordering::Relaxed);
        if observed.deferred_reported.swap(frame, Ordering::Relaxed) == frame && frame != 0 {
            return;
        }
        let now = observed.now_ms();
        let silent = u64::try_from(silent.as_millis()).unwrap_or(u64::MAX);
        observed.notify(Notice::Gap(GapNotice {
            connection: Arc::clone(&observed),
            kind: GapKind::Deferred,
            gap_ms: silent,
            since_ms: now.saturating_sub(silent),
            at_ms: now,
            unread_bytes: Some(unread_bytes),
            detail: None,
        }));
    }

    /// The sweep isolated this connection's worker.
    pub(super) fn isolated(&self, session: &str, connection: u64, detail: &str) {
        let Some(observed) = self.current(session, Some(connection)) else {
            return;
        };
        let now = observed.now_ms();
        let since = match observed.last_heartbeat.load(Ordering::Relaxed) {
            0 => observed.opened_ms,
            stamp => stamp - 1,
        };
        observed.notify(Notice::Gap(GapNotice {
            connection: Arc::clone(&observed),
            kind: GapKind::Isolated,
            gap_ms: now.saturating_sub(since),
            since_ms: since,
            at_ms: now,
            unread_bytes: None,
            detail: Some(detail.chars().take(240).collect()),
        }));
    }

    /// Runs the sampler until the broker stops. A second call returns at once.
    pub(super) async fn run(&self) {
        let Some(rx) = self.rx.lock().take() else {
            return;
        };
        Sampler::new(Arc::clone(&self.dropped)).run(rx).await;
    }
}

/// Byte-progress view of the connection reader. It changes no framing
/// behavior; it only stamps progress after each successful read.
struct Progress<'a, R> {
    inner: &'a mut R,
    connection: &'a Connection,
}

impl<R: AsyncRead + Unpin> AsyncRead for Progress<'_, R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let before = buf.filled().len();
        let polled = Pin::new(&mut *self.inner).poll_read(cx, buf);
        let read = buf.filled().len() - before;
        if read > 0 {
            self.connection.progressed(read);
        }
        polled
    }
}

/// `runtime_wire::read_frame` with phase and progress stamps. The framing is
/// identical: EOF while reading the header, including a partial header, ends
/// the stream with `None`; an oversized length and invalid JSON are
/// `InvalidData`; EOF inside the payload is an error.
pub(super) async fn read_frame<R: AsyncRead + Unpin>(
    reader: &mut R,
    connection: &Connection,
) -> io::Result<Option<Frame>> {
    connection.set_phase(Phase::Header, 0);
    let mut header = [0_u8; 4];
    let mut progress = Progress {
        inner: reader,
        connection,
    };
    match progress.read_exact(&mut header).await {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let len = u32::from_be_bytes(header) as usize;
    if len > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("runtime frame too large: {len} bytes"),
        ));
    }
    connection.set_phase(Phase::Payload, len);
    let mut payload = vec![0; len];
    progress.read_exact(&mut payload).await?;
    connection
        .phase
        .store(Phase::Decode as u8, Ordering::Relaxed);
    let frame = serde_json::from_slice(&payload)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error));
    connection.frame_done();
    frame
}

/// The verified cgroup identity of one worker connection.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Binding {
    control_group: String,
    invocation: String,
}

#[derive(Clone, Debug)]
enum BindingState {
    Verified(Binding),
    Unavailable(&'static str),
}

impl BindingState {
    fn label(&self) -> &'static str {
        match self {
            Self::Verified(_) => "verified",
            Self::Unavailable(reason) => reason,
        }
    }
}

struct Bound {
    connection: Weak<Connection>,
    state: BindingState,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Psi {
    some_us: u64,
    full_us: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Events {
    high: u64,
    max: u64,
    oom: u64,
    oom_kill: u64,
}

/// One read of a scope's counters. `None` is a field that could not be read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Sample {
    cpu: Option<Psi>,
    memory: Option<Psi>,
    io: Option<Psi>,
    events: Option<Events>,
    events_local: Option<Events>,
    current: Option<u64>,
    /// `u64::MAX` is `max` (no limit).
    high: Option<u64>,
    limit: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Scope {
    /// A cgroup v2 path relative to the mount, without a leading slash.
    Cgroup(String),
    Host,
}

struct Sampler {
    dropped: Arc<AtomicU64>,
    reported_dropped: u64,
    bound: HashMap<u64, Bound>,
    history: HashMap<Scope, VecDeque<(Instant, Sample)>>,
    broker_cgroup: Option<String>,
    gap_ids: u64,
}

impl Sampler {
    fn new(dropped: Arc<AtomicU64>) -> Self {
        Self {
            dropped,
            reported_dropped: 0,
            bound: HashMap::new(),
            history: HashMap::new(),
            broker_cgroup: own_cgroup(),
            gap_ids: 0,
        }
    }

    async fn run(mut self, mut rx: mpsc::Receiver<Notice>) {
        let mut interval = tokio::time::interval(SAMPLE_PERIOD);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                notice = rx.recv() => match notice {
                    Some(Notice::Bind(connection)) => self.bind(connection).await,
                    Some(Notice::Gap(gap)) => self.record(gap).await,
                    None => return,
                },
                _ = interval.tick() => self.sample_all().await,
            }
        }
    }

    async fn bind(&mut self, connection: Arc<Connection>) {
        self.bound
            .retain(|_, bound| bound.connection.strong_count() > 0);
        let state = if sampler_enabled() {
            resolve_binding(connection.unit.as_deref(), connection.peer_pid).await
        } else {
            BindingState::Unavailable("sampler_disabled")
        };
        if let BindingState::Verified(binding) = &state
            && let Some(path) = relative_cgroup(&binding.control_group)
        {
            // A unit name, and so its cgroup path, is reused across launches:
            // samples of an earlier invocation must not become this one's
            // baseline.
            self.history.remove(&Scope::Cgroup(path));
        }
        self.bound.insert(
            connection.connection,
            Bound {
                connection: Arc::downgrade(&connection),
                state,
            },
        );
    }

    /// Scopes of all live verified bindings, worker scopes first, shared
    /// ancestors deduplicated, both capped.
    fn scopes(&self) -> Vec<Scope> {
        let mut workers = Vec::new();
        let mut shared = Vec::new();
        let mut seen = HashSet::new();
        for bound in self.bound.values() {
            if bound.connection.strong_count() == 0 {
                continue;
            }
            let BindingState::Verified(binding) = &bound.state else {
                continue;
            };
            let chain = ancestry(&binding.control_group);
            for (depth, scope) in chain.into_iter().enumerate() {
                if !seen.insert(scope.clone()) {
                    continue;
                }
                if depth == 0 && workers.len() < MAX_WORKER_SCOPES {
                    workers.push(scope);
                } else if depth > 0 && shared.len() < MAX_SHARED_SCOPES {
                    shared.push(scope);
                }
            }
        }
        if let Some(own) = &self.broker_cgroup {
            let own = Scope::Cgroup(own.clone());
            if seen.insert(own.clone()) {
                shared.push(own);
            }
        }
        shared.push(Scope::Host);
        workers.extend(shared);
        workers
    }

    async fn sample_all(&mut self) {
        self.report_drops();
        if !sampler_enabled() {
            self.history.clear();
            return;
        }
        let scopes = self.scopes();
        let samples = read_scopes(scopes.clone()).await;
        let now = Instant::now();
        let live: HashSet<_> = scopes.into_iter().collect();
        self.history.retain(|scope, _| live.contains(scope));
        for (scope, sample) in samples {
            let ring = self.history.entry(scope).or_default();
            ring.push_back((now, sample));
            while ring.len() > SAMPLE_HISTORY {
                ring.pop_front();
            }
        }
    }

    fn report_drops(&mut self) {
        let dropped = self.dropped.load(Ordering::Relaxed);
        if dropped > self.reported_dropped {
            tracing::warn!(
                target: "cowboy_machine::heartbeat_gap",
                event_name = "cowboy.machine.heartbeat_gap.dropped",
                count = dropped - self.reported_dropped,
                "heartbeat gap notices dropped; this observation window is incomplete"
            );
            self.reported_dropped = dropped;
        }
    }

    async fn record(&mut self, gap: GapNotice) {
        self.gap_ids += 1;
        let gap_id = self.gap_ids;
        let observed = &gap.connection;
        let state = observed.snapshot(observed.now_ms());
        let binding = self
            .bound
            .get(&observed.connection)
            .map_or(BindingState::Unavailable("unbound"), |bound| {
                bound.state.clone()
            });
        let opt = |value: Option<u64>| value.map_or(UNAVAILABLE, saturate);
        tracing::warn!(
            target: "cowboy_machine::heartbeat_gap",
            event_name = "cowboy.machine.heartbeat_gap",
            session = %observed.session,
            gap_worker_epoch = %observed.epoch,
            gap_connection_id = observed.connection,
            gap_id,
            gap_kind = gap.kind.name(),
            gap_ms = gap.gap_ms,
            gap_unread_bytes = opt(gap.unread_bytes),
            gap_heartbeat_age_ms = opt(state.heartbeat_age_ms),
            gap_frame_age_ms = opt(state.frame_age_ms),
            gap_progress_age_ms = opt(state.progress_age_ms),
            gap_frame_phase = state.phase,
            gap_frame_declared_bytes = state.declared,
            gap_frame_read_bytes = state.read,
            gap_binding = binding.label(),
            gap_invocation = match &binding {
                BindingState::Verified(binding) => binding.invocation.as_str(),
                BindingState::Unavailable(_) => "",
            },
            gap_dropped_notices = self.dropped.load(Ordering::Relaxed),
            gap_sampler_enabled = sampler_enabled(),
            gap_detail = gap.detail.as_deref().unwrap_or(""),
            "worker heartbeat gap"
        );
        let BindingState::Verified(binding) = binding else {
            return;
        };
        if !sampler_enabled() {
            return;
        }
        let mut scopes = ancestry(&binding.control_group);
        if let Some(own) = &self.broker_cgroup {
            scopes.push(Scope::Cgroup(own.clone()));
        }
        scopes.push(Scope::Host);
        let fresh = read_scopes(scopes.clone()).await;
        let now = Instant::now();
        let since = now
            .checked_sub(Duration::from_millis(
                gap.at_ms.saturating_sub(gap.since_ms),
            ))
            .unwrap_or(now);
        for (depth, (scope, sample)) in fresh.into_iter().enumerate() {
            let baseline = self
                .history
                .get(&scope)
                .and_then(|ring| baseline(ring, since));
            log_scope(
                gap_id,
                &observed.session,
                depth,
                &scope,
                &self.broker_cgroup,
                baseline,
                (now, sample),
            );
        }
    }
}

fn saturate(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// The latest sample at or before `since`, else the oldest retained one (a
/// partial coverage that the record states).
fn baseline(ring: &VecDeque<(Instant, Sample)>, since: Instant) -> Option<(Instant, Sample)> {
    ring.iter()
        .rev()
        .find(|(at, _)| *at <= since)
        .or_else(|| ring.front())
        .copied()
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Delta {
    coverage_ms: i64,
    cpu: (i64, i64),
    memory: (i64, i64),
    io: (i64, i64),
    events: (i64, i64, i64, i64),
    events_local: (i64, i64),
}

fn delta(baseline: Option<(Instant, Sample)>, current: (Instant, Sample)) -> Delta {
    let Some((at, before)) = baseline else {
        return Delta {
            coverage_ms: UNAVAILABLE,
            cpu: (UNAVAILABLE, UNAVAILABLE),
            memory: (UNAVAILABLE, UNAVAILABLE),
            io: (UNAVAILABLE, UNAVAILABLE),
            events: (UNAVAILABLE, UNAVAILABLE, UNAVAILABLE, UNAVAILABLE),
            events_local: (UNAVAILABLE, UNAVAILABLE),
        };
    };
    let (now, after) = current;
    let diff = |before: Option<u64>, after: Option<u64>| match (before, after) {
        (Some(before), Some(after)) => saturate(after.saturating_sub(before)),
        _ => UNAVAILABLE,
    };
    let psi = |before: Option<Psi>, after: Option<Psi>| {
        (
            diff(before.map(|p| p.some_us), after.map(|p| p.some_us)),
            diff(
                before.and_then(|p| p.full_us),
                after.and_then(|p| p.full_us),
            ),
        )
    };
    let event = |before: Option<Events>, after: Option<Events>, pick: fn(&Events) -> u64| {
        diff(before.as_ref().map(pick), after.as_ref().map(pick))
    };
    Delta {
        coverage_ms: saturate(
            u64::try_from(now.saturating_duration_since(at).as_millis()).unwrap_or(u64::MAX),
        ),
        cpu: psi(before.cpu, after.cpu),
        memory: psi(before.memory, after.memory),
        io: psi(before.io, after.io),
        events: (
            event(before.events, after.events, |e| e.high),
            event(before.events, after.events, |e| e.max),
            event(before.events, after.events, |e| e.oom),
            event(before.events, after.events, |e| e.oom_kill),
        ),
        events_local: (
            event(before.events_local, after.events_local, |e| e.high),
            event(before.events_local, after.events_local, |e| e.max),
        ),
    }
}

fn log_scope(
    gap_id: u64,
    session: &str,
    depth: usize,
    scope: &Scope,
    broker: &Option<String>,
    baseline: Option<(Instant, Sample)>,
    current: (Instant, Sample),
) {
    let (kind, path) = match scope {
        Scope::Host => ("host", ""),
        Scope::Cgroup(path) if broker.as_deref() == Some(path.as_str()) => {
            ("broker", path.as_str())
        }
        Scope::Cgroup(path) if depth == 0 => ("worker", path.as_str()),
        Scope::Cgroup(path) => ("ancestor", path.as_str()),
    };
    let change = delta(baseline, current);
    let sample = current.1;
    let opt = |value: Option<u64>| value.map_or(UNAVAILABLE, saturate);
    tracing::warn!(
        target: "cowboy_machine::heartbeat_gap",
        event_name = "cowboy.machine.heartbeat_gap.scope",
        session = %session,
        gap_id,
        gap_scope_kind = kind,
        gap_scope_depth = depth as u64,
        gap_cgroup = path,
        gap_coverage_ms = change.coverage_ms,
        gap_cpu_some_us = change.cpu.0,
        gap_cpu_full_us = change.cpu.1,
        gap_memory_some_us = change.memory.0,
        gap_memory_full_us = change.memory.1,
        gap_io_some_us = change.io.0,
        gap_io_full_us = change.io.1,
        gap_memory_high_events = change.events.0,
        gap_memory_max_events = change.events.1,
        gap_oom_events = change.events.2,
        gap_oom_kill_events = change.events.3,
        gap_local_high_events = change.events_local.0,
        gap_local_max_events = change.events_local.1,
        gap_memory_current = opt(sample.current),
        gap_memory_high_limit = opt(sample.high),
        gap_memory_max_limit = opt(sample.limit),
        "worker heartbeat gap scope"
    );
}

/// `path` and every ancestor below the root, nearest first.
fn ancestry(control_group: &str) -> Vec<Scope> {
    let Some(mut path) = relative_cgroup(control_group) else {
        return Vec::new();
    };
    let mut chain = Vec::new();
    loop {
        chain.push(Scope::Cgroup(path.clone()));
        match path.rsplit_once('/') {
            Some((parent, _)) => path = parent.to_owned(),
            None => break,
        }
    }
    chain
}

/// A cgroup v2 path as the kernel reports it (`/a/b.slice/c.service`),
/// relative to the mount. Refuses the root, empty segments, `.` and `..`.
fn relative_cgroup(control_group: &str) -> Option<String> {
    let relative = control_group.strip_prefix('/')?;
    if relative.is_empty()
        || relative.len() > 4096
        || relative
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return None;
    }
    Some(relative.to_owned())
}

fn own_cgroup() -> Option<String> {
    let text = std::fs::read_to_string("/proc/self/cgroup").ok()?;
    let path = unified_cgroup(&text)?;
    relative_cgroup(&path)
}

/// The cgroup v2 entry (`0::/path`) of a `/proc/<pid>/cgroup` file.
fn unified_cgroup(text: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.strip_prefix("0::"))
        .map(str::to_owned)
}

/// Resolves and verifies a systemd worker unit's cgroup. The query runs once
/// per connection; the peer process must be a member of the unit's cgroup.
async fn resolve_binding(unit: Option<&str>, peer_pid: Option<u32>) -> BindingState {
    let Some(unit) = unit else {
        return BindingState::Unavailable("direct_mode");
    };
    let Some(peer_pid) = peer_pid else {
        return BindingState::Unavailable("peer_unknown");
    };
    let mut command = tokio::process::Command::new("systemctl");
    command
        .args([
            "--user",
            "show",
            unit,
            "--property=ControlGroup",
            "--property=InvocationID",
        ])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let output = match tokio::time::timeout(BIND_TIMEOUT, command.output()).await {
        Ok(Ok(output)) if output.status.success() => output,
        Ok(_) => return BindingState::Unavailable("unit_query_failed"),
        Err(_) => return BindingState::Unavailable("unit_query_timeout"),
    };
    let Some(binding) = parse_unit_show(&String::from_utf8_lossy(&output.stdout)) else {
        return BindingState::Unavailable("unit_unresolved");
    };
    let control_group = binding.control_group.clone();
    let member = tokio::task::spawn_blocking(move || peer_member(peer_pid, &control_group))
        .await
        .unwrap_or(Err("peer_check_failed"));
    match member {
        Ok(()) => BindingState::Verified(binding),
        Err(reason) => BindingState::Unavailable(reason),
    }
}

fn parse_unit_show(text: &str) -> Option<Binding> {
    let mut control_group = None;
    let mut invocation = None;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("ControlGroup=") {
            control_group = Some(value.trim().to_owned());
        } else if let Some(value) = line.strip_prefix("InvocationID=") {
            invocation = Some(value.trim().to_owned());
        }
    }
    let control_group = control_group.filter(|path| relative_cgroup(path).is_some())?;
    let invocation = invocation
        .filter(|id| id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit()))?;
    Some(Binding {
        control_group,
        invocation,
    })
}

/// Reads the peer's cgroup through a `/proc/<pid>` directory handle. If the
/// process exits (and its PID is reused) after the handle opens, reads through
/// it fail instead of describing another process.
fn peer_member(pid: u32, control_group: &str) -> Result<(), &'static str> {
    use rustix::fs::{Mode, OFlags, openat};
    let proc_root = Root::open("/proc", Some(PROC_SUPER_MAGIC)).map_err(|_| "proc_unavailable")?;
    let directory = openat(
        &proc_root.fd,
        pid.to_string(),
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| "peer_exited")?;
    let text = read_bounded_at(&directory, "cgroup").map_err(|_| "peer_exited")?;
    let path = unified_cgroup(&text).ok_or("peer_cgroup_unknown")?;
    if path == control_group || path.starts_with(&format!("{control_group}/")) {
        Ok(())
    } else {
        Err("peer_outside_unit")
    }
}

/// A directory handle on a validated filesystem.
struct Root {
    fd: rustix::fd::OwnedFd,
}

impl Root {
    fn open(path: &str, magic: Option<i64>) -> io::Result<Self> {
        use rustix::fs::{Mode, OFlags, fstatfs, open};
        let fd = open(
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        if let Some(magic) = magic {
            #[allow(clippy::unnecessary_cast)]
            let found = fstatfs(&fd)?.f_type as i64;
            if found != magic {
                return Err(io::Error::other("unexpected filesystem"));
            }
        }
        Ok(Self { fd })
    }

    fn read(&self, relative: &str) -> io::Result<String> {
        read_bounded_at(&self.fd, relative)
    }
}

fn read_bounded_at<Fd: rustix::fd::AsFd>(directory: Fd, relative: &str) -> io::Result<String> {
    use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
    use std::io::Read as _;
    let fd = openat2(
        directory,
        relative,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let mut text = String::new();
    std::fs::File::from(fd)
        .take(MAX_COUNTER_FILE as u64)
        .read_to_string(&mut text)?;
    Ok(text)
}

/// Reads every scope on a blocking thread; one unreadable field never hides
/// the others.
async fn read_scopes(scopes: Vec<Scope>) -> Vec<(Scope, Sample)> {
    tokio::task::spawn_blocking(move || {
        let cgroups = Root::open("/sys/fs/cgroup", Some(CGROUP2_SUPER_MAGIC)).ok();
        let proc = Root::open("/proc", Some(PROC_SUPER_MAGIC)).ok();
        scopes
            .into_iter()
            .map(|scope| {
                let sample = match &scope {
                    Scope::Cgroup(path) => cgroups
                        .as_ref()
                        .map(|root| read_cgroup(root, path))
                        .unwrap_or_default(),
                    Scope::Host => proc.as_ref().map(read_host).unwrap_or_default(),
                };
                (scope, sample)
            })
            .collect()
    })
    .await
    .unwrap_or_default()
}

fn read_cgroup(root: &Root, path: &str) -> Sample {
    let file = |name: &str| root.read(&format!("{path}/{name}")).ok();
    Sample {
        cpu: file("cpu.pressure").as_deref().and_then(parse_psi),
        memory: file("memory.pressure").as_deref().and_then(parse_psi),
        io: file("io.pressure").as_deref().and_then(parse_psi),
        events: file("memory.events").as_deref().and_then(parse_events),
        events_local: file("memory.events.local")
            .as_deref()
            .and_then(parse_events),
        current: file("memory.current").as_deref().and_then(parse_limit),
        high: file("memory.high").as_deref().and_then(parse_limit),
        limit: file("memory.max").as_deref().and_then(parse_limit),
    }
}

fn read_host(proc: &Root) -> Sample {
    let file = |name: &str| proc.read(&format!("pressure/{name}")).ok();
    Sample {
        cpu: file("cpu").as_deref().and_then(parse_psi),
        memory: file("memory").as_deref().and_then(parse_psi),
        io: file("io").as_deref().and_then(parse_psi),
        ..Sample::default()
    }
}

/// `some ... total=N` and optional `full ... total=N` lines.
fn parse_psi(text: &str) -> Option<Psi> {
    let total = |prefix: &str| {
        text.lines()
            .find(|line| line.starts_with(prefix))
            .and_then(|line| {
                line.split_whitespace()
                    .find_map(|field| field.strip_prefix("total="))
            })
            .and_then(|value| value.parse().ok())
    };
    Some(Psi {
        some_us: total("some ")?,
        full_us: total("full "),
    })
}

fn parse_events(text: &str) -> Option<Events> {
    let mut events = Events::default();
    let mut any = false;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(' ') else {
            continue;
        };
        let Ok(value) = value.trim().parse() else {
            continue;
        };
        let slot = match key {
            "high" => &mut events.high,
            "max" => &mut events.max,
            "oom" => &mut events.oom,
            "oom_kill" => &mut events.oom_kill,
            _ => continue,
        };
        *slot = value;
        any = true;
    }
    any.then_some(events)
}

fn parse_limit(text: &str) -> Option<u64> {
    match text.trim() {
        "max" => Some(u64::MAX),
        value => value.parse().ok(),
    }
}

#[cfg(test)]
#[path = "liveness_tests.rs"]
mod tests;
