//! Heartbeat-gap attribution (`docs/host-isolation-design.md`, L0).
//!
//! WHY: workers have been isolated after 48-58 s without a heartbeat while the
//! host ran at load above 400, and the mechanism is not established. Shared
//! `MemoryHigh` throttling, CPU, I/O, one large frame and a broker stall each
//! call for a different remedy. This module records what the broker saw and
//! the cgroup pressure around each gap, without changing any decision.
//!
//! The connection reader, the heartbeat arm and the heartbeat sweep only update
//! atomics on their own connection and `try_send` a small notice; no lock, scan
//! or await is added to them. Everything else runs in one sampler task: unit
//! queries in a two-lane bounded bind pool, counter reads on a blocking thread
//! every 10 s with deduplicated scopes, and records built from those periodic
//! samples.

use super::liveness_counters::{
    self as counters, CgroupDir, PeerHandle, Sample, UNAVAILABLE, saturate,
};
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
    sync::{Semaphore, mpsc},
};

/// A heartbeat absent for longer than this is recorded. Workers beat every
/// 10 s; the broker isolates after 45 s of frame silence.
pub(super) const GAP_THRESHOLD: Duration = Duration::from_secs(20);
const NOTICE_CAPACITY: usize = 64;
const SAMPLE_PERIOD: Duration = Duration::from_secs(10);
/// Samples kept per scope: 120 s covers the 45-60 s isolation window plus the
/// baseline before it.
const SAMPLE_HISTORY: usize = 12;
const MAX_WORKER_SCOPES: usize = 256;
const MAX_SHARED_SCOPES: usize = 64;
const MAX_PENDING_GAPS: usize = 64;
const BIND_LANES: usize = 2;
const BIND_QUEUE: usize = 64;
const BIND_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_UNIT_SHOW: u64 = 4096;
const MAX_IDENTIFIER: usize = 128;
const MAX_TEXT: usize = 240;
const LAG_TICK: Duration = Duration::from_millis(250);
/// Lag observations kept: 180 s at [`LAG_TICK`].
const LAG_HISTORY: usize = 720;

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

/// `text` cut to `limit` bytes on a character boundary, and whether it was.
fn bounded(text: &str, limit: usize) -> (String, bool) {
    if text.len() <= limit {
        return (text.to_owned(), false);
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_owned(), true)
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

/// What the broker had seen on a connection when a decision was taken. Frozen
/// into the notice, so later frames cannot overwrite it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Evidence {
    heartbeat_age_ms: Option<u64>,
    frame_age_ms: Option<u64>,
    progress_age_ms: Option<u64>,
    phase: u8,
    declared: u32,
    read: u64,
    /// Time since the first byte of a frame still being read.
    current_frame_ms: Option<u64>,
    /// Time waited for the first byte of the next frame.
    waiting_ms: Option<u64>,
    /// Since the previous heartbeat: the slowest completed frame (first byte
    /// to decoded), its size, the longest wait for a first byte, and frames.
    slowest_frame_ms: u64,
    slowest_frame_bytes: u64,
    longest_idle_ms: u64,
    frames: u64,
}

/// Clocks and frame progress of one worker connection. Written only by its
/// reader task, the heartbeat arm and the sweep; read by the sampler.
pub(super) struct Connection {
    session: String,
    connection: u64,
    epoch: String,
    unit: Option<String>,
    peer: Option<PeerHandle>,
    origin: Instant,
    opened_ms: u64,
    /// Milliseconds since `origin` plus one; zero means never.
    last_heartbeat: AtomicU64,
    last_frame: AtomicU64,
    last_progress: AtomicU64,
    frame_waiting_since: AtomicU64,
    frame_first_byte: AtomicU64,
    phase: AtomicU8,
    declared: AtomicU32,
    read: AtomicU64,
    slowest_frame_ms: AtomicU64,
    slowest_frame_bytes: AtomicU64,
    longest_idle_ms: AtomicU64,
    frames: AtomicU64,
    /// Heartbeat stamp (plus one) whose silence was already reported.
    threshold_reported: AtomicU64,
    /// Frame-clock stamp already covered by a backlog-extension record.
    deferred_reported: AtomicU64,
    notices: mpsc::Sender<Notice>,
    dropped: Arc<AtomicU64>,
}

