//! Durable core namespace identity. Prepared/ready is a recoverable handoff,
//! not a cross-database atomic transaction or permission to migrate online.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context as _, Result, ensure};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

use super::{StorageBackend, Store};
use crate::core_passkeys::PasskeyStorage;
use crate::core_security::{
    Authority, Config, NamespaceId, Phase, Source, StorageLocation, namespace_path,
};
use crate::plugin_dir::PluginDir;
use crate::plugin_storage::NamespaceBackend;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HandoffPoint {
    Prepared,
    Marked,
    NamespacePrepared,
    NamespaceCommitted,
}

macro_rules! query_core {
    ($store:expr, $query:expr, execute) => {
        match &$store.backend {
            StorageBackend::Postgres(db) => $query.execute(&db.pool).await?.rows_affected(),
            StorageBackend::Sqlite(db) => $query.execute(&db.pool).await?.rows_affected(),
        }
    };
    ($store:expr, $query:expr, $method:ident) => {
        match &$store.backend {
            StorageBackend::Postgres(db) => $query.$method(&db.pool).await?,
            StorageBackend::Sqlite(db) => $query.$method(&db.pool).await?,
        }
    };
}

impl Store {
    pub(crate) async fn security_authority(&self) -> Result<Option<(Authority, Phase)>> {
        let row = query_core!(
            self,
            sqlx::query_as::<_, (String, String)>(
                "SELECT authority, phase FROM core_security_authority WHERE singleton = 1"
            ),
            fetch_optional
        );
        row.map(|(json, phase)| {
            ensure!(
                json.len() <= 8192,
                "CoreSecurity authority exceeds its budget"
            );
            let authority: Authority = crate::auth_plugins::decode_private_json(
                json.as_bytes(),
                "CoreSecurity database authority",
            )?;
            authority.validate()?;
            let phase = match phase.as_str() {
                "prepared" => Phase::Prepared,
                "ready" => Phase::Ready,
                _ => anyhow::bail!("unknown CoreSecurity authority phase"),
            };
            Ok((authority, phase))
        })
        .transpose()
    }

    pub(crate) async fn require_legacy_security(&self) -> Result<()> {
        ensure!(
            self.security_authority().await?.is_none(),
            "CoreSecurity authority requires core configuration; legacy storage fallback is forbidden"
        );
        Ok(())
    }

    /// Server calls this immediately after `SQLx` migration, before restoring
    /// sessions, spawning writers, or staging any Plugin host.
    pub(crate) async fn initialize_core_security(
        &self,
        config: Option<&Config>,
        dir: &PluginDir,
    ) -> Result<()> {
        if let Some(config) = config {
            self.passkey_storage.attach_core(self, config, dir).await
        } else {
            ensure!(
                Authority::inspect(dir)?.is_none(),
                "CoreSecurity authority requires core configuration"
            );
            self.require_legacy_security().await
        }
    }

    fn security_location(&self, dir: &PluginDir, id: &NamespaceId) -> Result<StorageLocation> {
        let path = namespace_path(dir, id)?;
        let root = dir.root().canonicalize()?;
        match &self.backend {
            StorageBackend::Postgres(_) => Ok(StorageLocation::Postgres {
                data_root: root,
                schema: crate::plugin_host::postgres_schema_name(id.as_str())?,
            }),
            StorageBackend::Sqlite(db) => {
                let database = db
                    .durable_database_path()
                    .context("CoreSecurity requires file-backed SQLite")?
                    .canonicalize()?;
                let relative = path.strip_prefix(dir.root())?;
                Ok(StorageLocation::Sqlite {
                    database,
                    namespace: root.join(relative),
                })
            }
        }
    }

    pub(crate) async fn open_core_passkeys(
        &self,
        config: &Config,
        dir: &PluginDir,
    ) -> Result<PasskeyStorage> {
        self.open_core_passkeys_with(config, dir, |_| Ok(())).await
    }

    #[cfg(test)]
    pub(crate) async fn test_security_interruption(
        &self,
        config: &Config,
        dir: &PluginDir,
        point: HandoffPoint,
    ) -> Result<()> {
        self.open_core_passkeys_with(config, dir, |current| {
            ensure!(
                current != point,
                "injected CoreSecurity handoff interruption"
            );
            Ok(())
        })
        .await
        .map(|_| ())
    }

