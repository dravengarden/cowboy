//! Private bounded local evidence. Rotation never traverses another directory
//! or deletes project/native-session data. One process owns one directory.

use std::fs::{self, File, OpenOptions};
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::os::unix::fs::{
    DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _,
};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, ensure};
use sha2::{Digest as _, Sha256};

pub(crate) const DEFAULT_SEGMENT_BYTES: u64 = 8 * 1024 * 1024;
pub(crate) const DEFAULT_RETAINED_FILES: usize = 8;
const DAY_MS: i64 = 86_400_000;
const MIN_SEGMENT_BYTES: u64 = 64 * 1024;

pub(crate) fn default_directory(data_dir: &Path) -> PathBuf {
    let identity = format!(
        "{}:{}",
        rustix::process::geteuid().as_raw(),
        data_dir.display()
    );
    let digest = format!("{:x}", Sha256::digest(identity.as_bytes()));
    Path::new("/tmp").join(format!("cowboy-telemetry-{}", &digest[..20]))
}

pub(crate) struct TelemetryFile {
    directory: PathBuf,
    lock: WriterLock,
    file: File,
    bytes: u64,
    day: i64,
    segment_bytes: u64,
    retained_files: usize,
    retention_ms: i64,
}

// flock belongs to an open-file description, including descriptors briefly
// inherited by a concurrent fork before CLOEXEC runs. Closing this process's
// descriptor alone can therefore leave the lock held after owner teardown.
// Explicitly unlock on every acquired-lock exit, including failed startup.
struct WriterLock(File);

impl Drop for WriterLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

fn private_metadata(path: &Path, directory: bool) -> Result<Option<fs::Metadata>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("reading telemetry file metadata"),
    };
    ensure!(
        !metadata.file_type().is_symlink()
            && (if directory {
                metadata.is_dir()
            } else {
                metadata.is_file()
            })
            && metadata.uid() == rustix::process::geteuid().as_raw()
            && metadata.permissions().mode() & 0o077 == 0
            && (directory || metadata.nlink() == 1),
        "telemetry path must be an owned private directory or unlinked regular file"
    );
    Ok(Some(metadata))
}

fn open_private(path: &Path) -> Result<File> {
    private_metadata(path, false)?;
    let file = OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
        .context("opening private telemetry file")?;
    let actual = file.metadata()?;
    let expected = private_metadata(path, false)?.context("telemetry file disappeared")?;
    ensure!(
        actual.ino() == expected.ino() && actual.dev() == expected.dev(),
        "telemetry file changed while opening"
    );
    Ok(file)
}

