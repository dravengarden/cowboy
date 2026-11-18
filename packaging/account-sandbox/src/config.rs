//! The daemon's host configuration: one root-owned JSON file (default
//! `/etc/cowboy-account-sandbox/config.json`) written by the host
//! configuration. Every field is optional and falls back to the defaults
//! below; unknown fields are rejected. The schema is documented in
//! `packaging/account-sandbox/README.md`.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, ensure};
use serde::Deserialize;

pub const DEFAULT_CONFIG_PATH: &str = "/etc/cowboy-account-sandbox/config.json";
const GIB: u64 = 1024 * 1024 * 1024;
const MAX_CONFIG_BYTES: u64 = 64 * 1024;

/// Address ranges always denied to account units; `ip_deny` only appends to
/// them, so configuring a host address can never drop a private range.
/// Explicit prefixes: `systemd-run -p` rejects the symbolic names
/// (`localhost`, `link-local`, `multicast`) that unit files accept.
pub const BUILTIN_IP_DENY: &[&str] = &[
    "127.0.0.0/8",
    "::1/128",
    "169.254.0.0/16",
    "fe80::/10",
    "224.0.0.0/4",
    "ff00::/8",
    "10.0.0.0/8",
    "172.16.0.0/12",
    "192.168.0.0/16",
    "100.64.0.0/10",
];

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Root of account trees and the runtime store (root 0711).
    pub state_root: PathBuf,
    /// Root of per-unit runtime files (descriptors, execution sockets).
    pub run_dir: PathBuf,
    /// Root-held allow-list of vendor `agy` SHA-512 digests.
    pub agy_allow_list: PathBuf,
    /// The installed daemon binary, used as the units' `exec-confined` wrapper.
    pub exec_path: PathBuf,
    /// Size of each account's sparse ext4 home image.
    pub home_image_bytes: u64,
    /// Fixed inode count of each home image.
    pub home_image_inodes: u64,
    /// Host free space that must remain on the state root's filesystem.
    pub free_space_reserve_bytes: u64,
    /// Extra `IPAddressDeny=` entries appended to [`BUILTIN_IP_DENY`]
    /// (the host's own public addresses).
    pub ip_deny: Vec<String>,
    /// `IPAddressAllow=` entries (the resolver stub).
    pub ip_allow: Vec<String>,
    /// Runtime-class units (`runtime`, `probe`) per account. One by default:
    /// an account runs one runtime, so at most one turn of it can run
    /// (design "Revision 5 simplification"); a second start is `limit`.
    pub max_runtime_units: usize,
    /// `MemoryMax=` of each unit.
    pub unit_memory_max: String,
    /// `TasksMax=` of each unit.
    pub unit_tasks_max: u32,
    /// `size=` of each unit's `/tmp`.
    pub unit_tmp_size: String,
    /// Largest accepted runtime archive.
    pub max_runtime_archive_bytes: u64,
    /// Most entries in one runtime archive.
    pub max_runtime_entries: usize,
    /// Most runtimes kept in the store (live ones are always kept).
    pub max_runtimes: usize,
    /// Seconds to wait for a stopped unit to disappear.
    pub stop_timeout_secs: u64,
    pub tools: Tools,
}

/// Absolute paths of the host tools the daemon runs.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Tools {
    pub systemd_run: PathBuf,
    pub systemctl: PathBuf,
    pub useradd: PathBuf,
    pub userdel: PathBuf,
    pub mkfs_ext4: PathBuf,
    pub mount: PathBuf,
    pub umount: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            state_root: "/var/lib/cowboy-account-sandbox".into(),
            run_dir: "/run/cowboy-account-sandbox".into(),
            agy_allow_list: "/etc/cowboy-account-sandbox/agy-sha512.allow".into(),
            exec_path: "/usr/local/libexec/cowboy-account-sandbox".into(),
            home_image_bytes: 2 * GIB,
            home_image_inodes: 131_072,
            free_space_reserve_bytes: 8 * GIB,
            ip_deny: Vec::new(),
            ip_allow: vec!["127.0.0.53/32".to_owned()],
            max_runtime_units: 1,
            unit_memory_max: "3G".to_owned(),
            unit_tasks_max: 512,
            unit_tmp_size: "256M".to_owned(),
            max_runtime_archive_bytes: GIB,
            max_runtime_entries: 50_000,
            max_runtimes: 6,
            stop_timeout_secs: 30,
            tools: Tools::default(),
        }
    }
}

impl Default for Tools {
    fn default() -> Self {
        Self {
            systemd_run: "/usr/bin/systemd-run".into(),
            systemctl: "/usr/bin/systemctl".into(),
            useradd: "/usr/sbin/useradd".into(),
            userdel: "/usr/sbin/userdel".into(),
            mkfs_ext4: "/usr/sbin/mkfs.ext4".into(),
            mount: "/usr/bin/mount".into(),
            umount: "/usr/bin/umount".into(),
        }
    }
}