impl Connection {
    fn now_ms(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_millis()).unwrap_or(u64::MAX - 1)
    }

    fn at(&self, ms: u64) -> Instant {
        self.origin + Duration::from_millis(ms)
    }

    fn stamp(clock: &AtomicU64, now: u64) -> u64 {
        clock.swap(now.saturating_add(1), Ordering::Relaxed)
    }

    fn age(clock: &AtomicU64, now: u64) -> Option<u64> {
        match clock.load(Ordering::Relaxed) {
            0 => None,
            stamp => Some(now.saturating_sub(stamp - 1)),
        }
    }

    fn begin_frame(&self) {
        let now = self.now_ms();
        self.frame_first_byte.store(0, Ordering::Relaxed);
        self.frame_waiting_since
            .store(now.saturating_add(1), Ordering::Relaxed);
        self.declared.store(0, Ordering::Relaxed);
        self.read.store(0, Ordering::Relaxed);
        self.phase.store(Phase::Header as u8, Ordering::Relaxed);
    }

    fn begin_payload(&self, declared: usize) {
        self.declared.store(
            u32::try_from(declared).unwrap_or(u32::MAX),
            Ordering::Relaxed,
        );
        self.read.store(0, Ordering::Relaxed);
        self.phase.store(Phase::Payload as u8, Ordering::Relaxed);
    }

    fn progressed(&self, bytes: usize) {
        let now = self.now_ms();
        if self.frame_first_byte.load(Ordering::Relaxed) == 0 {
            self.frame_first_byte
                .store(now.saturating_add(1), Ordering::Relaxed);
            if let Some(idle) = Self::age(&self.frame_waiting_since, now) {
                self.longest_idle_ms.fetch_max(idle, Ordering::Relaxed);
            }
        }
        self.read.fetch_add(bytes as u64, Ordering::Relaxed);
        Self::stamp(&self.last_progress, now);
    }

    fn frame_done(&self) {
        let now = self.now_ms();
        self.phase.store(Phase::Handling as u8, Ordering::Relaxed);
        Self::stamp(&self.last_frame, now);
        self.frames.fetch_add(1, Ordering::Relaxed);
        let took = Self::age(&self.frame_first_byte, now).unwrap_or(0);
        if took >= self.slowest_frame_ms.load(Ordering::Relaxed) {
            self.slowest_frame_ms.store(took, Ordering::Relaxed);
            self.slowest_frame_bytes.store(
                u64::from(self.declared.load(Ordering::Relaxed)),
                Ordering::Relaxed,
            );
        }
    }

    fn evidence(&self, now: u64) -> Evidence {
        let phase = self.phase.load(Ordering::Relaxed);
        let first_byte = Self::age(&self.frame_first_byte, now);
        let reading = phase != Phase::Handling as u8;
        Evidence {
            heartbeat_age_ms: Self::age(&self.last_heartbeat, now),
            frame_age_ms: Self::age(&self.last_frame, now),
            progress_age_ms: Self::age(&self.last_progress, now),
            phase,
            declared: self.declared.load(Ordering::Relaxed),
            read: self.read.load(Ordering::Relaxed),
            current_frame_ms: first_byte.filter(|_| reading),
            waiting_ms: (reading && first_byte.is_none())
                .then(|| Self::age(&self.frame_waiting_since, now))
                .flatten(),
            slowest_frame_ms: self.slowest_frame_ms.load(Ordering::Relaxed),
            slowest_frame_bytes: self.slowest_frame_bytes.load(Ordering::Relaxed),
            longest_idle_ms: self.longest_idle_ms.load(Ordering::Relaxed),
            frames: self.frames.load(Ordering::Relaxed),
        }
    }

    fn reset_interval(&self) {
        self.slowest_frame_ms.store(0, Ordering::Relaxed);
        self.slowest_frame_bytes.store(0, Ordering::Relaxed);
        self.longest_idle_ms.store(0, Ordering::Relaxed);
        self.frames.store(0, Ordering::Relaxed);
    }

    /// When the current heartbeat silence began: the last heartbeat, or the
    /// connection's admission.
    fn silence_since(&self) -> u64 {
        match self.last_heartbeat.load(Ordering::Relaxed) {
            0 => self.opened_ms,
            stamp => stamp - 1,
        }
    }

    /// Records a received heartbeat. One that closes a gap above
    /// [`GAP_THRESHOLD`] produces a notice carrying the interval's evidence.
    pub(super) fn heartbeat(self: &Arc<Self>) {
        let now = self.now_ms();
        let since = self.silence_since();
        let evidence = self.evidence(now);
        let reported = self.threshold_reported.load(Ordering::Relaxed)
            == self
                .last_heartbeat
                .load(Ordering::Relaxed)
                .saturating_add(1);
        Self::stamp(&self.last_heartbeat, now);
        self.reset_interval();
        if let Some(gap) = gap_ms(since, now) {
            let detail = reported.then(|| "closes a reported threshold crossing".to_owned());
            self.notify(GapKind::Heartbeat, since, now, gap, evidence, None, detail);
        }
    }

    /// The sweep kept this silent worker because its output is still queued.
    /// Recorded once per frame-silence episode.
    pub(super) fn deferred(self: &Arc<Self>, unread_bytes: u64, silent: Duration) {
        let frame = self.last_frame.load(Ordering::Relaxed);
        if self.deferred_reported.swap(frame, Ordering::Relaxed) == frame && frame != 0 {
            return;
        }
        let now = self.now_ms();
        let silent = u64::try_from(silent.as_millis()).unwrap_or(u64::MAX);
        let evidence = self.evidence(now);
        let since = now.saturating_sub(silent);
        let unread = Some(unread_bytes);
        self.notify(
            GapKind::Deferred,
            since,
            now,
            silent,
            evidence,
            unread,
            None,
        );
    }

    /// The sweep isolated this worker. The decision's own measurement is
    /// carried, since the connection may close before the record is written.
    pub(super) fn isolated(self: &Arc<Self>, detail: &str, unread_bytes: u64) {
        let now = self.now_ms();
        let since = self.silence_since();
        let evidence = self.evidence(now);
        let detail = Some(bounded(detail, MAX_TEXT).0);
        let gap = now.saturating_sub(since);
        let unread = Some(unread_bytes);
        self.notify(GapKind::Isolated, since, now, gap, evidence, unread, detail);
    }

    /// A heartbeat silence crossed the threshold with no heartbeat or
    /// isolation yet; reported once per silence.
    fn check_threshold(self: &Arc<Self>) -> Option<GapNotice> {
        let heartbeat = self.last_heartbeat.load(Ordering::Relaxed);
        let now = self.now_ms();
        let since = self.silence_since();
        let gap = gap_ms(since, now)?;
        let marker = heartbeat.saturating_add(1);
        if self.threshold_reported.swap(marker, Ordering::Relaxed) == marker {
            return None;
        }
        let evidence = self.evidence(now);
        Some(self.notice(GapKind::Threshold, since, now, gap, evidence, None, None))
    }

    #[allow(clippy::too_many_arguments)]
    fn notice(
        self: &Arc<Self>,
        kind: GapKind,
        since_ms: u64,
        at_ms: u64,
        gap_ms: u64,
        evidence: Evidence,
        unread_bytes: Option<u64>,
        detail: Option<String>,
    ) -> GapNotice {
        GapNotice {
            connection: Arc::clone(self),
            kind,
            gap_ms,
            since: self.at(since_ms),
            at: self.at(at_ms),
            evidence,
            unread_bytes,
            detail,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn notify(
        self: &Arc<Self>,
        kind: GapKind,
        since_ms: u64,
        at_ms: u64,
        gap_ms: u64,
        evidence: Evidence,
        unread_bytes: Option<u64>,
        detail: Option<String>,
    ) {
        let notice = self.notice(
            kind,
            since_ms,
            at_ms,
            gap_ms,
            evidence,
            unread_bytes,
            detail,
        );
        self.send(Notice::Gap(notice));
    }

    fn send(&self, notice: Notice) {
        if self.notices.try_send(notice).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// A gap only when strictly above the threshold.
pub(super) fn gap_ms(since_ms: u64, now_ms: u64) -> Option<u64> {
    let gap = now_ms.saturating_sub(since_ms);
    (u128::from(gap) > GAP_THRESHOLD.as_millis()).then_some(gap)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GapKind {
    /// A heartbeat arrived after a gap.
    Heartbeat,
    /// The silence crossed the threshold; no heartbeat has closed it yet.
    Threshold,
    /// The sweep extended isolation because worker output was still queued.
    Deferred,
    /// The sweep isolated the worker.
    Isolated,
}

impl GapKind {
    fn name(self) -> &'static str {
        match self {
            Self::Heartbeat => "heartbeat",
            Self::Threshold => "threshold",
            Self::Deferred => "backlog_extension",
            Self::Isolated => "isolated",
        }
    }
}

struct GapNotice {
    connection: Arc<Connection>,
    kind: GapKind,
    gap_ms: u64,
    since: Instant,
    at: Instant,
    evidence: Evidence,
    unread_bytes: Option<u64>,
    detail: Option<String>,
}

enum Notice {
    Bind(Arc<Connection>),
    Gap(GapNotice),
}

/// Broker-owned entry point. `connect` runs once per admitted worker
/// connection; nothing it starts can block that connection.
pub(super) struct Hub {
    origin: Instant,
    instance: u64,
    tx: mpsc::Sender<Notice>,
    rx: Mutex<Option<mpsc::Receiver<Notice>>>,
    dropped: Arc<AtomicU64>,
}

impl Default for Hub {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel(NOTICE_CAPACITY);
        let clock = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |time| time.as_nanos() as u64);
        Self {
            origin: Instant::now(),
            instance: clock ^ (u64::from(std::process::id()) << 32),
            tx,
            rx: Mutex::new(Some(rx)),
            dropped: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl Hub {
    /// Registers an admitted worker connection. `peer_pid` is the
    /// `SO_PEERCRED` PID; its `/proc` entry is pinned here, before any await,
    /// so a later PID reuse cannot be mistaken for the peer.
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
            session: bounded(session, MAX_IDENTIFIER).0,
            connection,
            epoch: bounded(epoch, MAX_IDENTIFIER).0,
            unit: unit.map(|unit| bounded(&unit, MAX_TEXT).0),
            peer: peer_pid.and_then(counters::open_peer),
            origin: self.origin,
            opened_ms,
            last_heartbeat: AtomicU64::new(0),
            last_frame: AtomicU64::new(0),
            last_progress: AtomicU64::new(0),
            frame_waiting_since: AtomicU64::new(0),
            frame_first_byte: AtomicU64::new(0),
            phase: AtomicU8::new(Phase::Handling as u8),
            declared: AtomicU32::new(0),
            read: AtomicU64::new(0),
            slowest_frame_ms: AtomicU64::new(0),
            slowest_frame_bytes: AtomicU64::new(0),
            longest_idle_ms: AtomicU64::new(0),
            frames: AtomicU64::new(0),
            threshold_reported: AtomicU64::new(0),
            deferred_reported: AtomicU64::new(0),
            notices: self.tx.clone(),
            dropped: Arc::clone(&self.dropped),
        });
        observed.send(Notice::Bind(Arc::clone(&observed)));
        observed
    }

    /// Runs the sampler until the broker stops. A second call returns at once.
    pub(super) async fn run(&self) {
        let Some(rx) = self.rx.lock().take() else {
            return;
        };
        let lag = Arc::new(Lag::default());
        let _probe = AbortOnDrop(tokio::spawn(probe_lag(Arc::clone(&lag))));
        let (jobs, jobs_rx) = mpsc::channel(BIND_QUEUE);
        let (results, results_rx) = mpsc::channel(BIND_QUEUE);
        let _lane = AbortOnDrop(tokio::spawn(bind_lane(jobs_rx, results)));
        Sampler::new(self.instance, Arc::clone(&self.dropped), lag, jobs)
            .run(rx, results_rx)
            .await;
    }
}

struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
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
    connection.begin_frame();
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
    connection.begin_payload(len);
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

/// Scheduling lateness of the broker's own runtime, one entry per tick.
#[derive(Default)]
struct Lag(Mutex<VecDeque<(Instant, u64)>>);

impl Lag {
    fn record(&self, at: Instant, late_ms: u64) {
        let mut ticks = self.0.lock();
        ticks.push_back((at, late_ms));
        while ticks.len() > LAG_HISTORY {
            ticks.pop_front();
        }
    }

    /// The largest lateness observed between `since` and one tick after `at`,
    /// or `None` when the history does not reach back to `since`.
    fn max_between(&self, since: Instant, at: Instant) -> Option<u64> {
        let ticks = self.0.lock();
        let covered = ticks.front().is_some_and(|(first, _)| *first <= since);
        let max = ticks
            .iter()
            .filter(|(tick, _)| *tick >= since && *tick <= at + LAG_TICK)
            .map(|(_, late)| *late)
            .max();
        max.filter(|_| covered)
    }
}

async fn probe_lag(lag: Arc<Lag>) {
    let mut interval = tokio::time::interval(LAG_TICK);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        let scheduled = interval.tick().await.into_std();
        let now = Instant::now();
        let late =
            u64::try_from(now.saturating_duration_since(scheduled).as_millis()).unwrap_or(u64::MAX);
        lag.record(now, late);
    }
}

