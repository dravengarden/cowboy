//! The frozen CoreSecurity Passkey storage contract a legacy Plugin host must
//! match. Plugin activation, the catalog and the runtime check it before any
//! store exists, so it names only the host specification it validates.

use anyhow::{Context as _, Result, ensure};
use sha2::{Digest as _, Sha256};

pub const STORAGE_CAPABILITY: &str = "webauthn";

// The exact historical 0001 SQL remains in its immutable signed package.
// Accepting a different schema requires a separate core migration/reader gate;
// a Plugin publication or pin alone cannot upgrade credential storage.
pub(crate) const LEGACY_POSTGRES_CHECKSUM: &str =
    "sha256:7e3b4ad9b0cbffa54fd4067ddd6e32a90906ec6f75f704722a2f28aca33c3424";
pub(crate) const LEGACY_SQLITE_CHECKSUM: &str =
    "sha256:d8b43ab7949eb0919d8429acca66c60bcd0be4cdb5b4ac198c41677fd1deb385";

pub fn validate_legacy_host(host: &crate::plugin_host::PluginHostSpec) -> Result<()> {
    if !host
        .native_capabilities
        .iter()
        .any(|name| name == STORAGE_CAPABILITY)
    {
        return Ok(());
    }
    let storage = host
        .storage
        .as_ref()
        .context("CoreSecurity requires Passkey storage")?;
    for (migrations, checksum) in [
        (&storage.postgres, LEGACY_POSTGRES_CHECKSUM),
        (&storage.sqlite, LEGACY_SQLITE_CHECKSUM),
    ] {
        ensure!(
            migrations.migrations.len() == 1
                && migrations.migrations[0].version == "0001"
                && format!(
                    "sha256:{:x}",
                    Sha256::digest(migrations.migrations[0].sql.as_bytes())
                ) == checksum,
            "CoreSecurity Passkey schema is frozen; a Plugin selection cannot migrate credentials"
        );
    }
    Ok(())
}
