//! Account units: the fixed per-mode command table, the exact unit property
//! set (`docs/antigravity-real-accounts-design.md`, "Account units" and
//! amendments R3-3, R3-7, R3-11, R3-14) and the `systemd-run` command line.
//! No caller string reaches a property or the command line except validated
//! ids and digests.

use std::path::{Path, PathBuf};

use crate::config::{BUILTIN_IP_DENY, Config};
use crate::ids;
use crate::protocol::Mode;
use crate::store::Manifest;

/// Fixed, runtime-entry-relative command of each mode. Adjust here only.
pub const COMMANDS: &[(Mode, &[&str])] = &[
    (Mode::Runtime, &["bin/agy-proxy"]),
    (Mode::Login, &["bin/agy-launch", "--cowboy-login"]),
    (Mode::Logout, &["bin/agy-launch", "--cowboy-logout"]),
    (Mode::Probe, &["bin/agy-launch", "--version"]),
    // Print mode answers a leading supported slash command locally (Cloud
    // Code `retrieveUserQuotaSummary`) without starting a conversation or a
    // model turn; its log goes to the unit's private `/tmp`, so repeated
    // reports never accumulate files in the account home.
    (
        Mode::Usage,
        &[
            "bin/agy-launch",
            "-p",
            "/usage",
            "--output-format",
            "json",
            "--log-file",
            USAGE_LOG,
        ],
    ),
];

/// The `usage` unit's CLI log, inside its private, bounded `/tmp`.
pub const USAGE_LOG: &str = "/tmp/agy-usage.log";

/// Where a `runtime` unit sees its execution bridge socket.
pub const UNIT_EXEC_SOCKET: &str = "/run/cowboy-exec.sock";
/// Name of the credential carrying the per-turn execution descriptor.
pub const DESCRIPTOR_CREDENTIAL: &str = "descriptor";

/// Home-relative names of `agy`'s configuration (amendment R3-14).
pub const GEMINI_DIR: &str = ".gemini";
pub const GEMINI_STATE_DIR: &str = "antigravity-cli";
pub const GEMINI_CONFIG_DIR: &str = "config";
pub const HOOKS_FILE: &str = "hooks.json";
pub const SETTINGS_FILE: &str = "settings.json";
pub const TOKEN_FILE: &str = "antigravity-oauth-token";

#[must_use]
pub fn command(mode: Mode) -> &'static [&'static str] {
    COMMANDS
        .iter()
        .find_map(|(candidate, argv)| (*candidate == mode).then_some(*argv))
        .expect("every mode has a command")
}

/// `cga-<key>-<mode>-<nonce>.service`.
#[must_use]
pub fn unit_name(key: &str, mode: Mode, nonce: &str) -> String {
    format!("{}-{}-{nonce}.service", ids::user_name(key), mode.as_str())
}

/// `cowboy-accounts-<key>.slice`, nested under `cowboy-accounts.slice`.
#[must_use]
pub fn slice_name(key: &str) -> String {
    format!("cowboy-accounts-{key}.slice")
}

/// Absolute host paths of one account (identical inside its units).
#[derive(Debug, Clone)]
pub struct AccountPaths {
    pub image: PathBuf,
    pub home: PathBuf,
    pub gemini_tree: PathBuf,
}

impl AccountPaths {
    #[must_use]
    pub fn new(state_root: &Path, key: &str) -> Self {
        let dir = state_root.join("accounts").join(key);
        Self {
            image: dir.join("home.img"),
            home: dir.join("home"),
            gemini_tree: dir.join("gemini"),
        }
    }
}

/// Everything one unit's properties depend on.
#[derive(Debug, Clone)]
pub struct UnitSpec<'a> {
    pub key: &'a str,
    pub mode: Mode,
    pub nonce: &'a str,
    pub paths: &'a AccountPaths,
    pub runtime_dir: &'a Path,
    pub manifest: &'a Manifest,
    /// Host path of this unit's execution socket (`runtime` only).
    pub exec_socket: Option<&'a Path>,
    /// Host path of the daemon-written descriptor file.
    pub descriptor: Option<&'a Path>,
    /// Whether `/var/run` is a real directory that needs its own mask (when it
    /// is the usual symlink to `/run`, masking `/run` covers it and a second
    /// tmpfs would hide `/run`'s re-exposed paths).
    pub mask_var_run: bool,
}