#[derive(Clone)]
enum BindingState {
    Pending,
    Verified {
        control_group: String,
        invocation: String,
        directory: Arc<CgroupDir>,
    },
    Unavailable(&'static str),
}

impl BindingState {
    fn label(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Verified { .. } => "verified",
            Self::Unavailable(reason) => reason,
        }
    }
}

struct Bound {
    connection: Weak<Connection>,
    generation: u64,
    state: BindingState,
}

struct BindJob {
    connection: Arc<Connection>,
    generation: u64,
}

struct BindResult {
    connection: u64,
    generation: u64,
    state: BindingState,
}

/// Resolves bindings with at most [`BIND_LANES`] user-manager queries at
/// once. A full result queue drops the result; the binding then stays pending
/// and its gaps are recorded without scopes.
async fn bind_lane(mut jobs: mpsc::Receiver<BindJob>, results: mpsc::Sender<BindResult>) {
    let lanes = Arc::new(Semaphore::new(BIND_LANES));
    while let Some(job) = jobs.recv().await {
        let Ok(permit) = Arc::clone(&lanes).acquire_owned().await else {
            return;
        };
        let results = results.clone();
        tokio::spawn(async move {
            let state = resolve_binding(&job.connection).await;
            let _ = results.try_send(BindResult {
                connection: job.connection.connection,
                generation: job.generation,
                state,
            });
            drop(permit);
        });
    }
}