impl TelemetryFile {
    pub(crate) fn open(
        directory: PathBuf,
        segment_bytes: u64,
        retained_files: usize,
        now_ms: i64,
    ) -> Result<Self> {
        ensure!(
            directory.is_absolute() && directory.file_name().is_some(),
            "telemetry directory must be an absolute private child directory"
        );
        ensure!(
            (MIN_SEGMENT_BYTES..=64 * 1024 * 1024).contains(&segment_bytes),
            "telemetry segment size must be between 64 KiB and 64 MiB"
        );
        ensure!(
            (2..=32).contains(&retained_files),
            "telemetry retention must be between 2 and 32 files"
        );
        if private_metadata(&directory, true)?.is_none() {
            // Do not recursively create or chmod somebody else's ancestors.
            fs::DirBuilder::new()
                .mode(0o700)
                .create(&directory)
                .context("creating private telemetry directory")?;
        }
        private_metadata(&directory, true)?.context("telemetry directory disappeared")?;
        let lock = open_private(&directory.join(".writer.lock"))?;
        fs2::FileExt::try_lock_exclusive(&lock)
            .context("telemetry directory already has a writer")?;
        let lock = WriterLock(lock);
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(index) = name
                .strip_prefix("telemetry.jsonl.")
                .and_then(|suffix| suffix.parse::<usize>().ok())
            {
                ensure!(
                    index < retained_files,
                    "archive excess telemetry segments before reducing retention"
                );
            }
        }
        for index in 0..retained_files {
            if let Some(metadata) = private_metadata(&segment_path(&directory, index), false)? {
                ensure!(
                    metadata.len() <= segment_bytes,
                    "archive oversized telemetry segments before reducing segment size"
                );
            }
        }
        let mut file = open_private(&segment_path(&directory, 0))?;
        let metadata = file.metadata()?;
        ensure!(
            metadata.len() <= segment_bytes,
            "existing telemetry segment exceeds configured limit"
        );
        let day = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|age| i64::try_from(age.as_millis()).ok())
            .map_or(now_ms / DAY_MS, |timestamp| timestamp / DAY_MS);
        // A killed write may end in a partial JSON record. Never append another
        // record to it. Recovery reads at most this bounded segment.
        let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
        file.seek(SeekFrom::Start(0))?;
        (&mut file)
            .take(segment_bytes + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= segment_bytes,
            "telemetry segment changed during recovery"
        );
        let complete = bytes
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |index| index + 1);
        file.set_len(u64::try_from(complete)?)?;
        Ok(Self {
            directory,
            lock,
            file,
            bytes: u64::try_from(complete)?,
            day,
            segment_bytes,
            retained_files,
            retention_ms: 7 * DAY_MS,
        })
    }

    pub(crate) fn with_retention_seconds(mut self, seconds: u64) -> Result<Self> {
        ensure!(
            (60..=90 * 86400).contains(&seconds),
            "telemetry retention must be between one minute and 90 days"
        );
        self.retention_ms = i64::try_from(seconds)? * 1000;
        Ok(self)
    }

    fn maintain(&mut self, now_ms: i64) -> Result<()> {
        private_metadata(&self.directory, true)?.context("telemetry directory disappeared")?;
        for index in 0..self.retained_files {
            let path = segment_path(&self.directory, index);
            let Some(metadata) = private_metadata(&path, false)? else {
                continue;
            };
            let modified = metadata
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis();
            if index == 0 {
                let actual = self.file.metadata()?;
                ensure!(
                    actual.ino() == metadata.ino() && actual.dev() == metadata.dev(),
                    "telemetry file replaced"
                );
            }
            let cutoff = now_ms.saturating_sub(self.retention_ms).max(0);
            let bytes = crate::logs::storage::private_read(&path, self.segment_bytes)?;
            let mut kept = Vec::with_capacity(bytes.len());
            for line in bytes.split_inclusive(|b| *b == b'\n') {
                // Existing files predate ingestion timestamps; mtime is the
                // conservative fallback. New writes carry an owned local stamp.
                let observed = serde_json::from_slice::<serde_json::Value>(line)
                    .ok()
                    .and_then(|v| v["_cowboy_observed_ms"].as_i64())
                    .unwrap_or(modified as i64);
                if observed >= cutoff {
                    kept.extend_from_slice(line);
                }
            }
            if kept.len() != bytes.len() {
                if kept.is_empty() && index > 0 {
                    fs::remove_file(&path)?;
                } else {
                    let temporary = self
                        .directory
                        .join(format!(".expiry-{}.tmp", uuid::Uuid::new_v4().simple()));
                    let result = (|| -> Result<()> {
                        let mut next = OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .mode(0o600)
                            .custom_flags(libc::O_NOFOLLOW)
                            .open(&temporary)?;
                        next.write_all(&kept)?;
                        next.sync_all()?;
                        fs::rename(&temporary, &path)?;
                        File::open(&self.directory)?.sync_all()?;
                        Ok(())
                    })();
                    if result.is_err() {
                        let _ = fs::remove_file(&temporary);
                    }
                    result?;
                    if index == 0 {
                        self.file = open_private(&path)?;
                        self.bytes = kept.len() as u64;
                        self.day = now_ms / DAY_MS;
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn write(&mut self, records: &str, now_ms: i64) -> Result<()> {
        // An external /tmp cleaner can unlink an open file or writer lock.
        // Do not silently claim success while writing invisible evidence.
        private_metadata(&self.directory, true)?.context("telemetry directory disappeared")?;
        for (path, file) in [
            (self.directory.join(".writer.lock"), &self.lock.0),
            (segment_path(&self.directory, 0), &self.file),
        ] {
            let expected =
                private_metadata(&path, false)?.context("telemetry file or lock disappeared")?;
            let actual = file.metadata()?;
            ensure!(
                expected.dev() == actual.dev() && expected.ino() == actual.ino(),
                "telemetry file or writer lock was replaced"
            );
        }
        ensure!(
            self.file.metadata()?.len() == self.bytes,
            "telemetry segment changed or a failed record could not be repaired"
        );
        ensure!(
            records.is_empty() || records.ends_with('\n'),
            "telemetry records must end with a newline"
        );
        ensure!(
            records
                .split_inclusive('\n')
                .all(|line| line.len() as u64 <= self.segment_bytes),
            "telemetry record exceeds segment capacity"
        );
        for original in records.split_inclusive('\n') {
            let mut value: serde_json::Value = serde_json::from_str(original)?;
            let object = value
                .as_object_mut()
                .context("telemetry record must be an object")?;
            object.insert("_cowboy_observed_ms".into(), now_ms.into());
            let line = format!("{value}\n");
            ensure!(
                line.len() as u64 <= self.segment_bytes,
                "telemetry record and retention stamp exceed segment capacity"
            );
            let day = now_ms / DAY_MS;
            if self.bytes > 0
                && (self.bytes.saturating_add(line.len() as u64) > self.segment_bytes
                    || day > self.day)
            {
                self.rotate()?;
            }
            if self.bytes == 0 {
                self.day = day;
            }
            if let Err(error) = self.file.write_all(line.as_bytes()) {
                // Best effort remove only the just-failed partial record.
                let _ = self.file.set_len(self.bytes);
                return Err(error).context("writing local telemetry record");
            }
            self.bytes += line.len() as u64;
        }
        self.file.sync_data().context("syncing local telemetry")
    }

    fn rotate(&mut self) -> Result<()> {
        // Validate every exact destination before any material deletion.
        for index in 0..self.retained_files {
            private_metadata(&segment_path(&self.directory, index), false)?;
        }
        self.file.flush()?;
        let oldest = segment_path(&self.directory, self.retained_files - 1);
        match fs::remove_file(&oldest) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("retiring oldest telemetry segment"),
        }
        for index in (0..self.retained_files - 1).rev() {
            let source = segment_path(&self.directory, index);
            if private_metadata(&source, false)?.is_some() {
                fs::rename(source, segment_path(&self.directory, index + 1))?;
            }
        }
        self.file = open_private(&segment_path(&self.directory, 0))?;
        self.bytes = 0;
        Ok(())
    }
}

impl crate::logs::EvidenceSink for TelemetryFile {
    fn write(&mut self, records: &str, now: i64) -> Result<()> {
        TelemetryFile::write(self, records, now)
    }
    fn maintain(&mut self, now: i64) -> Result<()> {
        TelemetryFile::maintain(self, now)
    }
}

fn segment_path(directory: &Path, index: usize) -> PathBuf {
    directory.join(if index == 0 {
        "telemetry.jsonl".to_owned()
    } else {
        format!("telemetry.jsonl.{index}")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "cowboy-telemetry-test-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        path
    }

    #[test]
    fn idle_expiry_removes_old_rows_from_a_mixed_current_segment() {
        let path = directory();
        let mut writer = TelemetryFile::open(path.clone(), MIN_SEGMENT_BYTES, 3, 1000)
            .unwrap()
            .with_retention_seconds(60)
            .unwrap();
        writer.write("{\"old\":true}\n", 1000).unwrap();
        writer.write("{\"new\":true}\n", 50000).unwrap();
        writer.maintain(62000).unwrap();
        let contents = fs::read_to_string(segment_path(&path, 0)).unwrap();
        assert!(!contents.contains("old"));
        assert!(contents.contains("new"));
        writer.write("{\"after\":true}\n", 63000).unwrap();
        writer.maintain(124000).unwrap();
        assert_eq!(writer.bytes, 0);
        drop(writer);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn rotation_bounds_size_count_and_preserves_json_records() {
        let path = directory();
        let mut writer = TelemetryFile::open(path.clone(), MIN_SEGMENT_BYTES, 3, 1).unwrap();
        for index in 0..40 {
            writer
                .write(
                    &format!("{{\"index\":{index},\"text\":\"{}\"}}\n", "中".repeat(3000)),
                    1,
                )
                .unwrap();
        }
        let mut files = 0;
        for entry in fs::read_dir(&path).unwrap() {
            let entry = entry.unwrap();
            if entry.file_name() == ".writer.lock" {
                continue;
            }
            files += 1;
            assert!(entry.metadata().unwrap().len() <= MIN_SEGMENT_BYTES);
            for line in fs::read_to_string(entry.path()).unwrap().lines() {
                serde_json::from_str::<serde_json::Value>(line).unwrap();
            }
        }
        assert_eq!(files, 3);
        drop(writer);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn restart_repairs_partial_record_and_rejects_competing_writer() {
        let path = directory();
        let mut writer = TelemetryFile::open(path.clone(), MIN_SEGMENT_BYTES, 3, 1).unwrap();
        writer.write("{\"ok\":true}\n", 1).unwrap();
        assert!(TelemetryFile::open(path.clone(), MIN_SEGMENT_BYTES, 3, 1).is_err());
        writer.file.write_all(b"{\"unfinished\":").unwrap();
        assert!(writer.write("{\"next\":true}\n", 1).is_err());
        drop(writer);
        let mut writer = TelemetryFile::open(path.clone(), MIN_SEGMENT_BYTES, 3, 1).unwrap();
        writer.write("{\"next\":true}\n", 1).unwrap();
        let contents = fs::read_to_string(segment_path(&path, 0)).unwrap();
        let rows: Vec<serde_json::Value> = contents
            .lines()
            .map(|s| serde_json::from_str(s).unwrap())
            .collect();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["ok"], true);
        assert_eq!(rows[1]["next"], true);
        assert_eq!(rows[0]["_cowboy_observed_ms"], 1);
        drop(writer);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn owner_teardown_unlocks_even_while_an_inherited_descriptor_remains_open() {
        let path = directory();
        let writer = TelemetryFile::open(path.clone(), MIN_SEGMENT_BYTES, 3, 1).unwrap();
        let inherited = writer.lock.0.try_clone().unwrap();
        assert!(TelemetryFile::open(path.clone(), MIN_SEGMENT_BYTES, 3, 1).is_err());
        drop(writer);
        let next = TelemetryFile::open(path.clone(), MIN_SEGMENT_BYTES, 3, 1).unwrap();
        drop(inherited);
        // Closing an old duplicate must not release the new owner's lock.
        assert!(TelemetryFile::open(path.clone(), MIN_SEGMENT_BYTES, 3, 1).is_err());
        drop(next);
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn rotates_at_utc_day_and_rejects_unsafe_paths() {
        let path = directory();
        let mut writer = TelemetryFile::open(path.clone(), MIN_SEGMENT_BYTES, 3, 1).unwrap();
        writer.write("{}\n", 1).unwrap();
        writer.write("{}\n", DAY_MS + 1).unwrap();
        assert!(segment_path(&path, 1).is_file());
        drop(writer);
        std::os::unix::fs::symlink(path.join("untouched"), segment_path(&path, 2)).unwrap();
        assert!(TelemetryFile::open(path.clone(), MIN_SEGMENT_BYTES, 3, 1).is_err());
        assert!(!path.join("untouched").exists());
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn external_cleanup_and_reduced_retention_fail_visibly() {
        let path = directory();
        let mut writer = TelemetryFile::open(path.clone(), MIN_SEGMENT_BYTES, 3, 1).unwrap();
        writer.write("{}\n", 1).unwrap();
        writer.write("{}\n", DAY_MS + 1).unwrap();
        writer.write("{}\n", 2 * DAY_MS + 1).unwrap();
        drop(writer);
        assert!(TelemetryFile::open(path.clone(), MIN_SEGMENT_BYTES, 2, 1).is_err());
        let mut writer = TelemetryFile::open(path.clone(), MIN_SEGMENT_BYTES, 3, 1).unwrap();
        fs::remove_file(segment_path(&path, 0)).unwrap();
        assert!(writer.write("{}\n", 1).is_err());
        drop(writer);
        fs::remove_dir_all(path).unwrap();
    }
}