impl Config {
    /// Load `path`; a missing file at the default path means all defaults.
    pub fn load(path: &Path, required: bool) -> Result<Self> {
        let bytes = match std::fs::metadata(path) {
            Ok(metadata) => {
                ensure!(
                    metadata.len() <= MAX_CONFIG_BYTES,
                    "{} is too large",
                    path.display()
                );
                std::fs::read(path).with_context(|| format!("read {}", path.display()))?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && !required => {
                return Self::default().validated();
            }
            Err(error) => return Err(error).with_context(|| format!("stat {}", path.display())),
        };
        let config: Self =
            serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?;
        config.validated()
    }

    pub fn validated(self) -> Result<Self> {
        for (name, path) in [
            ("state_root", &self.state_root),
            ("run_dir", &self.run_dir),
            ("agy_allow_list", &self.agy_allow_list),
            ("exec_path", &self.exec_path),
        ] {
            ensure!(path.is_absolute(), "{name} must be absolute");
            ensure!(
                unit_safe(&path.to_string_lossy()),
                "{name} contains characters unsafe in unit properties"
            );
        }
        ensure!(
            self.home_image_bytes >= 64 * 1024 * 1024,
            "home_image_bytes is below 64 MiB"
        );
        ensure!(
            self.home_image_inodes >= 1024,
            "home_image_inodes is below 1024"
        );
        // Revision 5: one runtime per account is a safety invariant, not a
        // tunable; a larger value would admit concurrent turns of one account.
        ensure!(self.max_runtime_units == 1, "max_runtime_units must be 1");
        ensure!(self.max_runtimes >= 1, "max_runtimes must be positive");
        for range in self.ip_deny.iter().chain(&self.ip_allow) {
            ensure!(valid_ip_range(range), "invalid address range {range:?}");
        }
        ensure!(valid_size(&self.unit_memory_max), "invalid unit_memory_max");
        ensure!(valid_size(&self.unit_tmp_size), "invalid unit_tmp_size");
        Ok(self)
    }
}

/// Characters a path may contain when it is placed into a unit property.
pub fn unit_safe(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-' | b'+'))
}

/// An IPv4/IPv6 address with optional prefix, or a systemd range keyword.
fn valid_ip_range(range: &str) -> bool {
    if matches!(range, "any" | "localhost" | "link-local" | "multicast") {
        return true;
    }
    let (address, prefix) = match range.split_once('/') {
        Some((address, prefix)) => (address, Some(prefix)),
        None => (range, None),
    };
    let max = match address.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(_)) => 32,
        Ok(std::net::IpAddr::V6(_)) => 128,
        Err(_) => return false,
    };
    prefix.is_none_or(|prefix| prefix.parse::<u8>().is_ok_and(|bits| bits <= max))
}

/// A systemd size such as `3G`, `512M` or a byte count.
fn valid_size(value: &str) -> bool {
    let digits = value.trim_end_matches(['K', 'M', 'G', 'T']);
    !digits.is_empty()
        && value.len() - digits.len() <= 1
        && digits.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_validate_and_unknown_fields_fail() {
        Config::default().validated().unwrap();
        let config: Config = serde_json::from_str(
            r#"{"ip_deny":["40.160.141.233/32"],"home_image_bytes":2147483648}"#,
        )
        .unwrap();
        assert_eq!(config.ip_deny, ["40.160.141.233/32"]);
        assert_eq!(config.max_runtime_units, 1);
        assert!(serde_json::from_str::<Config>(r#"{"bogus":1}"#).is_err());
        assert!(serde_json::from_str::<Config>(r#"{"tools":{"bogus":"/x"}}"#).is_err());
    }

    #[test]
    fn ranges_and_sizes_are_checked() {
        assert!(valid_ip_range("10.0.0.0/8"));
        assert!(valid_ip_range("40.160.141.233"));
        assert!(valid_ip_range("fe80::/10"));
        assert!(!valid_ip_range("10.0.0.0/33"));
        assert!(!valid_ip_range("10.0.0.0/8 any"));
        assert!(valid_size("3G") && valid_size("1024") && !valid_size("3GG") && !valid_size("x"));
        let bad = Config {
            ip_deny: vec!["nope".into()],
            ..Config::default()
        };
        assert!(bad.validated().is_err());
    }

    #[test]
    fn runtime_limit_is_fixed_at_one() {
        for limit in [0, 2] {
            let config = Config {
                max_runtime_units: limit,
                ..Config::default()
            };
            assert!(config.validated().is_err(), "{limit} accepted");
        }
    }

    #[test]
    fn private_ranges_are_always_denied() {
        for range in [
            "10.0.0.0/8",
            "172.16.0.0/12",
            "192.168.0.0/16",
            "100.64.0.0/10",
            "169.254.0.0/16",
        ] {
            assert!(BUILTIN_IP_DENY.contains(&range), "{range} missing");
        }
    }
}