/// Resolves and verifies a systemd worker unit's cgroup: the unit's
/// `ControlGroup` directory is pinned, then the pinned peer process must be a
/// member of it.
async fn resolve_binding(connection: &Arc<Connection>) -> BindingState {
    let Some(unit) = connection.unit.as_deref() else {
        return BindingState::Unavailable("direct_mode");
    };
    if connection.peer.is_none() {
        return BindingState::Unavailable("peer_unknown");
    }
    let shown = match tokio::time::timeout(BIND_TIMEOUT, show_unit(unit)).await {
        Ok(Ok(text)) => text,
        Ok(Err(_)) => return BindingState::Unavailable("unit_query_failed"),
        Err(_) => return BindingState::Unavailable("unit_query_timeout"),
    };
    let Some((control_group, invocation)) = parse_unit_show(&shown) else {
        return BindingState::Unavailable("unit_unresolved");
    };
    let connection = Arc::clone(connection);
    let verified = tokio::task::spawn_blocking(move || {
        let directory = counters::open_cgroup(&control_group).map_err(|_| "cgroup_unavailable")?;
        let peer = connection.peer.as_ref().ok_or("peer_unknown")?;
        counters::peer_member(peer, &control_group)?;
        Ok::<_, &'static str>((control_group, directory))
    })
    .await
    .unwrap_or(Err("peer_check_failed"));
    match verified {
        Ok((control_group, directory)) => BindingState::Verified {
            control_group,
            invocation,
            directory: Arc::new(directory),
        },
        Err(reason) => BindingState::Unavailable(reason),
    }
}

