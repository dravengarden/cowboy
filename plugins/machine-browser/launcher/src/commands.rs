//! Step 8 of the launcher contract: the command handlers (`status`,
//! `attach`/`detach`, `navigate`, `resize`, `snapshot`, `stop`) and the
//! readiness report they share.

use std::fs::File;
use std::io::Read as _;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use rustix::process::{Pid, WaitStatus};
use serde_json::{Map, Value, json};

use crate::args::Args;
use crate::children::{self, Spec};
use crate::control::{self, Command, FailureKind, Parsed};
use crate::init::{
    HELPER_TIMEOUT, INSTANCE, INSTANCE_LOG, Init, NAVIGATE_TIMEOUT, PROFILE, pid_number,
    status_code,
};
use crate::mounts;
use crate::supervisor::log;

impl Init {
    pub(crate) fn handle_line(&mut self, line: &[u8]) -> Option<u8> {
        let request = match control::parse_request(line) {
            Parsed::Request(request) => request,
            Parsed::Invalid { id, message } => {
                if let Some(id) = id {
                    self.send(&control::reply_error(&id, FailureKind::Internal, &message));
                } else {
                    log(
                        "init",
                        &format!("ignoring malformed control line: {message}"),
                    );
                }
                return None;
            }
        };
        let id = request.id;
        match request.command {
            Command::Status => {
                let line = control::report_line(None, Some(&id), self.report_fields());
                self.send(&line);
            }
            Command::Attach { attachment_id } => {
                self.attachments.insert(attachment_id);
                self.send(&control::reply_ok(&id, Map::new()));
            }
            Command::Detach { attachment_id } => {
                self.attachments.remove(&attachment_id);
                self.send(&control::reply_ok(&id, Map::new()));
            }
            Command::Resize { width, height } => match self.resize(width, height) {
                Ok(()) => self.send(&control::reply_ok(&id, Map::new())),
                Err(error) => self.send(&control::reply_error(
                    &id,
                    FailureKind::Internal,
                    &format!("{error:#}"),
                )),
            },
            Command::Navigate { url } => match self.navigate(&url) {
                Ok(()) => self.send(&control::reply_ok(&id, Map::new())),
                Err(error) => self.send(&control::reply_error(
                    &id,
                    FailureKind::Internal,
                    &format!("{error:#}"),
                )),
            },
            Command::Snapshot { generation } => {
                if let Err((kind, message)) = self.start_copy_job(generation, Some(id.clone())) {
                    self.send(&control::reply_error(&id, kind, &message));
                }
            }
            Command::Stop {
                graceful_ms,
                snapshot_generation,
            } => {
                let fields = self.stop(Duration::from_millis(graceful_ms), snapshot_generation);
                self.send(&control::reply_ok(&id, fields));
                self.shutdown_display();
                return Some(0);
            }
        }
        None
    }

    pub(crate) fn report_fields(&self) -> Map<String, Value> {
        let pid_of = |pid: Option<Pid>| pid.map_or(0, pid_number);
        let mut fields = Map::new();
        fields.insert(
            "pids".to_owned(),
            json!({
                "supervisor": self.supervisor_pid,
                "init": 1,
                "display_server": pid_of(self.display),
                "window_manager": pid_of(self.window_manager),
                "browser": pid_of(self.browser),
            }),
        );
        fields.insert("start_times".to_owned(), json!(self.start_times));
        fields.insert(
            "namespaces".to_owned(),
            json!({
                "user": self.namespaces.get("user").cloned().unwrap_or_default(),
                "mnt": self.namespaces.get("mnt").cloned().unwrap_or_default(),
                "pid": self.namespaces.get("pid").cloned().unwrap_or_default(),
                "ipc": self.namespaces.get("ipc").cloned().unwrap_or_default(),
            }),
        );
        fields.insert("uid_map".to_owned(), json!(self.uid_map));
        fields.insert("gid_map".to_owned(), json!(self.gid_map));
        fields.insert(
            "labels".to_owned(),
            json!({ "init": self.label, "sealed": self.sealed_label }),
        );
        fields.insert(
            "mounts".to_owned(),
            Value::Array(mounts::report(&self.mount_targets).unwrap_or_default()),
        );
        fields.insert(
            "snapshot_in_progress".to_owned(),
            Value::Bool(self.copy_job.is_some()),
        );
        fields
    }

