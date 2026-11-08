//! Profile snapshots: binding a staging generation at `/mnt/publish`
//! through the retained staging descriptor, the forked copier a `snapshot`
//! command runs so the init stays responsive, the inline copy a `stop` with
//! `snapshot_generation` makes, and the replies and events either produces.

use std::fs::File;
use std::io::Read as _;
use std::os::fd::{AsFd, OwnedFd};

use anyhow::Result;
use rustix::fs::{AtFlags, Mode, OFlags};
use rustix::process::{Pid, Signal, getuid, kill_process};
use serde_json::{Map, Value, json};

use crate::control::{self, FailureKind};
use crate::copier::{self, Bounds, Exclusions};
use crate::init::{Init, KILL_GRACE, PROFILE, PUBLISH};
use crate::supervisor::log;
use crate::{mounts, raw};

/// A forked copier in flight.
pub(crate) struct CopyJob {
    pub(crate) generation: u64,
    pub(crate) pid: Pid,
    pub(crate) pipe: OwnedFd,
    /// The requesting command id and the client connection it came on.
    pub(crate) requester: Option<(Value, u64)>,
}

impl Init {
    /// Bind `staging/gen-<n>` at `/mnt/publish` through the retained
    /// staging descriptor.
    pub(crate) fn bind_publish(&self, generation: u64) -> Result<(), (FailureKind, String)> {
        let staging = self.staging.as_ref().ok_or_else(|| {
            (
                FailureKind::Internal,
                "staging descriptor missing".to_owned(),
            )
        })?;
        let name = format!("gen-{generation}");
        let target = rustix::fs::openat(
            staging,
            name.as_str(),
            OFlags::PATH | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|error| {
            (
                FailureKind::Host,
                format!("staging {name} is unavailable: {error}"),
            )
        })?;
        let stat = rustix::fs::statat(&target, "", AtFlags::EMPTY_PATH).map_err(|error| {
            (
                FailureKind::Host,
                format!("inspecting staging {name}: {error}"),
            )
        })?;
        if stat.st_uid != getuid().as_raw() {
            return Err((
                FailureKind::Host,
                format!("staging {name} is not owned by the Machine user"),
            ));
        }
        mounts::bind_fd(target.as_fd(), PUBLISH, false)
            .map_err(|error| (FailureKind::Sandbox, format!("{error:#}")))
    }

    pub(crate) fn copy_profile_to_publish(&self) -> Result<copier::Stats> {
        let source = copier::open_directory(PROFILE)?;
        let target = copier::open_directory(PUBLISH)?;
        copier::ensure_directory(&target, "the staging generation")?;
        copier::copy_tree(
            &source,
            &target,
            Bounds::for_profile(self.args.profile_bytes),
            &Exclusions::chromium(),
            true,
        )
    }

