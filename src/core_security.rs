//! Core local-authentication policy and the stopped-Controller ownership handoff.
//! This is not a Plugin capability, catalog selection, or online migration API.

use std::io::{Read as _, Write as _};
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};

const CONFIG_SCHEMA: &str = "dravengarden.cowboy.core-security/v1";
const MARKER: &str = ".core-security-v1";
const MAX_BYTES: u64 = 8192;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(try_from = "String")]
pub(crate) struct NamespaceId(String);

impl TryFrom<String> for NamespaceId {
    type Error = anyhow::Error;

    fn try_from(value: String) -> Result<Self> {
        crate::plugin_host::validate_plugin_id(&value)
            .map_err(|_| anyhow::anyhow!("invalid CoreSecurity namespace identity"))?;
        crate::plugin_host::postgres_schema_name(&value).map_err(|_| {
            anyhow::anyhow!("CoreSecurity namespace exceeds its portable identity budget")
        })?;
        Ok(Self(value))
    }
}

impl NamespaceId {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Source {
    Fresh,
    AdoptLegacy,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PasskeyConfig {
    pub namespace_id: NamespaceId,
    pub source: Source,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    schema: String,
    pub passkeys: PasskeyConfig,
}

fn read_private(path: &Path) -> Result<Option<Vec<u8>>> {
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("opening CoreSecurity record"),
    };
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.mode() & 0o077 == 0,
        "CoreSecurity record must be a private regular file"
    );
    ensure!(
        metadata.len() <= MAX_BYTES,
        "CoreSecurity record is too large"
    );
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_BYTES,
        "CoreSecurity record is too large"
    );
    Ok(Some(bytes))
}

impl Config {
    pub(crate) fn load(path: Option<&Path>) -> Result<Option<Self>> {
        let Some(path) = path else { return Ok(None) };
        ensure!(
            path.is_absolute(),
            "CoreSecurity configuration path must be absolute"
        );
        let bytes = read_private(path)?.context("CoreSecurity configuration is missing")?;
        let config: Self =
            crate::auth_plugins::decode_private_json(&bytes, "CoreSecurity configuration")?;
        ensure!(
            config.schema == CONFIG_SCHEMA,
            "unsupported CoreSecurity configuration"
        );
        Ok(Some(config))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Phase {
    Prepared,
    Ready,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "backend", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum StorageLocation {
    Postgres {
        data_root: PathBuf,
        schema: String,
    },
    Sqlite {
        database: PathBuf,
        namespace: PathBuf,
    },
}

impl StorageLocation {
    fn validate(&self, id: &NamespaceId) -> Result<()> {
        let paths = match self {
            Self::Postgres { data_root, schema } => {
                ensure!(
                    *schema == crate::plugin_host::postgres_schema_name(id.as_str())?,
                    "CoreSecurity schema identity mismatch"
                );
                vec![data_root]
            }
            Self::Sqlite {
                database,
                namespace,
            } => {
                ensure!(
                    database != namespace,
                    "core database and Passkey namespace must be distinct"
                );
                vec![database, namespace]
            }
        };
        ensure!(
            paths
                .iter()
                .all(|path| path.is_absolute()
                    && path.to_str().is_some_and(|text| text.len() <= 2048)),
            "invalid CoreSecurity storage location"
        );
        Ok(())
    }
}

/// Both durable authorities repeat this identity. The locator binds the
/// physical namespace and data root; it is never an HTTP/public projection.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Authority {
    pub schema: u16,
    pub binding_id: String,
    pub passkeys: PasskeyConfig,
    pub location: StorageLocation,
}

impl Authority {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.schema == 1
                && self.binding_id.len() == 32
                && self
                    .binding_id
                    .bytes()
                    .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f')),
            "invalid CoreSecurity authority"
        );
        self.location.validate(&self.passkeys.namespace_id)
    }

    pub(crate) fn inspect(dir: &crate::plugin_dir::PluginDir) -> Result<Option<Self>> {
        read_private(&dir.root().join(MARKER))?
            .map(|bytes| {
                let record: Self =
                    crate::auth_plugins::decode_private_json(&bytes, "CoreSecurity authority")?;
                record.validate()?;
                Ok(record)
            })
            .transpose()
    }

    /// Called only while the lifetime Controller lock is held, after the DB
    /// prepare has committed. Publish complete bytes atomically; never overwrite
    /// a different authority or treat a corrupt marker as absent.
    pub(crate) fn record(&self, dir: &crate::plugin_dir::PluginDir) -> Result<()> {
        self.validate()?;
        if let Some(existing) = Self::inspect(dir)? {
            ensure!(
                existing == *self,
                "CoreSecurity filesystem authority conflicts with the database"
            );
            return Ok(());
        }
        let pending = dir.root().join(format!(
            ".core-security-{}.partial",
            uuid::Uuid::new_v4().simple()
        ));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&pending)?;
        let bytes = serde_json::to_vec(self)?;
        ensure!(
            bytes.len() as u64 <= MAX_BYTES,
            "CoreSecurity authority exceeds its budget"
        );
        file.write_all(&bytes)?;
        file.sync_all()?;
        // A hard link is create-only; unlike rename it cannot replace authority.
        let result = std::fs::hard_link(&pending, dir.root().join(MARKER));
        std::fs::remove_file(&pending)?;
        if let Err(error) = result {
            ensure!(
                error.kind() == std::io::ErrorKind::AlreadyExists,
                "publishing CoreSecurity authority failed"
            );
            ensure!(
                Self::inspect(dir)?.as_ref() == Some(self),
                "CoreSecurity authority publication conflict"
            );
        }
        std::fs::File::open(dir.root())?.sync_all()?;
        Ok(())
    }
}

/// Cooperating readers hold this for the complete Controller lifetime, even in
/// legacy mode. It fences local cutover against an already-running new reader;
/// an older binary needs the separately accepted stopped-service reader floor.
pub(crate) struct ControllerLock(std::fs::File);

impl ControllerLock {
    pub(crate) fn acquire(dir: &crate::plugin_dir::PluginDir) -> Result<Self> {
        ensure!(
            dir.root().symlink_metadata()?.is_dir(),
            "Controller Plugin root must be a real directory"
        );
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(dir.root().join(".controller-security.lock"))?;
        ensure!(
            file.metadata()?.is_file(),
            "invalid Controller security lock"
        );
        fs2::FileExt::try_lock_exclusive(&file)
            .context("Controller data root is already in use")?;
        Ok(Self(file))
    }
}

impl Drop for ControllerLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.0);
    }
}

/// Validate paths without creating state or following namespace symlinks.
pub(crate) fn namespace_path(
    dir: &crate::plugin_dir::PluginDir,
    id: &NamespaceId,
) -> Result<PathBuf> {
    for path in [
        dir.root().to_owned(),
        dir.live_root(),
        dir.plugin_live_dir(id.as_str())?,
        dir.plugin_live_dir(id.as_str())?.join("state"),
    ] {
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) => ensure!(
                metadata.is_dir(),
                "CoreSecurity namespace contains a non-directory or symlink"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    let path = dir.plugin_live_dir(id.as_str())?.join("state/db.sqlite");
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) => ensure!(
            metadata.is_file() && metadata.nlink() == 1,
            "CoreSecurity namespace must be an unlinked regular database file"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(path)
}

#[cfg(test)]
mod tests;