    /// Run a helper to completion and return its status and captured stdout.
    pub(crate) fn run_helper(
        &mut self,
        name: &str,
        program: &str,
        arguments: &[String],
        environment: &[(String, String)],
        cgroup: &Path,
        timeout: Duration,
    ) -> Result<(WaitStatus, String)> {
        let (read_end, write_end) =
            rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC).context("helper pipe")?;
        let log_fd = children::log_file(&format!("{INSTANCE_LOG}/{name}.log"))?;
        let stdin = self.stdin_null.as_ref().context("stdin")?;
        let spec = Spec {
            name,
            program,
            arguments,
            environment,
            working_directory: INSTANCE,
            cgroup: Some(cgroup),
            stdin: children::borrow(stdin),
            stdout: children::borrow(&write_end),
            stderr: children::borrow(&log_fd),
        };
        let pid = children::spawn(&spec, self.sealing())?;
        drop(write_end);
        let status = self.wait_child(pid, timeout)?;
        let mut output = String::new();
        let mut pipe: File = read_end.into();
        let _ = pipe.read_to_string(&mut output);
        Ok((status, output))
    }

    pub(crate) fn xrandr(
        &mut self,
        arguments: &[&str],
        timeout: Duration,
    ) -> Result<(WaitStatus, String)> {
        let program = Args::instance_path(&self.args.xrandr);
        let arguments: Vec<String> = arguments
            .iter()
            .map(|argument| (*argument).to_owned())
            .collect();
        let environment = self.display_environment.clone();
        let cgroup = self.cgroup_display.clone();
        self.run_helper(
            "xrandr",
            &program,
            &arguments,
            &environment,
            &cgroup,
            timeout,
        )
    }

    /// Resize the framebuffer through `xrandr`: add a mode of the requested
    /// size to the display's output when it has none, then select it.
    pub(crate) fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        if !crate::args::geometry_allowed(width, height) {
            bail!("geometry {width}x{height} is out of bounds");
        }
        let (status, query) = self.xrandr(&["--query"], HELPER_TIMEOUT)?;
        if status.exit_status() != Some(0) {
            bail!("xrandr --query exited with {:?}", status_code(status));
        }
        let output = query
            .lines()
            .find(|line| line.contains(" connected"))
            .and_then(|line| line.split_whitespace().next())
            .map(str::to_owned)
            .context("xrandr reports no connected output")?;
        let mode = format!("{width}x{height}");
        let has_mode = query
            .lines()
            .skip(1)
            .any(|line| line.split_whitespace().next() == Some(mode.as_str()));
        if !has_mode {
            let (status, _) = self.xrandr(
                &[
                    "--newmode",
                    &mode,
                    "0",
                    &width.to_string(),
                    "0",
                    "0",
                    "0",
                    &height.to_string(),
                    "0",
                    "0",
                    "0",
                ],
                HELPER_TIMEOUT,
            )?;
            if status.exit_status() != Some(0) {
                bail!("xrandr --newmode exited with {:?}", status_code(status));
            }
            let (status, _) = self.xrandr(&["--addmode", &output, &mode], HELPER_TIMEOUT)?;
            if status.exit_status() != Some(0) {
                bail!("xrandr --addmode exited with {:?}", status_code(status));
            }
        }
        let (status, _) = self.xrandr(&["--output", &output, "--mode", &mode], HELPER_TIMEOUT)?;
        if status.exit_status() != Some(0) {
            bail!("xrandr --output exited with {:?}", status_code(status));
        }
        Ok(())
    }

    /// Re-invoke the browser against the running profile; its singleton
    /// hands the URL to the existing instance and the helper exits.
    pub(crate) fn navigate(&mut self, url: &str) -> Result<()> {
        control::validate_url(url).map_err(anyhow::Error::msg)?;
        if self.browser.is_none() {
            bail!("the browser is not running");
        }
        let program = Args::instance_path(&self.args.browser);
        let arguments = [format!("--user-data-dir={PROFILE}"), url.to_owned()];
        let environment = self.browser_environment.clone();
        let cgroup = self.cgroup_browser.clone();
        let (status, _) = self.run_helper(
            "navigate",
            &program,
            &arguments,
            &environment,
            &cgroup,
            NAVIGATE_TIMEOUT,
        )?;
        if status.exit_status() != Some(0) {
            bail!("navigation helper exited with {:?}", status_code(status));
        }
        Ok(())
    }
}