async fn show_unit(unit: &str) -> io::Result<String> {
    let mut child = tokio::process::Command::new("systemctl")
        .args([
            "--user",
            "show",
            unit,
            "--property=ControlGroup",
            "--property=InvocationID",
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let mut text = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        stdout.take(MAX_UNIT_SHOW).read_to_end(&mut text).await?;
    }
    if !child.wait().await?.success() {
        return Err(io::Error::other("systemctl show failed"));
    }
    Ok(String::from_utf8_lossy(&text).into_owned())
}

fn parse_unit_show(text: &str) -> Option<(String, String)> {
    let mut control_group = None;
    let mut invocation = None;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("ControlGroup=") {
            control_group = Some(value.trim().to_owned());
        } else if let Some(value) = line.strip_prefix("InvocationID=") {
            invocation = Some(value.trim().to_owned());
        }
    }
    let control_group = control_group.filter(|path| counters::relative_cgroup(path).is_some())?;
    let invocation = invocation
        .filter(|id| id.len() == 32 && id.bytes().all(|byte| byte.is_ascii_hexdigit()))?;
    Some((control_group, invocation))
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Key {
    Worker(u64),
    Ancestor(String),
    Broker,
    Host,
}

enum Target {
    Worker(u64, Arc<CgroupDir>),
    Path(Key, String),
    Host,
}

type History = VecDeque<(Instant, Sample)>;

struct Sampler {
    instance: u64,
    dropped: Arc<AtomicU64>,
    reported_dropped: u64,
    lag: Arc<Lag>,
    jobs: mpsc::Sender<BindJob>,
    enabled: bool,
    generation: u64,
    bound: HashMap<u64, Bound>,
    history: HashMap<Key, History>,
    broker_cgroup: Option<String>,
    pending: VecDeque<GapNotice>,
    gap_ids: u64,
}

impl Sampler {
    fn new(
        instance: u64,
        dropped: Arc<AtomicU64>,
        lag: Arc<Lag>,
        jobs: mpsc::Sender<BindJob>,
    ) -> Self {
        Self {
            instance,
            dropped,
            reported_dropped: 0,
            lag,
            jobs,
            enabled: sampler_enabled(),
            generation: 0,
            bound: HashMap::new(),
            history: HashMap::new(),
            broker_cgroup: None,
            pending: VecDeque::new(),
            gap_ids: 0,
        }
    }

    async fn run(
        mut self,
        mut rx: mpsc::Receiver<Notice>,
        mut results: mpsc::Receiver<BindResult>,
    ) {
        self.broker_cgroup = tokio::task::spawn_blocking(counters::broker_cgroup)
            .await
            .ok()
            .flatten();
        let mut tick = tokio::time::interval(SAMPLE_PERIOD);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                notice = rx.recv() => match notice {
                    Some(Notice::Bind(connection)) => self.bind(&connection),
                    Some(Notice::Gap(gap)) => self.queue(gap),
                    None => return,
                },
                Some(result) = results.recv() => self.bound(result),
                _ = tick.tick() => self.tick().await,
            }
        }
    }

    fn submit(&self, connection: &Arc<Connection>) -> BindingState {
        let job = BindJob {
            connection: Arc::clone(connection),
            generation: self.generation,
        };
        match self.jobs.try_send(job) {
            Ok(()) => BindingState::Pending,
            Err(_) => BindingState::Unavailable("bind_queue_full"),
        }
    }

    fn bind(&mut self, connection: &Arc<Connection>) {
        self.bound
            .retain(|_, bound| bound.connection.strong_count() > 0);
        let state = if self.enabled {
            self.submit(connection)
        } else {
            BindingState::Unavailable("sampler_disabled")
        };
        self.bound.insert(
            connection.connection,
            Bound {
                connection: Arc::downgrade(connection),
                generation: self.generation,
                state,
            },
        );
    }

    fn bound(&mut self, result: BindResult) {
        let Some(bound) = self.bound.get_mut(&result.connection) else {
            return;
        };
        // A query started before a kill-switch transition is discarded.
        if bound.generation != result.generation || result.generation != self.generation {
            return;
        }
        bound.state = result.state;
        self.history.remove(&Key::Worker(result.connection));
    }

    fn queue(&mut self, gap: GapNotice) {
        if self.pending.len() >= MAX_PENDING_GAPS {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        self.pending.push_back(gap);
        if !self.enabled {
            self.flush();
        }
    }

    fn live(&self) -> Vec<Arc<Connection>> {
        self.bound
            .values()
            .filter_map(|bound| bound.connection.upgrade())
            .collect()
    }

    /// Applies a kill-switch edit: enabling rebinds every live connection,
    /// disabling drops all samples and records pending gaps without scopes.
    fn reconcile(&mut self) {
        let enabled = sampler_enabled();
        if enabled == self.enabled {
            return;
        }
        self.enabled = enabled;
        self.generation += 1;
        self.history.clear();
        let generation = self.generation;
        for connection in self.live() {
            let state = if enabled {
                self.submit(&connection)
            } else {
                BindingState::Unavailable("sampler_disabled")
            };
            if let Some(bound) = self.bound.get_mut(&connection.connection) {
                bound.generation = generation;
                bound.state = state;
            }
        }
        if !enabled {
            self.flush();
        }
    }

    async fn tick(&mut self) {
        self.report_drops();
        self.bound
            .retain(|_, bound| bound.connection.strong_count() > 0);
        self.reconcile();
        for connection in self.live() {
            if let Some(gap) = connection.check_threshold() {
                self.queue(gap);
            }
        }
        if self.enabled {
            let plan = self.plan();
            let keys: HashSet<Key> = plan.iter().map(target_key).collect();
            let samples = read_plan(plan).await;
            let now = Instant::now();
            self.history.retain(|key, _| keys.contains(key));
            for (key, sample) in samples {
                let ring = self.history.entry(key).or_default();
                ring.push_back((now, sample));
                while ring.len() > SAMPLE_HISTORY {
                    ring.pop_front();
                }
            }
        }
        self.flush();
    }

    /// Scopes of all verified live bindings: worker cgroups first, then their
    /// shared ancestors once each, both capped; the broker and host are
    /// always included.
    fn plan(&self) -> Vec<Target> {
        let mut verified: Vec<_> = self
            .bound
            .iter()
            .filter(|(_, bound)| bound.connection.strong_count() > 0)
            .filter_map(|(id, bound)| match &bound.state {
                BindingState::Verified {
                    control_group,
                    directory,
                    ..
                } => Some((*id, control_group.clone(), Arc::clone(directory))),
                _ => None,
            })
            .collect();
        verified.sort_by_key(|(id, _, _)| *id);
        verified.truncate(MAX_WORKER_SCOPES);
        let mut seen = HashSet::new();
        let mut shared = Vec::new();
        for (_, control_group, _) in &verified {
            let Some(relative) = counters::relative_cgroup(control_group) else {
                continue;
            };
            for ancestor in counters::ancestors(&relative) {
                if shared.len() < MAX_SHARED_SCOPES && seen.insert(ancestor.clone()) {
                    shared.push(Target::Path(Key::Ancestor(ancestor.clone()), ancestor));
                }
            }
        }
        let mut plan: Vec<Target> = verified
            .into_iter()
            .map(|(id, _, directory)| Target::Worker(id, directory))
            .collect();
        plan.extend(shared);
        if let Some(own) = &self.broker_cgroup {
            plan.push(Target::Path(Key::Broker, own.clone()));
        }
        plan.push(Target::Host);
        plan
    }

    fn report_drops(&mut self) {
        let dropped = self.dropped.load(Ordering::Relaxed);
        if dropped > self.reported_dropped {
            tracing::warn!(
                target: "cowboy_machine::heartbeat_gap",
                event_name = "cowboy.machine.heartbeat_gap.dropped",
                gap_instance = self.instance,
                count = dropped - self.reported_dropped,
                "heartbeat gap notices dropped; this observation window is incomplete"
            );
            self.reported_dropped = dropped;
        }
    }

    fn flush(&mut self) {
        while let Some(gap) = self.pending.pop_front() {
            self.record(&gap);
        }
    }

    fn record(&mut self, gap: &GapNotice) {
        self.gap_ids += 1;
        let gap_id = self.gap_ids;
        let observed = &gap.connection;
        let binding = self
            .bound
            .get(&observed.connection)
            .map_or(BindingState::Unavailable("unbound"), |bound| {
                bound.state.clone()
            });
        let evidence = &gap.evidence;
        let opt = |value: Option<u64>| value.map_or(UNAVAILABLE, saturate);
        let lag = self.lag.max_between(gap.since, gap.at);
        let invocation = match &binding {
            BindingState::Verified { invocation, .. } => invocation.as_str(),
            _ => "",
        };
        tracing::warn!(
            target: "cowboy_machine::heartbeat_gap",
            event_name = "cowboy.machine.heartbeat_gap",
            session = %observed.session,
            gap_worker_epoch = %observed.epoch,
            gap_connection_id = observed.connection,
            gap_instance = self.instance,
            gap_id,
            gap_kind = gap.kind.name(),
            gap_ms = gap.gap_ms,
            gap_unread_bytes = opt(gap.unread_bytes),
            gap_heartbeat_age_ms = opt(evidence.heartbeat_age_ms),
            gap_frame_age_ms = opt(evidence.frame_age_ms),
            gap_progress_age_ms = opt(evidence.progress_age_ms),
            gap_frame_phase = Phase::name(evidence.phase),
            gap_frame_declared_bytes = evidence.declared,
            gap_frame_read_bytes = evidence.read,
            gap_frame_current_ms = opt(evidence.current_frame_ms),
            gap_frame_waiting_ms = opt(evidence.waiting_ms),
            gap_slowest_frame_ms = evidence.slowest_frame_ms,
            gap_slowest_frame_bytes = evidence.slowest_frame_bytes,
            gap_longest_idle_ms = evidence.longest_idle_ms,
            gap_frames = evidence.frames,
            gap_broker_lag_ms = opt(lag),
            gap_binding = binding.label(),
            gap_invocation = invocation,
            gap_unit = observed.unit.as_deref().unwrap_or(""),
            gap_dropped_notices = self.dropped.load(Ordering::Relaxed),
            gap_sampler_enabled = self.enabled,
            gap_detail = gap.detail.as_deref().unwrap_or(""),
            "worker heartbeat gap"
        );
        let BindingState::Verified { control_group, .. } = binding else {
            return;
        };
        if !self.enabled {
            return;
        }
        let mut scopes = vec![(
            Key::Worker(observed.connection),
            "worker",
            control_group.clone(),
        )];
        if let Some(relative) = counters::relative_cgroup(&control_group) {
            for ancestor in counters::ancestors(&relative) {
                scopes.push((Key::Ancestor(ancestor.clone()), "ancestor", ancestor));
            }
        }
        if let Some(own) = &self.broker_cgroup {
            scopes.push((Key::Broker, "broker", own.clone()));
        }
        scopes.push((Key::Host, "host", String::new()));
        for (depth, (key, kind, path)) in scopes.into_iter().enumerate() {
            let Some(ring) = self.history.get(&key) else {
                continue;
            };
            let scope = Scope {
                instance: self.instance,
                gap_id,
                depth,
                kind,
                path: &path,
            };
            log_scope(&scope, &observed.session, ring, gap);
        }
    }
}