    /// Start an asynchronous copy of the profile into `staging/gen-<n>`; a
    /// forked copier does the work so the init stays responsive.
    pub(crate) fn start_copy_job(
        &mut self,
        generation: u64,
        requester: Option<Value>,
    ) -> Result<(), (FailureKind, String)> {
        if self.copy_job.is_some() {
            return Err((
                FailureKind::Internal,
                "a snapshot copy is already in progress".to_owned(),
            ));
        }
        if self.stopping {
            return Err((FailureKind::Internal, "the instance is stopping".to_owned()));
        }
        self.bind_publish(generation)?;
        let (read_end, write_end) = match rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC)
        {
            Ok(pipe) => pipe,
            Err(error) => {
                let _ = mounts::unmount_detached(PUBLISH);
                return Err((FailureKind::Internal, format!("copier pipe: {error}")));
            }
        };
        match raw::fork() {
            Err(error) => {
                let _ = mounts::unmount_detached(PUBLISH);
                Err((
                    FailureKind::Internal,
                    format!("forking the copier: {error}"),
                ))
            }
            Ok(raw::Fork::Child) => {
                drop(read_end);
                let report = match self.copy_profile_to_publish() {
                    Ok(stats) => format!("ok {} {}", stats.bytes, stats.entries),
                    Err(error) => format!("err {error:#}"),
                };
                let _ = rustix::io::write(&write_end, report.as_bytes());
                raw::exit_now(0)
            }
            Ok(raw::Fork::Parent(pid)) => {
                drop(write_end);
                self.copy_job = Some(CopyJob {
                    generation,
                    pid,
                    pipe: read_end,
                    requester: requester.map(|id| (id, self.client_serial)),
                });
                Ok(())
            }
        }
    }

    /// Collect the copier's result, unmount `/mnt/publish`, answer the
    /// requester (or queue a `snapshot_done` event for the next client).
    pub(crate) fn finish_copy_job(&mut self) {
        let Some(job) = self.copy_job.take() else {
            return;
        };
        let mut report = String::new();
        let mut pipe: File = job.pipe.into();
        let _ = pipe.read_to_string(&mut report);
        let _ = self.wait_child(job.pid, KILL_GRACE);
        if let Err(error) = mounts::unmount_detached(PUBLISH) {
            log("init", &format!("{error:#}"));
        }
        let outcome = parse_copier_report(&report);
        let same_client = job
            .requester
            .as_ref()
            .is_some_and(|(_, serial)| *serial == self.client_serial && self.client.is_some());
        match (outcome, job.requester) {
            (Ok(stats), Some((id, _))) if same_client => {
                let mut fields = Map::new();
                fields.insert("bytes".to_owned(), json!(stats.bytes));
                fields.insert("entries".to_owned(), json!(stats.entries));
                self.send(&control::reply_ok(&id, fields));
            }
            (Ok(stats), _) => {
                let line = control::event_snapshot_done(job.generation, stats.bytes, stats.entries);
                self.send(&line);
            }
            (Err(message), Some((id, _))) if same_client => {
                self.send(&control::reply_error(&id, FailureKind::Internal, &message));
            }
            (Err(message), _) => {
                let line = control::event_error(
                    FailureKind::Internal,
                    &format!("snapshot gen-{} failed: {message}", job.generation),
                );
                self.send(&line);
            }
        }
    }

    pub(crate) fn cancel_copy_job(&mut self, reason: &str) {
        let Some(job) = self.copy_job.take() else {
            return;
        };
        let _ = kill_process(job.pid, Signal::KILL);
        let _ = self.wait_child(job.pid, KILL_GRACE);
        let _ = mounts::unmount_detached(PUBLISH);
        if let Some((id, serial)) = job.requester
            && serial == self.client_serial
            && self.client.is_some()
        {
            self.send(&control::reply_error(
                &id,
                FailureKind::Internal,
                &format!("snapshot cancelled: {reason}"),
            ));
        }
    }

    pub(crate) fn snapshot_inline(&mut self, generation: u64) -> Result<copier::Stats, String> {
        self.bind_publish(generation)
            .map_err(|(_, message)| message)?;
        let copied = self.copy_profile_to_publish();
        if let Err(error) = mounts::unmount_detached(PUBLISH) {
            log("init", &format!("{error:#}"));
        }
        copied.map_err(|error| format!("{error:#}"))
    }
}

pub(crate) fn parse_copier_report(report: &str) -> Result<copier::Stats, String> {
    let mut parts = report.splitn(3, ' ');
    match parts.next() {
        Some("ok") => {
            let bytes = parts.next().and_then(|value| value.parse().ok());
            let entries = parts.next().and_then(|value| value.trim().parse().ok());
            match (bytes, entries) {
                (Some(bytes), Some(entries)) => Ok(copier::Stats { bytes, entries }),
                _ => Err("copier reported malformed statistics".to_owned()),
            }
        }
        Some("err") => Err(parts.collect::<Vec<_>>().join(" ")),
        _ => Err("copier ended without a report".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copier_reports_parse() {
        assert_eq!(
            parse_copier_report("ok 4096 12").unwrap(),
            copier::Stats {
                bytes: 4096,
                entries: 12
            }
        );
        assert_eq!(
            parse_copier_report("err copy exceeds its byte bound at Cookies").unwrap_err(),
            "copy exceeds its byte bound at Cookies"
        );
        assert!(parse_copier_report("").is_err());
        assert!(parse_copier_report("ok x y").is_err());
    }
}
