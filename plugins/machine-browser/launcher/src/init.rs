//! The namespace init: PID 1 of the instance. It owns the mounts, the control
//! socket, the three display and browser processes, every helper and every
//! snapshot copy, reaps everything, and stays alive until `stop` completes
//! or the unit's deadline kills it.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::{Read as _, Write as _};
use std::os::fd::OwnedFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::io::Errno;
use rustix::process::{Pid, Signal, WaitOptions, WaitStatus, kill_process, waitpid};
use serde_json::{Map, Value, json};

use crate::args::Args;
use crate::children;
use crate::control::{self, FailureKind, LineBuffer};
use crate::supervisor::{exit_code, log, read_signal};
use crate::{cgroup, raw};

pub(crate) const SIGNALS: &[i32] = &[libc::SIGTERM, libc::SIGINT, libc::SIGHUP, libc::SIGCHLD];
pub(crate) const MNT: &str = "/mnt";
pub(crate) const RUNTIME: &str = "/mnt/runtime";
pub(crate) const PROFILE: &str = "/mnt/profile";
pub(crate) const INSTANCE: &str = "/mnt/instance";
pub(crate) const SOCKETS: &str = "/mnt/sockets";
pub(crate) const SNAPSHOT: &str = "/mnt/snapshot";
pub(crate) const PUBLISH: &str = "/mnt/publish";
pub(crate) const INSTANCE_HOME: &str = "/mnt/instance/home";
pub(crate) const INSTANCE_RUN: &str = "/mnt/instance/run";
pub(crate) const INSTANCE_LOG: &str = "/mnt/instance/log";
pub(crate) const XAUTH_FILE: &str = "/mnt/instance/xauth";
pub(crate) const FONTS_CONF: &str = "/mnt/instance/fonts.conf";
pub(crate) const FONT_CACHE: &str = "/mnt/instance/fontconfig";
pub(crate) const RFB_SOCKET: &str = "/mnt/sockets/rfb.sock";
pub(crate) const NSSWITCH: &str = "/usr/share/cowboy-browser/nsswitch.conf";
pub(crate) const HELPER_TIMEOUT: Duration = Duration::from_secs(15);
pub(crate) const DISPLAY_TIMEOUT: Duration = Duration::from_secs(15);
pub(crate) const NAVIGATE_TIMEOUT: Duration = Duration::from_secs(20);
pub(crate) const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);
pub(crate) const KILL_GRACE: Duration = Duration::from_secs(3);
pub(crate) const FATAL_CLIENT_WAIT: Duration = Duration::from_secs(5);
pub(crate) const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
/// Graceful wait used when the unit itself is stopped (`SIGTERM`), inside
/// the unit's 20 s stop timeout.
pub(crate) const SIGNAL_STOP_GRACE: Duration = Duration::from_secs(10);
pub(crate) const LOG_TAIL_BYTES: usize = 4096;

pub struct Context {
    pub args: Args,
    pub cgroup_dir: PathBuf,
    pub listener: UnixListener,
    pub supervisor_pid: i32,
}

pub(crate) struct Fatal {
    pub(crate) kind: FailureKind,
    pub(crate) message: String,
}

impl Fatal {
    pub(crate) fn new(kind: FailureKind, error: &anyhow::Error) -> Self {
        Self {
            kind,
            message: format!("{error:#}"),
        }
    }
}