fn target_key(target: &Target) -> Key {
    match target {
        Target::Worker(id, _) => Key::Worker(*id),
        Target::Path(key, _) => key.clone(),
        Target::Host => Key::Host,
    }
}

/// Reads every planned scope on one blocking thread; one unreadable field
/// never hides the others.
async fn read_plan(plan: Vec<Target>) -> Vec<(Key, Sample)> {
    tokio::task::spawn_blocking(move || {
        plan.into_iter()
            .map(|target| {
                let key = target_key(&target);
                let sample = match &target {
                    Target::Worker(_, directory) => counters::read_cgroup(directory),
                    Target::Path(_, path) => counters::read_path(path),
                    Target::Host => counters::read_host(),
                };
                (key, sample)
            })
            .collect()
    })
    .await
    .unwrap_or_default()
}

struct Scope<'a> {
    instance: u64,
    gap_id: u64,
    depth: usize,
    kind: &'static str,
    path: &'a str,
}

/// Milliseconds from `earlier` to `later`, negative when `later` is earlier.
fn signed_ms(later: Instant, earlier: Instant) -> i64 {
    match later.checked_duration_since(earlier) {
        Some(ahead) => saturate(u64::try_from(ahead.as_millis()).unwrap_or(u64::MAX)),
        None => {
            -saturate(u64::try_from(earlier.duration_since(later).as_millis()).unwrap_or(u64::MAX))
        }
    }
}

