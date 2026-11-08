//! Durable records of the Machine browser: the per-account control record
//! that survives reset and revocation, and the per-instance record the Machine
//! reconciles against after a restart. Every write replaces the file
//! atomically (temporary file, fsync, rename, parent fsync).

use std::fs;
use std::io::Write as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};

use super::launcher::ReadyReport;
use crate::machine_protocol::machine_browser::{Geometry, Purpose};

/// Control record beside, not inside, the deleted `browser/` tree.
pub const CONTROL_FILE: &str = "browser-state.json";
pub const BROWSER_DIR: &str = "browser";
pub const SNAPSHOTS_DIR: &str = "snapshots";
pub const STAGING_DIR: &str = "staging";
/// Instance records below the Machine state directory.
pub const INSTANCES_DIR: &str = "browsers";
const SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Reset,
    Revoke,
}

/// An admission fence: raised before a reset or revocation touches anything,
/// lowered only by its completion, resumed after a crash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingOperation {
    pub kind: OperationKind,
    pub operation_id: String,
    pub next_profile_revision: u64,
    pub started_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompletedOperation {
    pub kind: OperationKind,
    pub operation_id: String,
    pub profile_revision: u64,
    pub completed_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlRecord {
    pub schema_version: u16,
    /// Advances on every reset or revocation; a snapshot publishes only if the
    /// revision it captured is still current.
    pub profile_revision: u64,
    /// Full browser version of the runtime that last opened the profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browser_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_generation: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_generation_at_ms: Option<i64>,
    /// Next snapshot generation number; never reused.
    pub next_generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<PendingOperation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_operation: Option<CompletedOperation>,
}

impl Default for ControlRecord {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            profile_revision: 1,
            browser_version: None,
            generation_digest: None,
            current_generation: None,
            current_generation_at_ms: None,
            next_generation: 1,
            pending: None,
            last_operation: None,
        }
    }
}

impl ControlRecord {
    /// Allocate the next snapshot generation number.
    pub fn allocate_generation(&mut self) -> u64 {
        let generation = self.next_generation;
        self.next_generation += 1;
        generation
    }
}

/// Read the account's control record; an account without one starts fresh.
///
/// # Errors
/// Returns when an existing record is unreadable or has an unknown schema.
pub fn read_control(account_dir: &Path) -> Result<ControlRecord> {
    let path = account_dir.join(CONTROL_FILE);
    match fs::read(&path) {
        Ok(bytes) => {
            let record: ControlRecord = serde_json::from_slice(&bytes)
                .with_context(|| format!("decoding {}", path.display()))?;
            ensure!(
                record.schema_version == SCHEMA_VERSION,
                "unsupported browser control record schema"
            );
            Ok(record)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(ControlRecord::default()),
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

/// Replace the account's control record atomically.
///
/// # Errors
/// Returns when the record cannot be written durably.
pub fn write_control(account_dir: &Path, record: &ControlRecord) -> Result<()> {
    replace_file(
        account_dir,
        CONTROL_FILE,
        &serde_json::to_vec_pretty(record)?,
    )
}

/// Temporary file, fsync, rename over `name`, fsync the directory.
pub(super) fn replace_file(directory: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    let temporary = directory.join(format!("{name}.tmp"));
    let target = directory.join(name);
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temporary)
        .with_context(|| format!("creating {}", temporary.display()))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temporary, &target).with_context(|| format!("publishing {}", target.display()))?;
    fs::File::open(directory)?.sync_all()?;
    Ok(())
}

#[must_use]
pub fn browser_root(account_dir: &Path) -> PathBuf {
    account_dir.join(BROWSER_DIR)
}

#[must_use]
pub fn snapshots_root(account_dir: &Path) -> PathBuf {
    browser_root(account_dir).join(SNAPSHOTS_DIR)
}

#[must_use]
pub fn staging_root(account_dir: &Path) -> PathBuf {
    browser_root(account_dir).join(STAGING_DIR)
}

#[must_use]
pub fn generation_name(generation: u64) -> String {
    format!("gen-{generation}")
}

/// Generation number of a `gen-<n>` directory name.
#[must_use]
pub fn parse_generation_name(name: &str) -> Option<u64> {
    let digits = name.strip_prefix("gen-")?;
    (!digits.is_empty() && digits.len() <= 18 && digits.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| digits.parse().ok())
        .flatten()
}

/// Everything a restart needs to recognise, re-verify or stop a unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstanceRecord {
    pub schema_version: u16,
    pub instance_id: String,
    pub account_id: String,
    pub purpose: Purpose,
    pub plugin_id: String,
    pub plugin_version: String,
    pub generation_digest: String,
    pub unit: String,
    pub invocation_id: String,
    pub boot_id: String,
    pub incarnation: u64,
    pub deadline_ms: i64,
    pub profile_revision: u64,
    pub started_at_ms: i64,
    pub runtime_dir: PathBuf,
    pub cgroup_dir: PathBuf,
    pub geometry: Geometry,
    pub scale: u8,
    pub ready: ReadyReport,
}

