//! Steps 1-6 of the launcher contract as the init runs them: signals and
//! cgroups, the host facts and descriptors that must outlive masking, the
//! private mount table, the instance tmpfs, the facts the Machine re-verifies,
//! and the three display and browser processes. Readiness is sent at the end.

use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::fs::{FileTypeExt as _, PermissionsExt as _};
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use rustix::fs::{Mode, OFlags};
use rustix::process::{Pid, Signal, getuid, set_parent_process_death_signal};

use crate::args::Args;
use crate::children::{self, Sealing, Spec};
use crate::control::{self, FailureKind};
use crate::copier::{self, Bounds, Exclusions};
use crate::init::{
    DISPLAY_TIMEOUT, FONT_CACHE, FONTS_CONF, Fatal, HELPER_TIMEOUT, INSTANCE, INSTANCE_HOME,
    INSTANCE_LOG, INSTANCE_RUN, Init, LOG_TAIL_BYTES, MNT, NSSWITCH, PROFILE, PUBLISH, RFB_SOCKET,
    RUNTIME, SIGNALS, SNAPSHOT, SOCKETS, XAUTH_FILE, pid_number, status_code,
};
use crate::mounts::{self, HostFacts};
use crate::supervisor::log;
use crate::{cgroup, raw};

impl Init {
    /// Build the instance and start its processes; `ready` follows.
    pub(crate) fn setup(&mut self) -> Result<(), Fatal> {
        let (facts, snapshot) = self.prepare_host()?;
        self.mount_instance(&facts, snapshot)?;
        self.prepare_instance_directories(&facts)?;
        self.collect_facts()?;
        self.start_children()?;
        let ready = control::report_line(Some("ready"), None, self.report_fields());
        self.send(&ready);
        log("init", "ready");
        Ok(())
    }

    /// Step 1 remainder: the death signal, the signalfd, the sub-cgroups, and
    /// the host facts and descriptors that must be taken before masking.
    fn prepare_host(&mut self) -> Result<(HostFacts, Option<OwnedFd>), Fatal> {
        let internal = |error: anyhow::Error| Fatal::new(FailureKind::Internal, &error);
        let host = |error: anyhow::Error| Fatal::new(FailureKind::Host, &error);
        // The relay is in the parent pid namespace, so `getppid` reads 0 here;
        // the death signal is the only parent-liveness mechanism available.
        set_parent_process_death_signal(Some(Signal::KILL))
            .context("parent death signal")
            .map_err(internal)?;
        self.signals = Some(
            raw::signalfd(SIGNALS, true)
                .context("signalfd")
                .map_err(internal)?,
        );
        self.stdin_null = Some(children::dev_null().map_err(internal)?);
        self.last_capability = children::last_capability();
        self.cgroup_display = cgroup::ensure_child(&self.cgroup_unit, "display").map_err(host)?;
        self.cgroup_browser = cgroup::ensure_child(&self.cgroup_unit, "browser").map_err(host)?;
        let facts = mounts::open_host_facts();
        self.staging = Some(open_directory_path(&self.args.staging_dir).map_err(host)?);
        let snapshot = match &self.args.snapshot_dir {
            Some(dir) => Some(open_directory_path(dir).map_err(host)?),
            None => None,
        };
        self.verify_runtime_files().map_err(host)?;
        Ok((facts, snapshot))
    }