fn log_scope(scope: &Scope<'_>, session: &str, ring: &History, gap: &GapNotice) {
    let Some((end_at, end)) = ring.back() else {
        return;
    };
    let Some((start_at, start)) = counters::baseline(ring, gap.since) else {
        return;
    };
    let change = counters::delta(start, end);
    let (cgroup, truncated) = bounded(scope.path, MAX_TEXT);
    let opt = |value: Option<u64>| value.map_or(UNAVAILABLE, saturate);
    tracing::warn!(
        target: "cowboy_machine::heartbeat_gap",
        event_name = "cowboy.machine.heartbeat_gap.scope",
        session = %session,
        gap_instance = scope.instance,
        gap_id = scope.gap_id,
        gap_scope_kind = scope.kind,
        gap_scope_depth = scope.depth as u64,
        gap_cgroup = cgroup.as_str(),
        gap_cgroup_truncated = truncated,
        gap_baseline_lead_ms = signed_ms(gap.since, *start_at),
        gap_end_lag_ms = signed_ms(*end_at, gap.at),
        gap_coverage_ms = signed_ms(*end_at, *start_at),
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
        gap_memory_current = opt(end.current),
        gap_memory_high_limit = opt(end.high),
        gap_memory_max_limit = opt(end.limit),
        "worker heartbeat gap scope"
    );
}

#[cfg(test)]
#[path = "liveness_tests.rs"]
mod tests;