pub(crate) struct Init {
    pub(crate) args: Args,
    pub(crate) cgroup_unit: PathBuf,
    pub(crate) cgroup_display: PathBuf,
    pub(crate) cgroup_browser: PathBuf,
    pub(crate) supervisor_pid: i32,
    pub(crate) listener: UnixListener,
    pub(crate) client: Option<UnixStream>,
    pub(crate) client_serial: u64,
    pub(crate) lines: LineBuffer,
    pub(crate) signals: Option<OwnedFd>,
    pub(crate) stdin_null: Option<OwnedFd>,
    pub(crate) staging: Option<OwnedFd>,
    pub(crate) display_environment: Vec<(String, String)>,
    pub(crate) browser_environment: Vec<(String, String)>,
    pub(crate) display: Option<Pid>,
    pub(crate) window_manager: Option<Pid>,
    pub(crate) browser: Option<Pid>,
    pub(crate) start_times: BTreeMap<String, u64>,
    pub(crate) namespaces: BTreeMap<String, String>,
    pub(crate) uid_map: String,
    pub(crate) gid_map: String,
    pub(crate) label: String,
    pub(crate) sealed_label: Option<String>,
    pub(crate) mount_targets: Vec<String>,
    pub(crate) last_capability: u32,
    pub(crate) pending: VecDeque<String>,
    pub(crate) copy_job: Option<crate::snapshot::CopyJob>,
    pub(crate) deferred_exits: Vec<(Pid, WaitStatus)>,
    pub(crate) attachments: BTreeSet<String>,
    pub(crate) stopping: bool,
    pub(crate) terminate_requested: bool,
}

/// Never returns: serves the instance and exits with the unit's status.
pub fn main(context: Context) -> ! {
    let mut init = Init::new(context);
    if let Err(fatal) = init.setup() {
        init.fail(fatal.kind, &fatal.message);
    }
    let code = init.serve();
    raw::exit_now(i32::from(code))
}

/// Report a failure to the Machine before the event loop exists: wait a
/// bounded time for a client and send the `error` event.
pub fn deliver_fatal(listener: &UnixListener, kind: FailureKind, message: &str) {
    let line = control::event_error(kind, message);
    if let Some(mut client) = accept_within(listener, FATAL_CLIENT_WAIT) {
        let _ = client.set_nonblocking(false);
        let _ = client.write_all(line.as_bytes());
        let _ = client.shutdown(std::net::Shutdown::Both);
    }
}