    /// Step 2: the private mount table, in the documented order, then the
    /// masks; records the targets the readiness report lists.
    fn mount_instance(
        &mut self,
        facts: &HostFacts,
        snapshot: Option<OwnedFd>,
    ) -> Result<(), Fatal> {
        let internal = |error: anyhow::Error| Fatal::new(FailureKind::Internal, &error);
        let sandbox = |error: anyhow::Error| Fatal::new(FailureKind::Sandbox, &error);
        let uid = getuid().as_raw();
        mounts::make_private().map_err(sandbox)?;
        mounts::tmpfs(MNT, 1 << 20, 0o755, rustix::mount::MountFlags::NOEXEC).map_err(sandbox)?;
        for alias in [RUNTIME, PROFILE, INSTANCE, SOCKETS, SNAPSHOT, PUBLISH] {
            std::fs::create_dir(alias)
                .with_context(|| format!("creating {alias}"))
                .map_err(sandbox)?;
        }
        let runtime_dir = path_string(&self.args.runtime_dir).map_err(internal)?;
        mounts::bind(&runtime_dir, RUNTIME, true).map_err(sandbox)?;
        mounts::bind(&format!("{RUNTIME}/usr"), "/usr", true).map_err(sandbox)?;
        mounts::bind(&format!("{RUNTIME}/opt"), "/opt", true).map_err(sandbox)?;
        let plain = rustix::mount::MountFlags::empty();
        mounts::tmpfs(PROFILE, self.args.profile_bytes, 0o700, plain).map_err(sandbox)?;
        mounts::tmpfs(INSTANCE, self.args.instance_bytes, 0o700, plain).map_err(sandbox)?;
        let control_dir = path_string(&self.args.control_dir).map_err(internal)?;
        mounts::bind(&control_dir, SOCKETS, false).map_err(sandbox)?;
        if let Some(snapshot) = snapshot {
            mounts::bind_fd(snapshot.as_fd(), SNAPSHOT, true).map_err(sandbox)?;
            let restored = self.restore_snapshot();
            mounts::unmount_detached(SNAPSHOT).map_err(sandbox)?;
            drop(snapshot);
            let stats = restored.map_err(internal)?;
            log(
                "init",
                &format!(
                    "restored snapshot: {} bytes in {} entries",
                    stats.bytes, stats.entries
                ),
            );
        }
        mounts::tmpfs("/tmp", self.args.tmp_bytes, 0o1777, plain).map_err(sandbox)?;
        mounts::tmpfs("/dev/shm", self.args.profile_bytes, 0o1777, plain).map_err(sandbox)?;
        mounts::proc().map_err(sandbox)?;
        let policy_source = format!("{RUNTIME}/{}", self.args.policy_file.display());
        mounts::build_etc(facts, NSSWITCH, &policy_source, &self.args.policy_directory)
            .map_err(sandbox)?;
        let mut masked = Vec::new();
        for target in std::iter::once("/home".to_owned())
            .chain(std::iter::once(format!("/run/user/{uid}")))
            .chain(
                self.args
                    .masks
                    .iter()
                    .map(|mask| mask.display().to_string()),
            )
        {
            if mounts::mask(&target).map_err(sandbox)? {
                masked.push(target);
            }
        }
        self.mount_targets = [
            MNT, RUNTIME, "/usr", "/opt", PROFILE, INSTANCE, SOCKETS, "/tmp", "/dev/shm", "/proc",
            "/etc", "/etc/ssl",
        ]
        .into_iter()
        .map(str::to_owned)
        .chain(
            mounts::HOST_ETC_FILES
                .iter()
                .map(|name| format!("/etc/{name}")),
        )
        .chain(std::iter::once(format!(
            "{}/policy.json",
            self.args.policy_directory.display()
        )))
        .chain(masked)
        .collect();
        Ok(())
    }

