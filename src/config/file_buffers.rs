//! Device file-buffer settings projected for remote session runtimes.
//!
//! The Machine owns the Device configuration; a session runtime only reads
//! this small JSON projection, whose path reaches it through the worker's
//! execution descriptor. The file is rewritten atomically whenever a live
//! change alters it, so a running session follows edits within seconds.
use super::{Config, Handle, schema};
use std::path::{Path, PathBuf};

/// Worker environment naming the projection file.
pub const POLICY_ENV: &str = "COWBOY_FILE_BUFFERS_POLICY";
const POLL: std::time::Duration = std::time::Duration::from_secs(5);

#[must_use]
pub fn path(state_dir: &Path) -> PathBuf {
    state_dir.join("file-buffers.json")
}

#[must_use]
pub fn projection(config: &Config) -> serde_json::Value {
    serde_json::json!({
        "schema": 1,
        "enabled": config.get(&schema::DEVICE_FILE_BUFFERS_ENABLED),
        "max_file_size": config.get(&schema::DEVICE_FILE_BUFFERS_MAX_FILE_SIZE),
        "excluded": config.get(&schema::DEVICE_FILE_BUFFERS_EXCLUDED),
        "cache_max_bytes": config.get(&schema::DEVICE_FILE_BUFFERS_CACHE_MAX_BYTES),
        "cache_max_age_seconds": config.get(&schema::DEVICE_FILE_BUFFERS_CACHE_MAX_AGE).as_secs(),
        "target_temp_max_age_seconds":
            config.get(&schema::DEVICE_FILE_BUFFERS_TARGET_TEMP_MAX_AGE).as_secs(),
    })
}

/// Replace the projection atomically; readers never see a partial file.
///
/// # Errors
/// When the file cannot be written or renamed.
pub fn publish(path: &Path, value: &serde_json::Value) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
    // A fresh Machine publishes before anything else has created its state
    // directory.
    if let Some(parent) = path.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
    }
    let temporary = path.with_extension(format!("json.{}.tmp", std::process::id()));
    let written = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(value.to_string().as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    written
}

/// Publish now, then republish whenever the effective projection changes.
///
/// # Errors
/// When the first projection cannot be written.
pub fn start(handle: Handle, path: PathBuf) -> std::io::Result<tokio::task::JoinHandle<()>> {
    let mut last = projection(&handle.current());
    publish(&path, &last)?;
    Ok(tokio::spawn(async move {
        let mut tick = tokio::time::interval(POLL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let next = projection(&handle.current());
            if next == last {
                continue;
            }
            match publish(&path, &next) {
                Ok(()) => last = next,
                Err(error) => tracing::warn!(%error, "file buffer policy was not republished"),
            }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_carries_every_setting_and_publishes_atomically() {
        let report = super::super::check_text(
            super::super::Scope::Device,
            "schema = 1\n[file_buffers]\nenabled = false\nmax_file_size = \"64KiB\"\nexcluded = [\"*.bin\"]\ncache_max_age = \"2h\"\n",
            None,
        );
        let value = projection(report.config.as_ref().unwrap());
        assert_eq!(
            value,
            serde_json::json!({"schema": 1, "enabled": false, "max_file_size": 65536,
                "excluded": ["*.bin"], "cache_max_bytes": 64 << 20,
                "cache_max_age_seconds": 7200, "target_temp_max_age_seconds": 3600})
        );
        let directory = tempfile::tempdir().unwrap();
        let file = path(directory.path());
        publish(&file, &value).unwrap();
        let read: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(read, value);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn a_fresh_state_directory_is_created_privately() {
        use std::os::unix::fs::PermissionsExt as _;
        let directory = tempfile::tempdir().unwrap();
        let state = directory.path().join("fresh-machine-state");
        publish(&path(&state), &serde_json::json!({"schema": 1})).unwrap();
        assert!(path(&state).is_file());
        assert_eq!(
            std::fs::metadata(&state).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
}