fn accept_within(listener: &UnixListener, wait: Duration) -> Option<UnixStream> {
    let deadline = Instant::now() + wait;
    loop {
        match listener.accept() {
            Ok((stream, _)) => return Some(stream),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => return None,
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        let mut fds = [PollFd::new(&listener, PollFlags::IN)];
        let _ = poll(&mut fds, Some(&timespec(remaining)));
    }
}

pub(crate) fn timespec(duration: Duration) -> Timespec {
    Timespec {
        tv_sec: i64::try_from(duration.as_secs()).unwrap_or(i64::MAX),
        tv_nsec: i64::from(duration.subsec_nanos()),
    }
}

pub(crate) fn pid_number(pid: Pid) -> i32 {
    pid.as_raw_nonzero().get()
}

pub(crate) fn status_code(status: WaitStatus) -> Option<i32> {
    status
        .exit_status()
        .or_else(|| status.terminating_signal().map(|signal| 128 + signal))
}

impl Init {
    pub(crate) fn new(context: Context) -> Self {
        Self {
            args: context.args,
            cgroup_display: context.cgroup_dir.join("display"),
            cgroup_browser: context.cgroup_dir.join("browser"),
            cgroup_unit: context.cgroup_dir,
            supervisor_pid: context.supervisor_pid,
            listener: context.listener,
            client: None,
            client_serial: 0,
            lines: LineBuffer::new(),
            signals: None,
            stdin_null: None,
            staging: None,
            display_environment: Vec::new(),
            browser_environment: Vec::new(),
            display: None,
            window_manager: None,
            browser: None,
            start_times: BTreeMap::new(),
            namespaces: BTreeMap::new(),
            uid_map: String::new(),
            gid_map: String::new(),
            label: String::new(),
            sealed_label: None,
            mount_targets: Vec::new(),
            last_capability: 0,
            pending: VecDeque::new(),
            copy_job: None,
            deferred_exits: Vec::new(),
            attachments: BTreeSet::new(),
            stopping: false,
            terminate_requested: false,
        }
    }

    /// Block up to `timeout` for signals, then drain them: `SIGCHLD` reaps
    /// into the deferred list, termination signals are remembered.
    pub(crate) fn wait_signals(&mut self, timeout: Duration) {
        if let Some(signals) = &self.signals {
            let mut fds = [PollFd::new(signals, PollFlags::IN)];
            let _ = poll(&mut fds, Some(&timespec(timeout)));
        }
        self.drain_signals();
    }

    pub(crate) fn drain_signals(&mut self) {
        let mut saw_child = false;
        if let Some(signals) = &self.signals {
            loop {
                match read_signal(signals) {
                    Ok(libc::SIGCHLD) => saw_child = true,
                    Ok(_) => self.terminate_requested = true,
                    Err(_) => break,
                }
            }
        }
        if saw_child {
            self.reap();
        }
    }

    pub(crate) fn reap(&mut self) {
        loop {
            match waitpid(None, WaitOptions::NOHANG) {
                Ok(Some((pid, status))) => self.deferred_exits.push((pid, status)),
                Err(Errno::INTR) => {}
                Ok(None) | Err(_) => break,
            }
        }
    }

    /// Remove `pid`'s exit from the deferred list.
    pub(crate) fn take_exit(&mut self, pid: Pid) -> Option<WaitStatus> {
        self.reap();
        let index = self
            .deferred_exits
            .iter()
            .position(|(exited, _)| *exited == pid)?;
        Some(self.deferred_exits.remove(index).1)
    }

    /// Wait for `pid` to exit, killing it when `timeout` passes. Other
    /// children's exits stay deferred.
    pub(crate) fn wait_child(&mut self, pid: Pid, timeout: Duration) -> Result<WaitStatus> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.take_exit(pid) {
                return Ok(status);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                let _ = kill_process(pid, Signal::KILL);
                let killed = Instant::now() + KILL_GRACE;
                loop {
                    if let Some(_status) = self.take_exit(pid) {
                        bail!(
                            "helper timed out after {} ms and was killed",
                            timeout.as_millis()
                        );
                    }
                    let left = killed.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        bail!("helper timed out and did not die");
                    }
                    self.wait_signals(left.min(Duration::from_millis(100)));
                }
            }
            self.wait_signals(remaining.min(Duration::from_millis(100)));
        }
    }

    pub(crate) fn serve(&mut self) -> u8 {
        loop {
            if let Some(code) = self.process_deferred() {
                return code;
            }
            if self.terminate_requested && !self.stopping {
                log("init", "termination signal: stopping gracefully");
                let fields = self.stop(SIGNAL_STOP_GRACE, None);
                log(
                    "init",
                    &format!("stopped on signal: {}", Value::Object(fields)),
                );
                self.shutdown_display();
                return 0;
            }
            let (signals_ready, listener_ready, client_ready, copier_ready) = self.poll_once();
            if signals_ready {
                self.drain_signals();
            }
            if listener_ready {
                self.accept_client();
            }
            if client_ready && let Some(code) = self.serve_client() {
                return code;
            }
            if copier_ready {
                self.finish_copy_job();
            }
        }
    }

    pub(crate) fn poll_once(&mut self) -> (bool, bool, bool, bool) {
        let Some(signals) = &self.signals else {
            return (false, false, false, false);
        };
        let mut fds = vec![PollFd::new(signals, PollFlags::IN)];
        let mut has_client = false;
        match &self.client {
            Some(client) => {
                has_client = true;
                fds.push(PollFd::new(client, PollFlags::IN));
            }
            None => fds.push(PollFd::new(&self.listener, PollFlags::IN)),
        }
        if let Some(job) = &self.copy_job {
            fds.push(PollFd::new(&job.pipe, PollFlags::IN));
        }
        match poll(&mut fds, None) {
            Ok(_) => {}
            Err(Errno::INTR) => return (false, false, false, false),
            Err(error) => {
                log("init", &format!("poll failed: {error}"));
                return (true, false, false, false);
            }
        }
        let ready = |index: usize| {
            fds.get(index).is_some_and(|fd| {
                fd.revents()
                    .intersects(PollFlags::IN | PollFlags::HUP | PollFlags::ERR | PollFlags::NVAL)
            })
        };
        let signals_ready = ready(0);
        let second = ready(1);
        let copier_ready = ready(2);
        (
            signals_ready,
            !has_client && second,
            has_client && second,
            copier_ready,
        )
    }

    pub(crate) fn accept_client(&mut self) {
        match self.listener.accept() {
            Ok((stream, _)) => {
                if stream.set_nonblocking(true).is_err() {
                    return;
                }
                self.client = Some(stream);
                self.client_serial += 1;
                self.lines = LineBuffer::new();
                let pending: Vec<String> = self.pending.drain(..).collect();
                for line in pending {
                    self.send(&line);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => log("init", &format!("accept failed: {error}")),
        }
    }

    /// Read what the client sent and handle every complete command.
    pub(crate) fn serve_client(&mut self) -> Option<u8> {
        let mut buffer = [0_u8; 16 * 1024];
        let read = match self.client.as_mut()?.read(&mut buffer) {
            Ok(0) => {
                self.disconnect_client();
                return None;
            }
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return None,
            Err(_) => {
                self.disconnect_client();
                return None;
            }
        };
        let lines = match self.lines.push(&buffer[..read]) {
            Ok(lines) => lines,
            Err(message) => {
                log("init", &format!("dropping client: {message}"));
                self.disconnect_client();
                return None;
            }
        };
        for line in lines {
            if let Some(code) = self.handle_line(&line) {
                return Some(code);
            }
        }
        None
    }

    pub(crate) fn disconnect_client(&mut self) {
        self.client = None;
        self.lines = LineBuffer::new();
    }

    /// Exits reaped since the last pass: a dead browser, display server or
    /// window manager ends the instance; a finished copier completes its
    /// snapshot; everything else (reparented orphans) is just reaped.
    pub(crate) fn process_deferred(&mut self) -> Option<u8> {
        let exits = std::mem::take(&mut self.deferred_exits);
        for (pid, status) in exits {
            if Some(pid) == self.browser {
                self.browser = None;
                if self.stopping {
                    continue;
                }
                let code = status_code(status);
                log(
                    "init",
                    &format!(
                        "browser exited unexpectedly with {code:?}; log tail:\n{}",
                        children::log_tail(&format!("{INSTANCE_LOG}/browser.log"), LOG_TAIL_BYTES)
                            .trim()
                    ),
                );
                self.send(&control::event_exited("browser", code));
                self.stopping = true;
                self.cancel_copy_job("the browser exited");
                self.shutdown_display();
                return Some(exit_code(status));
            }
            let name = if Some(pid) == self.display {
                self.display = None;
                Some("display_server")
            } else if Some(pid) == self.window_manager {
                self.window_manager = None;
                Some("window_manager")
            } else {
                None
            };
            if let Some(name) = name {
                if self.stopping {
                    continue;
                }
                let code = status_code(status);
                log("init", &format!("{name} exited unexpectedly with {code:?}"));
                self.send(&control::event_exited(name, code));
                self.stopping = true;
                self.cancel_copy_job(&format!("the {name} exited"));
                self.shutdown_display();
                return Some(FailureKind::Internal.exit_code());
            }
            if self.copy_job.as_ref().is_some_and(|job| job.pid == pid) {
                self.finish_copy_job();
            }
        }
        None
    }

    /// The graceful stop: `SIGTERM` to the browser main process only, wait
    /// for the `browser/` cgroup to drain, kill what is left, then (when
    /// asked) copy the profile into the staging generation.
    pub(crate) fn stop(
        &mut self,
        graceful: Duration,
        snapshot_generation: Option<u64>,
    ) -> Map<String, Value> {
        self.stopping = true;
        self.cancel_copy_job("the instance is stopping");
        let graceful = self.drain_browser(graceful);
        let mut fields = Map::new();
        fields.insert("graceful".to_owned(), Value::Bool(graceful));
        if let Some(generation) = snapshot_generation {
            match self.snapshot_inline(generation) {
                Ok(stats) => {
                    fields.insert(
                        "snapshot".to_owned(),
                        json!({ "bytes": stats.bytes, "entries": stats.entries }),
                    );
                }
                Err(message) => {
                    log(
                        "init",
                        &format!("stop snapshot gen-{generation} failed: {message}"),
                    );
                    fields.insert("snapshot_error".to_owned(), Value::String(message));
                }
            }
        }
        fields
    }

    /// Returns whether every browser process left on `SIGTERM` within the
    /// grace period.
    pub(crate) fn drain_browser(&mut self, graceful: Duration) -> bool {
        if let Some(browser) = self.browser {
            let _ = kill_process(browser, Signal::TERM);
        }
        let deadline = Instant::now() + graceful;
        loop {
            if cgroup::is_empty(&self.cgroup_browser).unwrap_or(true) {
                return true;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            self.wait_signals(remaining.min(Duration::from_millis(100)));
        }
        log(
            "init",
            "browser did not drain in time; killing the browser cgroup",
        );
        let _ = cgroup::kill_all(&self.cgroup_browser);
        let deadline = Instant::now() + KILL_GRACE;
        while !cgroup::is_empty(&self.cgroup_browser).unwrap_or(true) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            self.wait_signals(remaining.min(Duration::from_millis(100)));
        }
        false
    }

    /// Stop the window manager and the display server, then kill anything
    /// left in either sub-cgroup.
    pub(crate) fn shutdown_display(&mut self) {
        self.stopping = true;
        for pid in [self.window_manager, self.display].into_iter().flatten() {
            let _ = kill_process(pid, Signal::TERM);
        }
        let deadline = Instant::now() + SHUTDOWN_GRACE;
        loop {
            let display_empty = cgroup::is_empty(&self.cgroup_display).unwrap_or(true);
            let browser_empty = cgroup::is_empty(&self.cgroup_browser).unwrap_or(true);
            if display_empty && browser_empty {
                break;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                let _ = cgroup::kill_all(&self.cgroup_display);
                let _ = cgroup::kill_all(&self.cgroup_browser);
                self.wait_signals(Duration::from_millis(200));
                break;
            }
            self.wait_signals(remaining.min(Duration::from_millis(100)));
        }
        self.reap();
        self.deferred_exits.clear();
        self.display = None;
        self.window_manager = None;
        self.browser = None;
    }

    /// Fatal failure: report it, end every child, exit with the kind's code.
    pub(crate) fn fail(&mut self, kind: FailureKind, message: &str) -> ! {
        log("init", &format!("fatal ({}): {message}", kind.as_str()));
        self.stopping = true;
        self.cancel_copy_job("the instance failed");
        let _ = cgroup::kill_all(&self.cgroup_browser);
        let _ = cgroup::kill_all(&self.cgroup_display);
        let line = control::event_error(kind, message);
        if self.client.is_some() {
            self.send(&line);
        } else {
            deliver_fatal(&self.listener, kind, message);
        }
        raw::exit_now(i32::from(kind.exit_code()))
    }

    /// Write a line to the connected client, or queue it for the next one.
    pub(crate) fn send(&mut self, line: &str) {
        let Some(client) = self.client.as_mut() else {
            self.pending.push_back(line.to_owned());
            return;
        };
        let bytes = line.as_bytes();
        let mut written = 0;
        let deadline = Instant::now() + WRITE_TIMEOUT;
        while written < bytes.len() {
            match client.write(&bytes[written..]) {
                Ok(0) => break,
                Ok(count) => written += count,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        break;
                    }
                    let mut fds = [PollFd::new(&*client, PollFlags::OUT)];
                    let _ = poll(&mut fds, Some(&timespec(remaining)));
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        if written < bytes.len() {
            log("init", "dropping an unresponsive control client");
            self.disconnect_client();
        }
    }
}