    /// The instance tmpfs: `HOME`, XDG directories, logs, the X authority
    /// file, the fontconfig configuration, and the children's environments.
    fn prepare_instance_directories(&mut self, facts: &HostFacts) -> Result<(), Fatal> {
        let internal = |error: anyhow::Error| Fatal::new(FailureKind::Internal, &error);
        for directory in [
            INSTANCE_HOME,
            &format!("{INSTANCE_HOME}/.config"),
            &format!("{INSTANCE_HOME}/.cache"),
            INSTANCE_RUN,
            INSTANCE_LOG,
            FONT_CACHE,
        ] {
            std::fs::create_dir_all(directory)
                .with_context(|| format!("creating {directory}"))
                .map_err(internal)?;
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))
                .with_context(|| format!("securing {directory}"))
                .map_err(internal)?;
        }
        std::fs::write(FONTS_CONF, fonts_conf())
            .context("writing fonts.conf")
            .map_err(internal)?;
        drop(
            rustix::fs::open(
                XAUTH_FILE,
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC,
                Mode::from_raw_mode(0o600),
            )
            .context("creating the X authority file")
            .map_err(internal)?,
        );
        self.display_environment = self.display_environment(&facts.timezone);
        self.browser_environment = self.browser_environment(&facts.timezone);
        Ok(())
    }

    /// The facts the Machine re-verifies from outside: namespace identities,
    /// id maps, the init's label and start time.
    fn collect_facts(&mut self) -> Result<(), Fatal> {
        let internal = |error: anyhow::Error| Fatal::new(FailureKind::Internal, &error);
        for name in ["user", "mnt", "pid", "ipc"] {
            let identity = crate::procinfo::read_namespace("self", name)
                .with_context(|| format!("reading the {name} namespace"))
                .map_err(internal)?;
            self.namespaces.insert(name.to_owned(), identity);
        }
        self.uid_map = crate::procinfo::normalize_map(
            &std::fs::read_to_string("/proc/self/uid_map")
                .context("reading uid_map")
                .map_err(internal)?,
        );
        self.gid_map = crate::procinfo::normalize_map(
            &std::fs::read_to_string("/proc/self/gid_map")
                .context("reading gid_map")
                .map_err(internal)?,
        );
        self.label = crate::procinfo::read_label("self");
        self.start_times.insert(
            "init".to_owned(),
            crate::procinfo::read_start_time(None)
                .context("reading the init start time")
                .map_err(internal)?,
        );
        Ok(())
    }

    /// Steps 4-6: the X cookie, the display server, the read-only socket
    /// export, the window manager and the browser.
    fn start_children(&mut self) -> Result<(), Fatal> {
        let host = |error: anyhow::Error| Fatal::new(FailureKind::Host, &error);
        let sandbox = |error: anyhow::Error| Fatal::new(FailureKind::Sandbox, &error);
        self.write_cookie().map_err(host)?;
        let display = self
            .spawn_display_server()
            .map_err(|error| Self::classify_spawn(&error))?;
        self.display = Some(display);
        self.record_start_time("display_server", display);
        self.wait_for_display().map_err(host)?;
        mounts::remount_readonly(SOCKETS).map_err(sandbox)?;
        let window_manager = self
            .spawn_window_manager()
            .map_err(|error| Self::classify_spawn(&error))?;
        self.window_manager = Some(window_manager);
        self.record_start_time("window_manager", window_manager);
        let browser = self
            .spawn_browser()
            .map_err(|error| Self::classify_spawn(&error))?;
        self.browser = Some(browser);
        self.record_start_time("browser", browser);
        if self.args.sealed_profile.is_some() {
            self.sealed_label = Some(crate::procinfo::read_label(
                &pid_number(browser).to_string(),
            ));
        }
        Ok(())
    }

    fn record_start_time(&mut self, name: &str, pid: Pid) {
        self.start_times.insert(
            name.to_owned(),
            crate::procinfo::read_start_time(Some(pid_number(pid))).unwrap_or_default(),
        );
    }

    pub(crate) fn classify_spawn(error: &anyhow::Error) -> Fatal {
        let message = format!("{error:#}");
        let kind = if message.contains("AppArmor transition") {
            FailureKind::Sandbox
        } else {
            FailureKind::Host
        };
        Fatal { kind, message }
    }

    pub(crate) fn verify_runtime_files(&self) -> Result<()> {
        for relative in [
            &self.args.browser,
            &self.args.display_server,
            &self.args.window_manager,
            &self.args.xrandr,
            &self.args.xkbcomp,
            &self.args.xauth,
            &self.args.policy_file,
        ] {
            let path = self.args.runtime_dir.join(relative);
            if !std::fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.is_file()) {
                bail!("runtime file {} is missing", relative.display());
            }
        }
        for directory in ["usr", "opt", "usr/share/cowboy-browser"] {
            if !self.args.runtime_dir.join(directory).is_dir() {
                bail!("runtime directory {directory} is missing");
            }
        }
        Ok(())
    }

    pub(crate) fn restore_snapshot(&self) -> Result<copier::Stats> {
        let source = copier::open_directory(SNAPSHOT)?;
        let target = copier::open_directory(PROFILE)?;
        copier::copy_tree(
            &source,
            &target,
            Bounds::for_profile(self.args.profile_bytes),
            &Exclusions::chromium(),
            false,
        )
        .context("restoring the snapshot into the profile")
    }

    pub(crate) fn display_environment(&self, timezone: &str) -> Vec<(String, String)> {
        [
            ("PATH", "/usr/bin"),
            ("HOME", INSTANCE_HOME),
            ("TMPDIR", "/tmp"),
            ("XDG_RUNTIME_DIR", INSTANCE_RUN),
            ("DISPLAY", &self.args.display_name()),
            ("XAUTHORITY", XAUTH_FILE),
            ("FONTCONFIG_FILE", FONTS_CONF),
            ("LANG", "en_US.UTF-8"),
            ("TZ", timezone),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
    }

    pub(crate) fn browser_environment(&self, timezone: &str) -> Vec<(String, String)> {
        [
            ("PATH", "/usr/bin"),
            ("FONTCONFIG_FILE", FONTS_CONF),
            ("GSETTINGS_SCHEMA_DIR", "/usr/share/glib-2.0/schemas"),
            ("DISPLAY", &self.args.display_name()),
            ("XAUTHORITY", XAUTH_FILE),
            ("HOME", INSTANCE_HOME),
            ("XDG_CONFIG_HOME", &format!("{INSTANCE_HOME}/.config")),
            ("XDG_CACHE_HOME", &format!("{INSTANCE_HOME}/.cache")),
            ("XDG_RUNTIME_DIR", INSTANCE_RUN),
            ("TMPDIR", "/tmp"),
            ("TZ", timezone),
            ("LANG", "en_US.UTF-8"),
            ("SSL_CERT_FILE", "/etc/ssl/certs/ca-certificates.crt"),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
    }

    pub(crate) fn sealing(&self) -> Sealing<'_> {
        Sealing {
            profile: self.args.sealed_profile.as_deref(),
            last_capability: self.last_capability,
        }
    }

    /// Fork a sealed child whose output goes to its own log file.
    pub(crate) fn spawn(
        &mut self,
        name: &str,
        program: &str,
        arguments: &[String],
        environment: &[(String, String)],
        cgroup: &Path,
    ) -> Result<Pid> {
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
            stdout: children::borrow(&log_fd),
            stderr: children::borrow(&log_fd),
        };
        children::spawn(&spec, self.sealing())
    }

    pub(crate) fn write_cookie(&mut self) -> Result<()> {
        let mut bytes = [0_u8; 16];
        let mut filled = 0;
        while filled < bytes.len() {
            filled += rustix::rand::getrandom(
                &mut bytes[filled..],
                rustix::rand::GetRandomFlags::empty(),
            )
            .context("getrandom")?;
        }
        let cookie = bytes.iter().fold(String::new(), |mut hex, byte| {
            use std::fmt::Write as _;
            let _ = write!(hex, "{byte:02x}");
            hex
        });
        let arguments = [
            "-q".to_owned(),
            "-f".to_owned(),
            XAUTH_FILE.to_owned(),
            "add".to_owned(),
            self.args.display_name(),
            ".".to_owned(),
            cookie,
        ];
        let program = Args::instance_path(&self.args.xauth);
        let environment = self.display_environment.clone();
        let cgroup = self.cgroup_display.clone();
        let pid = self.spawn("xauth", &program, &arguments, &environment, &cgroup)?;
        let status = self.wait_child(pid, HELPER_TIMEOUT)?;
        if status.exit_status() != Some(0) {
            bail!(
                "xauth exited with {:?}: {}",
                status_code(status),
                children::log_tail(&format!("{INSTANCE_LOG}/xauth.log"), LOG_TAIL_BYTES).trim()
            );
        }
        Ok(())
    }

    pub(crate) fn spawn_display_server(&mut self) -> Result<Pid> {
        let arguments: Vec<String> = [
            self.args.display_name(),
            "-auth".to_owned(),
            XAUTH_FILE.to_owned(),
            "-nolisten".to_owned(),
            "tcp".to_owned(),
            "-nolisten".to_owned(),
            "local".to_owned(),
            "-rfbport".to_owned(),
            "-1".to_owned(),
            "-rfbunixpath".to_owned(),
            RFB_SOCKET.to_owned(),
            "-rfbunixmode".to_owned(),
            "0600".to_owned(),
            "-SecurityTypes".to_owned(),
            "None".to_owned(),
            "-AcceptSetDesktopSize=0".to_owned(),
            "-SendPrimary=0".to_owned(),
            "-SendCutText=1".to_owned(),
            "-AcceptCutText=1".to_owned(),
            // Native touch (runtime/native-touch.patch): a direct touch
            // device beside a pointer typed as a mouse for every viewer, and
            // touch contacts from clients that negotiate them.
            "-AcceptTouchEvents=1".to_owned(),
            "-geometry".to_owned(),
            self.args.geometry(),
            "-depth".to_owned(),
            "24".to_owned(),
        ]
        .to_vec();
        let program = Args::instance_path(&self.args.display_server);
        let environment = self.display_environment.clone();
        let cgroup = self.cgroup_display.clone();
        self.spawn(
            "display-server",
            &program,
            &arguments,
            &environment,
            &cgroup,
        )
    }

    pub(crate) fn spawn_window_manager(&mut self) -> Result<Pid> {
        let arguments = [
            "-use_titlebar".to_owned(),
            "no".to_owned(),
            "-use_cursor".to_owned(),
            "yes".to_owned(),
        ];
        let program = Args::instance_path(&self.args.window_manager);
        let environment = self.display_environment.clone();
        let cgroup = self.cgroup_display.clone();
        self.spawn(
            "window-manager",
            &program,
            &arguments,
            &environment,
            &cgroup,
        )
    }

    pub(crate) fn browser_arguments(&self) -> Vec<String> {
        vec![
            format!("--user-data-dir={PROFILE}"),
            "--no-first-run".to_owned(),
            "--no-default-browser-check".to_owned(),
            "--disable-sync".to_owned(),
            "--disable-background-networking".to_owned(),
            "--disable-component-update".to_owned(),
            "--disable-extensions".to_owned(),
            "--disable-breakpad".to_owned(),
            "--disable-crash-reporter".to_owned(),
            "--disable-quic".to_owned(),
            "--disk-cache-size=1048576".to_owned(),
            "--password-store=basic".to_owned(),
            "--lang=en-US".to_owned(),
            "--window-position=0,0".to_owned(),
            format!("--window-size={}", self.args.window_size()),
            format!("--force-device-scale-factor={}", self.args.scale),
            // Chromium's own new-tab page, with the policy's sign-in shortcuts.
            "chrome://newtab".to_owned(),
        ]
    }

    pub(crate) fn spawn_browser(&mut self) -> Result<Pid> {
        let arguments = self.browser_arguments();
        let program = Args::instance_path(&self.args.browser);
        let environment = self.browser_environment.clone();
        let cgroup = self.cgroup_browser.clone();
        self.spawn("browser", &program, &arguments, &environment, &cgroup)
    }

    /// Wait until the display server listens on its RFB and X sockets.
    pub(crate) fn wait_for_display(&mut self) -> Result<()> {
        let x_socket = format!("/tmp/.X11-unix/X{}", self.args.display);
        let deadline = Instant::now() + DISPLAY_TIMEOUT;
        loop {
            if is_socket(RFB_SOCKET) && is_socket(&x_socket) {
                return Ok(());
            }
            let Some(display) = self.display else {
                bail!("display server is not running");
            };
            if let Some(status) = self.take_exit(display) {
                self.display = None;
                bail!(
                    "display server exited with {:?} before listening: {}",
                    status_code(status),
                    children::log_tail(
                        &format!("{INSTANCE_LOG}/display-server.log"),
                        LOG_TAIL_BYTES
                    )
                    .trim()
                );
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                bail!(
                    "display server did not open its sockets in time: {}",
                    children::log_tail(
                        &format!("{INSTANCE_LOG}/display-server.log"),
                        LOG_TAIL_BYTES
                    )
                    .trim()
                );
            }
            self.wait_signals(remaining.min(Duration::from_millis(50)));
        }
    }
}

fn open_directory_path(path: &Path) -> Result<OwnedFd> {
    rustix::fs::open(
        path,
        OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .with_context(|| format!("opening directory {}", path.display()))
}

fn path_string(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_owned)
        .with_context(|| format!("{} is not UTF-8", path.display()))
}

fn is_socket(path: &str) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_socket())
}

fn fonts_conf() -> String {
    format!(
        "<?xml version=\"1.0\"?>\n<!DOCTYPE fontconfig SYSTEM \"fonts.dtd\">\n<fontconfig>\n  <dir>/usr/share/fonts</dir>\n  <cachedir>{FONT_CACHE}</cachedir>\n</fontconfig>\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fonts_conf_points_at_the_artifact_fonts_and_the_instance_cache() {
        let conf = fonts_conf();
        assert!(conf.contains("<dir>/usr/share/fonts</dir>"));
        assert!(conf.contains("<cachedir>/mnt/instance/fontconfig</cachedir>"));
    }
}