#[must_use]
pub fn instances_root(state_dir: &Path) -> PathBuf {
    state_dir.join(INSTANCES_DIR)
}

/// Read every instance record. Unreadable records are removed: a record the
/// Machine cannot interpret cannot authorise adoption, and the unit it named
/// is stopped as unrecorded.
///
/// # Errors
/// Returns when the records directory cannot be read.
pub fn read_instances(state_dir: &Path) -> Result<Vec<InstanceRecord>> {
    let root = instances_root(state_dir);
    let mut records = Vec::new();
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(records),
        Err(error) => return Err(error).with_context(|| format!("reading {}", root.display())),
    };
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        match fs::read(&path)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| Ok(serde_json::from_slice::<InstanceRecord>(&bytes)?))
        {
            Ok(record) if record.schema_version == SCHEMA_VERSION => records.push(record),
            Ok(_) | Err(_) => {
                tracing::warn!(path = %path.display(), "removing unreadable browser instance record");
                let _ = fs::remove_file(&path);
            }
        }
    }
    records.sort_by(|left, right| left.instance_id.cmp(&right.instance_id));
    Ok(records)
}

/// # Errors
/// Returns when the record cannot be written durably.
pub fn write_instance(state_dir: &Path, record: &InstanceRecord) -> Result<()> {
    let root = instances_root(state_dir);
    fs::create_dir_all(&root)?;
    crate::provider_account::prepare_private_directory(&root)?;
    replace_file(
        &root,
        &format!("{}.json", record.instance_id),
        &serde_json::to_vec_pretty(record)?,
    )
}

/// # Errors
/// Returns when an existing record cannot be removed.
pub fn remove_instance(state_dir: &Path, instance_id: &str) -> Result<()> {
    let path = instances_root(state_dir).join(format!("{instance_id}.json"));
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("removing {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_record_round_trips_and_survives_missing_file() {
        let temp = tempfile::tempdir().unwrap();
        let mut record = read_control(temp.path()).unwrap();
        assert_eq!(record, ControlRecord::default());
        assert_eq!(record.allocate_generation(), 1);
        assert_eq!(record.allocate_generation(), 2);
        record.pending = Some(PendingOperation {
            kind: OperationKind::Reset,
            operation_id: "op-1".into(),
            next_profile_revision: 2,
            started_at_ms: 5,
        });
        write_control(temp.path(), &record).unwrap();
        assert!(!temp.path().join("browser-state.json.tmp").exists());
        assert_eq!(read_control(temp.path()).unwrap(), record);
        std::fs::write(temp.path().join(CONTROL_FILE), b"{\"schema_version\":9}").unwrap();
        assert!(read_control(temp.path()).is_err());
    }

    #[test]
    fn generation_names_are_strict() {
        assert_eq!(parse_generation_name("gen-12"), Some(12));
        assert_eq!(parse_generation_name("gen-"), None);
        assert_eq!(parse_generation_name("gen-1a"), None);
        assert_eq!(parse_generation_name("snapshot"), None);
        assert_eq!(generation_name(7), "gen-7");
    }
}