fn display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// The exact property list of a unit.
#[must_use]
pub fn properties(config: &Config, spec: &UnitSpec<'_>) -> Vec<String> {
    let user = ids::user_name(spec.key);
    let home = display(&spec.paths.home);
    let gemini = format!("{home}/{GEMINI_DIR}");
    let state = format!("{gemini}/{GEMINI_STATE_DIR}");
    let runtime = display(spec.runtime_dir);
    let mut deny: Vec<&str> = BUILTIN_IP_DENY.to_vec();
    deny.extend(config.ip_deny.iter().map(String::as_str));
    let mut properties = vec![
        "Type=exec".to_owned(),
        format!("User={user}"),
        format!("Group={user}"),
        "SupplementaryGroups=".to_owned(),
        "ProtectSystem=strict".to_owned(),
        "ProtectHome=tmpfs".to_owned(),
        "PrivateTmp=yes".to_owned(),
        "PrivateDevices=yes".to_owned(),
        format!("TemporaryFileSystem={}", display(&config.state_root)),
        "TemporaryFileSystem=/run".to_owned(),
    ];
    if spec.mask_var_run {
        properties.push("TemporaryFileSystem=/var/run".to_owned());
    }
    properties.extend([
        format!(
            "TemporaryFileSystem=/tmp:size={},mode=1777",
            config.unit_tmp_size
        ),
        // `PrivateTmp=` alone backs `/var/tmp` with host storage the account
        // could fill without bound; it shares the bounded `/tmp` budget.
        format!(
            "TemporaryFileSystem=/var/tmp:size={},mode=1777",
            config.unit_tmp_size
        ),
        format!("BindPaths={home}"),
        format!("BindReadOnlyPaths={runtime}"),
        format!(
            "BindReadOnlyPaths={}:{gemini}",
            display(&spec.paths.gemini_tree)
        ),
        format!("BindPaths={state}"),
        format!(
            "BindReadOnlyPaths={runtime}/{}:{gemini}/{GEMINI_CONFIG_DIR}/{HOOKS_FILE}",
            spec.manifest.hooks
        ),
        format!(
            "BindReadOnlyPaths={runtime}/{}:{state}/{SETTINGS_FILE}",
            spec.manifest.settings
        ),
        "BindReadOnlyPaths=-/run/systemd/resolve".to_owned(),
    ]);
    if let Some(socket) = spec.exec_socket {
        properties.push(format!("BindPaths={}:{UNIT_EXEC_SOCKET}", display(socket)));
    }
    properties.extend(
        [
            "NoNewPrivileges=yes",
            "RestrictSUIDSGID=yes",
            "LockPersonality=yes",
            "ProtectKernelTunables=yes",
            "ProtectKernelModules=yes",
            "ProtectKernelLogs=yes",
            "ProtectControlGroups=yes",
            "ProtectClock=yes",
            "ProtectHostname=yes",
            "RestrictNamespaces=yes",
            "RestrictRealtime=yes",
            "MemoryDenyWriteExecute=no",
            "SystemCallArchitectures=native",
            "SystemCallFilter=@system-service",
            "SystemCallFilter=~io_uring_setup io_uring_enter io_uring_register",
            // Refuse a filtered call with ENOSYS instead of killing the
            // process: Node's libuv probes io_uring_setup at startup and
            // falls back to epoll on ENOSYS, but systemd's default action
            // (SIGSYS) killed agy-proxy before it answered `initialize`.
            // Per-call `:ENOSYS` suffixes cannot be combined with the
            // @system-service allow-list (the unit fails with status 228).
            "SystemCallErrorNumber=ENOSYS",
            "CapabilityBoundingSet=",
            "AmbientCapabilities=",
            "ProtectProc=invisible",
            "ProcSubset=pid",
            "PrivateIPC=yes",
            "RemoveIPC=yes",
            "UMask=0077",
            "KeyringMode=private",
            "RestrictAddressFamilies=AF_INET AF_UNIX AF_NETLINK",
        ]
        .map(str::to_owned),
    );
    properties.extend([
        format!("IPAddressDeny={}", deny.join(" ")),
        format!("IPAddressAllow={}", config.ip_allow.join(" ")),
        format!("MemoryMax={}", config.unit_memory_max),
        "MemorySwapMax=0".to_owned(),
        format!("TasksMax={}", config.unit_tasks_max),
        format!("Slice={}", slice_name(spec.key)),
        format!("WorkingDirectory={home}"),
        format!("Environment=HOME={home}"),
        format!("Environment=PATH={runtime}/bin:/usr/bin:/bin"),
        "Environment=LANG=C.UTF-8".to_owned(),
        "Environment=TZ=UTC".to_owned(),
        "Environment=TMPDIR=/tmp".to_owned(),
        "Environment=AGY_CLI_DISABLE_AUTO_UPDATE=true".to_owned(),
        "Environment=DBUS_SESSION_BUS_ADDRESS=unix:path=/nonexistent".to_owned(),
    ]);
    if let Some(descriptor) = spec.descriptor {
        properties.push(format!(
            "LoadCredential={DESCRIPTOR_CREDENTIAL}:{}",
            display(descriptor)
        ));
    }
    properties.push(format!("TimeoutStopSec={}s", config.stop_timeout_secs));
    properties
}

