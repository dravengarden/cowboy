//! Atomic durable replacement for small, single-owner configuration files.
use std::{fs, io::Write as _, os::unix::fs::OpenOptionsExt as _, path::Path};

pub(crate) fn write(path: &Path, value: &impl serde::Serialize) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("missing config directory"))?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".cowboy-{:032x}.tmp", rand::random::<u128>()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(&serde_json::to_vec_pretty(value)?)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    let _ = fs::remove_file(temporary);
    result
}