    async fn open_core_passkeys_with(
        &self,
        config: &Config,
        dir: &PluginDir,
        checkpoint: impl Fn(HandoffPoint) -> Result<()>,
    ) -> Result<PasskeyStorage> {
        let mut existing = self.security_authority().await?;
        let marker = Authority::inspect(dir)?;
        if let Some(marker) = &marker {
            ensure!(
                existing
                    .as_ref()
                    .is_some_and(|(authority, _)| authority == marker),
                "CoreSecurity marker has no matching database authority; restore the original database"
            );
        }
        let location = self.security_location(dir, &config.passkeys.namespace_id)?;
        if existing.is_none() {
            match config.passkeys.source {
                Source::Fresh => self.validate_fresh_security(dir).await?,
                Source::AdoptLegacy => {
                    let backend = self
                        .security_namespace(dir, &config.passkeys.namespace_id, false)
                        .await?;
                    PasskeyStorage::validate_adoption(&backend).await?;
                }
            }
            let authority = Authority {
                schema: 1,
                binding_id: uuid::Uuid::new_v4().simple().to_string(),
                passkeys: config.passkeys.clone(),
                location: location.clone(),
            };
            authority.validate()?;
            let encoded = serde_json::to_string(&authority)?;
            ensure!(
                encoded.len() <= 8192,
                "CoreSecurity authority exceeds its budget"
            );
            let now = chrono::Utc::now().timestamp_millis();
            query_core!(self, sqlx::query(
                "INSERT INTO core_security_authority (singleton, authority, phase, created_at_ms, updated_at_ms) \
                 VALUES (1, $1, 'prepared', $2, $2) ON CONFLICT (singleton) DO NOTHING"
            ).bind(&encoded).bind(now), execute);
            existing = self.security_authority().await?;
        }
        let (authority, phase) = existing.context("CoreSecurity prepare is missing")?;
        ensure!(
            authority.passkeys == config.passkeys && authority.location == location,
            "CoreSecurity namespace/configuration conflicts with durable authority"
        );
        checkpoint(HandoffPoint::Prepared)?;
        // Intent is committed first. A crash before marker publication resumes
        // from that exact row; a missing config already fails the DB guard.
        authority.record(dir)?;
        checkpoint(HandoffPoint::Marked)?;
        let fresh = phase == Phase::Prepared && config.passkeys.source == Source::Fresh;
        let backend = self
            .security_namespace(dir, &config.passkeys.namespace_id, fresh)
            .await?;
        let storage = PasskeyStorage::open_core(backend, &authority, phase, &checkpoint).await?;
        if matches!(self.backend, StorageBackend::Sqlite(_)) {
            let path = namespace_path(dir, &config.passkeys.namespace_id)?;
            // New directory entries must survive before the core DB reports
            // ready; namespace WAL commits use synchronous=FULL as well.
            for parent in path.ancestors().skip(1) {
                std::fs::File::open(parent)?.sync_all()?;
                if parent == dir.root() {
                    break;
                }
            }
        }
        checkpoint(HandoffPoint::NamespaceCommitted)?;
        if phase == Phase::Prepared {
            let encoded = serde_json::to_string(&authority)?;
            let changed = query_core!(
                self,
                sqlx::query(
                    "UPDATE core_security_authority SET phase = 'ready', updated_at_ms = $2 \
                 WHERE singleton = 1 AND authority = $1 AND phase = 'prepared'"
                )
                .bind(&encoded)
                .bind(chrono::Utc::now().timestamp_millis()),
                execute
            );
            ensure!(
                changed == 1 || self.security_authority().await? == Some((authority, Phase::Ready)),
                "CoreSecurity ready commit lost authority"
            );
        }
        Ok(storage)
    }

    async fn validate_fresh_security(&self, dir: &PluginDir) -> Result<()> {
        let snapshot = self.export_passkey_snapshot().await?;
        ensure!(
            snapshot.user.is_empty() && snapshot.admin.is_empty() && snapshot.ceremonies.is_empty(),
            "fresh CoreSecurity refuses legacy credentials or ceremonies; use an accepted legacy import and adopt_legacy"
        );
        // Fresh means no previous local storage authority, not merely a new ID.
        for entry in std::fs::read_dir(dir.live_root())? {
            let entry = entry?;
            ensure!(
                entry.file_type()?.is_dir()
                    && !entry.path().join("state").try_exists()?
                    && !entry.path().join(".catalog-authority-v1").try_exists()?,
                "fresh CoreSecurity refuses existing Plugin namespaces; use adopt_legacy"
            );
        }
        if let StorageBackend::Postgres(db) = &self.backend {
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM pg_namespace WHERE nspname LIKE 'plugin\\_%' ESCAPE '\\'",
            )
            .fetch_one(&db.pool)
            .await?;
            ensure!(
                count == 0,
                "fresh CoreSecurity refuses existing PostgreSQL Plugin namespaces"
            );
        }
        Ok(())
    }

    async fn security_namespace(
        &self,
        dir: &PluginDir,
        id: &NamespaceId,
        create: bool,
    ) -> Result<NamespaceBackend> {
        let path = namespace_path(dir, id)?;
        match &self.backend {
            StorageBackend::Postgres(db) => {
                let schema = crate::plugin_host::postgres_schema_name(id.as_str())?;
                if create {
                    sqlx::query(&format!("CREATE SCHEMA IF NOT EXISTS {schema}"))
                        .execute(&db.pool)
                        .await?;
                } else {
                    let exists: bool = sqlx::query_scalar(
                        "SELECT EXISTS(SELECT 1 FROM pg_namespace WHERE nspname = $1)",
                    )
                    .bind(&schema)
                    .fetch_one(&db.pool)
                    .await?;
                    ensure!(
                        exists,
                        "CoreSecurity namespace is missing; refusing recreation"
                    );
                }
                Ok(NamespaceBackend::Postgres {
                    pool: db.pool.clone(),
                    schema,
                })
            }
            StorageBackend::Sqlite(_) => {
                if create {
                    dir.ensure_state_dir(id.as_str())?;
                    namespace_path(dir, id)?;
                } else {
                    ensure!(
                        path.try_exists()?,
                        "CoreSecurity namespace is missing; refusing recreation"
                    );
                }
                validate_sqlite_sidecars(&path)?;
                let options = SqliteConnectOptions::new()
                    .filename(&path)
                    .create_if_missing(create)
                    .foreign_keys(true)
                    .journal_mode(SqliteJournalMode::Wal)
                    .synchronous(SqliteSynchronous::Full)
                    .busy_timeout(Duration::from_secs(5));
                let pool = SqlitePoolOptions::new()
                    .max_connections(4)
                    .acquire_timeout(Duration::from_secs(5))
                    .connect_with(options)
                    .await
                    .context("opening core Passkey namespace")?;
                Ok(NamespaceBackend::Sqlite(pool))
            }
        }
    }
}

fn validate_sqlite_sidecars(path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt as _;
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        match std::fs::symlink_metadata(Path::new(&name)) {
            Ok(metadata) => ensure!(
                metadata.is_file() && metadata.nlink() == 1,
                "unsafe CoreSecurity SQLite sidecar"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}
