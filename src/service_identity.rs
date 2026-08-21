//! Stable identity and local resource namespace for one Cowboy Service.

use std::path::{Path, PathBuf};

#[cfg(feature = "full")]
use anyhow::Context as _;
use anyhow::Result;
#[cfg(feature = "full")]
use std::io::Read as _;

#[cfg(feature = "full")]
const SERVICE_ID_FILE: &str = "service-id";
const SERVICE_ID_RANDOM_BYTES: usize = 16;

pub(crate) fn valid_service_id(value: &str) -> bool {
    value.len() == 4 + SERVICE_ID_RANDOM_BYTES * 2
        && value.starts_with("svc-")
        && value[4..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(feature = "full")]
pub(crate) fn load_or_create(data_dir: &Path) -> Result<String> {
    std::fs::create_dir_all(data_dir).context("creating Cowboy data directory")?;
    let path = data_dir.join(SERVICE_ID_FILE);
    if path.exists() {
        return read(&path);
    }

    let service_id = generate()?;
    let temporary = data_dir.join(format!(".{SERVICE_ID_FILE}.{}.tmp", std::process::id()));
    std::fs::write(&temporary, format!("{service_id}\n"))
        .context("writing temporary Cowboy Service identity")?;
    set_mode(&temporary, 0o600)?;
    match std::fs::hard_link(&temporary, &path) {
        Ok(()) => {
            std::fs::remove_file(&temporary).context("removing temporary Service identity")?;
            Ok(service_id)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            std::fs::remove_file(&temporary).context("removing raced Service identity")?;
            read(&path)
        }
        Err(error) => {
            let _ = std::fs::remove_file(&temporary);
            Err(error).context("publishing Cowboy Service identity")
        }
    }
}

pub(crate) fn service_state_dir(home: &Path, service_id: &str) -> Result<PathBuf> {
    anyhow::ensure!(valid_service_id(service_id), "invalid Cowboy Service id");
    Ok(home
        .join(".local/state/cowboy-machine/services")
        .join(service_id))
}

#[cfg(feature = "full")]
fn read(path: &Path) -> Result<String> {
    let value = std::fs::read_to_string(path)
        .with_context(|| format!("reading Cowboy Service identity {}", path.display()))?;
    let value = value.trim().to_owned();
    anyhow::ensure!(valid_service_id(&value), "invalid Cowboy Service identity");
    Ok(value)
}

#[cfg(feature = "full")]
fn generate() -> Result<String> {
    let mut random = [0_u8; SERVICE_ID_RANDOM_BYTES];
    std::fs::File::open("/dev/urandom")
        .context("opening OS randomness")?
        .read_exact(&mut random)
        .context("reading OS randomness")?;
    let suffix = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    Ok(format!("svc-{suffix}"))
}

#[cfg(all(feature = "full", unix))]
fn set_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

#[cfg(all(feature = "full", not(unix)))]
fn set_mode(_path: &Path, _mode: u32) -> std::io::Result<()> {
    Ok(())
}

#[cfg(all(test, feature = "full"))]
mod tests {
    use super::*;

    #[test]
    fn generated_service_identity_is_stable_and_path_safe() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-service-identity-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = std::fs::remove_dir_all(&root);
        let first = load_or_create(&root).unwrap();
        let second = load_or_create(&root).unwrap();
        assert_eq!(first, second);
        assert!(valid_service_id(&first));
        assert_eq!(
            service_state_dir(Path::new("/home/me"), &first).unwrap(),
            Path::new("/home/me/.local/state/cowboy-machine/services").join(first)
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