/// `systemd-run --pipe --wait --quiet --collect --unit=<name> -p ... --
/// <daemon> exec-confined -- <runtime>/<fixed command>`.
#[must_use]
pub fn systemd_run_argv(config: &Config, spec: &UnitSpec<'_>) -> Vec<String> {
    let mut argv = vec![
        display(&config.tools.systemd_run),
        "--pipe".to_owned(),
        "--wait".to_owned(),
        "--quiet".to_owned(),
        "--collect".to_owned(),
        format!("--unit={}", unit_name(spec.key, spec.mode, spec.nonce)),
    ];
    for property in properties(config, spec) {
        argv.push("-p".to_owned());
        argv.push(property);
    }
    argv.push("--".to_owned());
    argv.push(display(&config.exec_path));
    argv.push("exec-confined".to_owned());
    argv.push("--".to_owned());
    let mut command = command(spec.mode).iter();
    let program = command.next().expect("commands are non-empty");
    argv.push(format!("{}/{program}", display(spec.runtime_dir)));
    argv.extend(command.map(|argument| (*argument).to_owned()));
    argv
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        Manifest {
            agy: "bin/agy".into(),
            hooks: "policy/hooks.json".into(),
            settings: "policy/settings.json".into(),
        }
    }

    #[test]
    fn every_mode_has_a_fixed_command() {
        for mode in [
            Mode::Runtime,
            Mode::Login,
            Mode::Logout,
            Mode::Probe,
            Mode::Usage,
        ] {
            assert!(!command(mode).is_empty());
        }
        assert_eq!(command(Mode::Login), ["bin/agy-launch", "--cowboy-login"]);
        assert_eq!(
            command(Mode::Usage),
            [
                "bin/agy-launch",
                "-p",
                "/usage",
                "--output-format",
                "json",
                "--log-file",
                "/tmp/agy-usage.log"
            ]
        );
    }

    #[test]
    fn runtime_unit_properties_snapshot() {
        let config = Config {
            ip_deny: vec!["40.160.141.233/32".into()],
            ..Config::default()
        };
        let key = "0123456789abcdef";
        let paths = AccountPaths::new(&config.state_root, key);
        let runtime = config.state_root.join("runtime").join("ab".repeat(32));
        let socket = PathBuf::from("/run/cowboy-account-sandbox/units/00112233aabbccdd/exec.sock");
        let descriptor =
            PathBuf::from("/run/cowboy-account-sandbox/units/00112233aabbccdd/descriptor");
        let manifest = manifest();
        let spec = UnitSpec {
            key,
            mode: Mode::Runtime,
            nonce: "00112233aabbccdd",
            paths: &paths,
            runtime_dir: &runtime,
            manifest: &manifest,
            exec_socket: Some(&socket),
            descriptor: Some(&descriptor),
            mask_var_run: false,
        };
        let home = "/var/lib/cowboy-account-sandbox/accounts/0123456789abcdef/home";
        let rt = format!(
            "/var/lib/cowboy-account-sandbox/runtime/{}",
            "ab".repeat(32)
        );
        let expected: Vec<String> = vec![
            "Type=exec".into(),
            "User=cga-0123456789abcdef".into(),
            "Group=cga-0123456789abcdef".into(),
            "SupplementaryGroups=".into(),
            "ProtectSystem=strict".into(),
            "ProtectHome=tmpfs".into(),
            "PrivateTmp=yes".into(),
            "PrivateDevices=yes".into(),
            "TemporaryFileSystem=/var/lib/cowboy-account-sandbox".into(),
            "TemporaryFileSystem=/run".into(),
            "TemporaryFileSystem=/tmp:size=256M,mode=1777".into(),
            "TemporaryFileSystem=/var/tmp:size=256M,mode=1777".into(),
            format!("BindPaths={home}"),
            format!("BindReadOnlyPaths={rt}"),
            format!("BindReadOnlyPaths=/var/lib/cowboy-account-sandbox/accounts/0123456789abcdef/gemini:{home}/.gemini"),
            format!("BindPaths={home}/.gemini/antigravity-cli"),
            format!("BindReadOnlyPaths={rt}/policy/hooks.json:{home}/.gemini/config/hooks.json"),
            format!("BindReadOnlyPaths={rt}/policy/settings.json:{home}/.gemini/antigravity-cli/settings.json"),
            "BindReadOnlyPaths=-/run/systemd/resolve".into(),
            "BindPaths=/run/cowboy-account-sandbox/units/00112233aabbccdd/exec.sock:/run/cowboy-exec.sock".into(),
            "NoNewPrivileges=yes".into(),
            "RestrictSUIDSGID=yes".into(),
            "LockPersonality=yes".into(),
            "ProtectKernelTunables=yes".into(),
            "ProtectKernelModules=yes".into(),
            "ProtectKernelLogs=yes".into(),
            "ProtectControlGroups=yes".into(),
            "ProtectClock=yes".into(),
            "ProtectHostname=yes".into(),
            "RestrictNamespaces=yes".into(),
            "RestrictRealtime=yes".into(),
            "MemoryDenyWriteExecute=no".into(),
            "SystemCallArchitectures=native".into(),
            "SystemCallFilter=@system-service".into(),
            "SystemCallFilter=~io_uring_setup io_uring_enter io_uring_register".into(),
            "SystemCallErrorNumber=ENOSYS".into(),
            "CapabilityBoundingSet=".into(),
            "AmbientCapabilities=".into(),
            "ProtectProc=invisible".into(),
            "ProcSubset=pid".into(),
            "PrivateIPC=yes".into(),
            "RemoveIPC=yes".into(),
            "UMask=0077".into(),
            "KeyringMode=private".into(),
            "RestrictAddressFamilies=AF_INET AF_UNIX AF_NETLINK".into(),
            "IPAddressDeny=127.0.0.0/8 ::1/128 169.254.0.0/16 fe80::/10 224.0.0.0/4 ff00::/8 10.0.0.0/8 172.16.0.0/12 192.168.0.0/16 100.64.0.0/10 40.160.141.233/32".into(),
            "IPAddressAllow=127.0.0.53/32".into(),
            "MemoryMax=3G".into(),
            "MemorySwapMax=0".into(),
            "TasksMax=512".into(),
            "Slice=cowboy-accounts-0123456789abcdef.slice".into(),
            format!("WorkingDirectory={home}"),
            format!("Environment=HOME={home}"),
            format!("Environment=PATH={rt}/bin:/usr/bin:/bin"),
            "Environment=LANG=C.UTF-8".into(),
            "Environment=TZ=UTC".into(),
            "Environment=TMPDIR=/tmp".into(),
            "Environment=AGY_CLI_DISABLE_AUTO_UPDATE=true".into(),
            "Environment=DBUS_SESSION_BUS_ADDRESS=unix:path=/nonexistent".into(),
            "LoadCredential=descriptor:/run/cowboy-account-sandbox/units/00112233aabbccdd/descriptor".into(),
            "TimeoutStopSec=30s".into(),
        ];
        assert_eq!(properties(&config, &spec), expected);

        let argv = systemd_run_argv(&config, &spec);
        assert_eq!(
            argv[..6],
            [
                "/usr/bin/systemd-run",
                "--pipe",
                "--wait",
                "--quiet",
                "--collect",
                "--unit=cga-0123456789abcdef-runtime-00112233aabbccdd.service"
            ]
        );
        let tail = &argv[argv.len() - 5..];
        assert_eq!(
            tail,
            [
                "--",
                "/usr/local/libexec/cowboy-account-sandbox",
                "exec-confined",
                "--",
                &format!("{rt}/bin/agy-proxy")
            ]
        );
    }

    #[test]
    fn login_units_have_no_socket_and_mask_var_run_when_asked() {
        let config = Config::default();
        let key = "0123456789abcdef";
        let paths = AccountPaths::new(&config.state_root, key);
        let runtime = config.state_root.join("runtime").join("cd".repeat(32));
        let manifest = manifest();
        let spec = UnitSpec {
            key,
            mode: Mode::Login,
            nonce: "00112233aabbccdd",
            paths: &paths,
            runtime_dir: &runtime,
            manifest: &manifest,
            exec_socket: None,
            descriptor: None,
            mask_var_run: true,
        };
        let properties = properties(&config, &spec);
        assert!(properties.contains(&"TemporaryFileSystem=/var/run".to_owned()));
        assert!(
            !properties
                .iter()
                .any(|p| p.contains("cowboy-exec.sock") || p.starts_with("LoadCredential"))
        );
        let argv = systemd_run_argv(&config, &spec);
        assert_eq!(
            argv[argv.len() - 2..],
            [
                format!("{}/bin/agy-launch", runtime.display()),
                "--cowboy-login".to_owned()
            ]
        );
    }
}
