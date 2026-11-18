//! An in-memory [`Host`] for unprivileged tests: users map to the test
//! process's own ids, mounts are recorded, and units are fake processes the
//! test finishes explicitly (or `stop_unit` kills).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use anyhow::{Result, bail};

use crate::host::{Host, UnitChild, UnitStatus, UnitStdio};

#[derive(Default)]
pub struct State {
    pub users: HashMap<String, (u32, u32)>,
    pub removed_users: Vec<String>,
    pub mounted: HashSet<PathBuf>,
    pub units: HashMap<String, String>,
    pub exits: HashMap<String, Option<i32>>,
    pub spawned: Vec<Vec<String>>,
    pub stdio: Vec<UnitStdio>,
    /// Units whose `systemd-run` client was killed.
    pub killed: HashSet<String>,
    next_invocation: u64,
}

#[derive(Default)]
pub struct Inner {
    pub state: Mutex<State>,
    signal: Condvar,
}

pub struct FakeHost {
    pub inner: Arc<Inner>,
    pub free: AtomicU64,
    /// The next this many `stop_unit` calls fail.
    pub stop_failures: AtomicU64,
    /// `systemd-run` clients submit their unit only on [`FakeHost::submit`].
    pub defer_submission: std::sync::atomic::AtomicBool,
    /// The next this many `unit_status` calls fail.
    pub status_failures: AtomicU64,
    /// `systemd-run` clients exit right after submitting their unit, which
    /// systemd keeps running.
    pub client_exits_on_spawn: std::sync::atomic::AtomicBool,
}

impl FakeHost {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Arc::default(),
            free: AtomicU64::new(u64::MAX),
            stop_failures: AtomicU64::new(0),
            defer_submission: std::sync::atomic::AtomicBool::new(false),
            status_failures: AtomicU64::new(0),
            client_exits_on_spawn: std::sync::atomic::AtomicBool::new(false),
        })
    }

    /// A deferred client submits its unit now; `false` when it was killed
    /// first (a killed client never submits).
    pub fn submit(&self, unit: &str) -> bool {
        let mut state = self.state();
        if state.killed.contains(unit) {
            return false;
        }
        state.next_invocation += 1;
        let invocation = format!("{:032x}", state.next_invocation);
        state.units.insert(unit.to_owned(), invocation);
        true
    }

    pub fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.inner.state.lock().unwrap()
    }

    /// The unit process exits with `code` (`None`: killed).
    pub fn finish(&self, unit: &str, code: Option<i32>) {
        let mut state = self.state();
        state.units.remove(unit);
        state.exits.insert(unit.to_owned(), code);
        drop(state);
        self.inner.signal.notify_all();
    }

    /// The unit's `systemd-run` client exits with `code` while systemd still
    /// runs the unit.
    pub fn exit_client_only(&self, unit: &str, code: Option<i32>) {
        self.state().exits.insert(unit.to_owned(), code);
        self.inner.signal.notify_all();
    }

    /// systemd finishes a unit whose client already exited.
    pub fn end_unit(&self, unit: &str) {
        self.state().units.remove(unit);
    }

    pub fn unit_status_of(&self, unit: &str) -> Option<String> {
        self.state().units.get(unit).cloned()
    }
}

struct FakeChild {
    inner: Arc<Inner>,
    unit: String,
}

impl UnitChild for FakeChild {
    fn wait(&mut self) -> Result<Option<i32>> {
        let mut state = self.inner.state.lock().unwrap();
        loop {
            if let Some(exit) = state.exits.get(&self.unit) {
                return Ok(*exit);
            }
            state = self.inner.signal.wait(state).unwrap();
        }
    }

    fn canceller(&self) -> Box<dyn Fn() + Send + Sync> {
        let (inner, unit) = (Arc::clone(&self.inner), self.unit.clone());
        Box::new(move || {
            let mut state = inner.state.lock().unwrap();
            state.killed.insert(unit.clone());
            state.exits.entry(unit.clone()).or_insert(None);
            drop(state);
            inner.signal.notify_all();
        })
    }
}

impl Host for FakeHost {
    fn ensure_user(&self, user: &str) -> Result<(u32, u32)> {
        let ids = (
            rustix::process::getuid().as_raw(),
            rustix::process::getgid().as_raw(),
        );
        self.state().users.insert(user.to_owned(), ids);
        Ok(ids)
    }

    fn remove_user(&self, user: &str) -> Result<()> {
        let mut state = self.state();
        state.users.remove(user);
        state.removed_users.push(user.to_owned());
        Ok(())
    }

    fn format_image(&self, image: &Path, _inodes: u64, _owner: (u32, u32)) -> Result<()> {
        if !image.is_file() {
            bail!("image file missing");
        }
        Ok(())
    }

    fn mount_image(&self, _image: &Path, target: &Path) -> Result<()> {
        self.state().mounted.insert(target.to_owned());
        Ok(())
    }

    fn unmount(&self, target: &Path) -> Result<()> {
        self.state().mounted.remove(target);
        Ok(())
    }

    fn is_mounted(&self, target: &Path) -> Result<bool> {
        Ok(self.state().mounted.contains(target))
    }

    fn spawn_unit(&self, argv: &[String], stdio: UnitStdio) -> Result<Box<dyn UnitChild>> {
        let unit = argv
            .iter()
            .find_map(|a| a.strip_prefix("--unit="))
            .unwrap()
            .to_owned();
        let mut state = self.state();
        if !self.defer_submission.load(Ordering::Relaxed) {
            state.next_invocation += 1;
            let invocation = format!("{:032x}", state.next_invocation);
            state.units.insert(unit.clone(), invocation);
        }
        state.spawned.push(argv.to_vec());
        state.stdio.push(stdio);
        if self.client_exits_on_spawn.load(Ordering::Relaxed) {
            state.exits.insert(unit.clone(), Some(1));
            drop(state);
            self.inner.signal.notify_all();
            // Let the waiter observe the exit before the starter looks.
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        Ok(Box::new(FakeChild {
            inner: Arc::clone(&self.inner),
            unit,
        }))
    }

    fn unit_status(&self, unit: &str) -> Result<Option<UnitStatus>> {
        if self
            .status_failures
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
            .is_ok()
        {
            bail!("systemctl show {unit} failed");
        }
        Ok(self.state().units.get(unit).map(|invocation| UnitStatus {
            active_state: "active".into(),
            invocation_id: Some(invocation.clone()),
            job_pending: false,
            cgroup_populated: false,
        }))
    }

    fn stop_unit(&self, unit: &str) -> Result<()> {
        if self
            .stop_failures
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_sub(1))
            .is_ok()
        {
            bail!("systemctl stop {unit} failed");
        }
        if self.state().units.contains_key(unit) {
            self.finish(unit, None);
        }
        Ok(())
    }

    fn list_units(&self, pattern: &str) -> Result<Vec<String>> {
        let prefix = pattern.trim_end_matches('*');
        let mut units: Vec<String> = self
            .state()
            .units
            .keys()
            .filter(|u| u.starts_with(prefix))
            .cloned()
            .collect();
        units.sort();
        Ok(units)
    }

    fn boot_id(&self) -> Result<String> {
        Ok("boot-1".into())
    }

    fn free_bytes(&self, _path: &Path) -> Result<u64> {
        Ok(self.free.load(Ordering::Relaxed))
    }
}
