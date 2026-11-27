//! Rollback-safe Draft Trash metadata, atomic optimistic purge, retention
//! sweeps, tombstones, and folder-only draft reparenting.

use super::{DraftDocument, DraftKind, StorageBackend, Store, legacy_stored};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Exact optimistic concurrency target for explicit permanent draft deletion.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PurgeDraftTarget {
    pub id: String,
    pub expected_revision: i64,
    pub expected_trashed_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PurgeDraftError {
    Conflict(String),
    Invalid(String),
}

/// Persist a discovery-time timestamp for a legacy deleted draft that had no
/// `draft_document_trash_meta` row, guaranteeing one full retention window
/// instead of using a stale `updated_at_ms`.
pub(super) async fn ensure_legacy_trash_timestamps(
    store: &Store,
    owner: &str,
    docs: &mut [DraftDocument],
) -> Result<()> {
    let missing: Vec<String> = docs
        .iter()
        .filter(|doc| doc.deleted && doc.trashed_at_ms.is_none())
        .map(|doc| doc.id.clone())
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let now = chrono::Utc::now().timestamp_millis();
    macro_rules! backfill {
        ($db:expr) => {{
            let mut resolved = HashMap::with_capacity(missing.len());
            for id in &missing {
                sqlx::query(
                    "INSERT INTO draft_document_trash_meta (owner_user_id, document_id, trashed_at_ms) \
                     VALUES ($1, $2, $3) ON CONFLICT (owner_user_id, document_id) DO NOTHING",
                )
                .bind(owner)
                .bind(id)
                .bind(now)
                .execute(&$db.pool)
                .await?;
                if let Some(ts) = sqlx::query_scalar::<_, i64>(
                    "SELECT trashed_at_ms FROM draft_document_trash_meta \
                     WHERE owner_user_id = $1 AND document_id = $2",
                )
                .bind(owner)
                .bind(id)
                .fetch_optional(&$db.pool)
                .await?
                {
                    resolved.insert(id.clone(), ts);
                }
            }
            resolved
        }};
    }
    let resolved = match &store.backend {
        StorageBackend::Postgres(db) => backfill!(db),
        StorageBackend::Sqlite(db) => backfill!(db),
    };
    for doc in docs.iter_mut() {
        if doc.deleted && doc.trashed_at_ms.is_none() {
            doc.trashed_at_ms = resolved.get(&doc.id).copied();
        }
    }
    Ok(())
}

impl Store {
    /// Load a trashed draft's full document (with Trash metadata) plus any
    /// preserved raw ACP `content` blocks JSON for verbatim session restore.
    pub async fn draft_trash_for_restore(
        &self,
        owner: &str,
        id: &str,
    ) -> Result<Option<(DraftDocument, Option<String>)>> {
        macro_rules! read {
            ($db:expr) => {
                sqlx::query_as::<_, (String, Option<i64>, Option<String>, Option<String>)>(
                    "SELECT d.value, m.trashed_at_ms, m.source_session_id, m.raw_content_json \
                     FROM draft_documents d \
                     LEFT JOIN draft_document_trash_meta m \
                       ON m.owner_user_id = d.owner_user_id AND m.document_id = d.id \
                     WHERE d.owner_user_id = $1 AND d.id = $2",
                )
                .bind(owner)
                .bind(id)
                .fetch_optional(&$db.pool)
                .await?
            };
        }
        let row = match &self.backend {
            StorageBackend::Postgres(db) => read!(db),
            StorageBackend::Sqlite(db) => read!(db),
        };
        let Some((value, trashed_at_ms, source_session_id, raw_content_json)) = row else {
            return Ok(None);
        };
        let mut doc: DraftDocument = serde_json::from_str(&value)?;
        doc.trashed_at_ms = trashed_at_ms;
        doc.source_session_id = source_session_id;
        ensure_legacy_trash_timestamps(self, owner, std::slice::from_mut(&mut doc)).await?;
        Ok(Some((doc, raw_content_json)))
    }

    /// Permanently delete trashed drafts whose observed `revision` and
    /// `trashed_at_ms` still match inside one serialized transaction.
    /// Keeps `draft_document_operations` receipts and records tombstones so a
    /// retried `Create` can never resurrect purged content.
    #[allow(clippy::too_many_lines)] // dual-backend atomic transaction macro
    pub async fn purge_trashed_drafts(
        &self,
        owner: &str,
        targets: &[PurgeDraftTarget],
    ) -> Result<std::result::Result<Vec<(String, i64)>, PurgeDraftError>> {
        if targets.is_empty() || targets.len() > 500 {
            return Ok(Err(PurgeDraftError::Invalid(
                "Provide between 1 and 500 draft targets".to_owned(),
            )));
        }
        let now = chrono::Utc::now().timestamp_millis();
        macro_rules! purge_tx {
            ($db:expr, $folder_query:literal) => {{
                let mut tx = $db.pool.begin().await?;
                sqlx::query(
                    "INSERT INTO draft_document_owners (owner_user_id, revision) VALUES ($1, 1) \
                     ON CONFLICT (owner_user_id) DO UPDATE SET revision = draft_document_owners.revision + 1",
                )
                .bind(owner)
                .execute(&mut *tx)
                .await?;
                let folder_rows: Vec<(String, Option<i64>)> =
                    sqlx::query_as($folder_query).bind(owner).fetch_all(&mut *tx).await?;
                let trashed_folders: HashMap<String, i64> = folder_rows
                    .into_iter()
                    .filter_map(|(fid, ts)| ts.map(|at| (fid, at)))
                    .collect();

                let mut doomed: Vec<(String, i64)> = Vec::with_capacity(targets.len());
                let mut already_purged: Vec<(String, i64)> = Vec::new();
                let mut seen = std::collections::HashSet::with_capacity(targets.len());
                for target in targets {
                    if !super::valid_id(&target.id) {
                        return Ok(Err(PurgeDraftError::Invalid("Invalid draft id".to_owned())));
                    }
                    if !seen.insert(target.id.as_str()) {
                        continue;
                    }
                    let row: Option<(String, Option<i64>)> = sqlx::query_as(
                        "SELECT d.metadata, m.trashed_at_ms \
                         FROM draft_documents d \
                         LEFT JOIN draft_document_trash_meta m \
                           ON m.owner_user_id = d.owner_user_id AND m.document_id = d.id \
                         WHERE d.owner_user_id = $1 AND d.id = $2",
                    )
                    .bind(owner)
                    .bind(&target.id)
                    .fetch_optional(&mut *tx)
                    .await?;
                    let Some((meta, meta_trashed_at)) = row else {
                        if let Some(tomb_rev) = sqlx::query_scalar::<_, i64>(
                            "SELECT revision FROM draft_document_tombstones \
                             WHERE owner_user_id = $1 AND document_id = $2",
                        )
                        .bind(owner)
                        .bind(&target.id)
                        .fetch_optional(&mut *tx)
                        .await?
                        {
                            already_purged.push((target.id.clone(), tomb_rev));
                            continue;
                        }
                        return Ok(Err(PurgeDraftError::Conflict(
                            "Draft changed since Trash was opened; refresh and try again"
                                .to_owned(),
                        )));
                    };
                    let doc: DraftDocument = serde_json::from_str(&meta)?;
                    if doc.revision != target.expected_revision {
                        return Ok(Err(PurgeDraftError::Conflict(
                            "Draft changed since Trash was opened; refresh and try again"
                                .to_owned(),
                        )));
                    }
                    let eligible = if doc.deleted {
                        meta_trashed_at == Some(target.expected_trashed_at_ms)
                    } else {
                        doc.parent_id
                            .as_deref()
                            .and_then(|parent| trashed_folders.get(parent).copied())
                            == Some(target.expected_trashed_at_ms)
                    };
                    if !eligible {
                        return Ok(Err(PurgeDraftError::Conflict(
                            "Only unchanged items still in Trash can be deleted permanently"
                                .to_owned(),
                        )));
                    }
                    doomed.push((doc.id, doc.revision + 1));
                }
                for (id, purged_rev) in &doomed {
                    sqlx::query(
                        "INSERT INTO draft_document_tombstones \
                         (owner_user_id, document_id, revision, purged_at_ms) \
                         VALUES ($1, $2, $3, $4) \
                         ON CONFLICT (owner_user_id, document_id) DO UPDATE SET \
                           revision = CASE WHEN draft_document_tombstones.revision > excluded.revision \
                             THEN draft_document_tombstones.revision ELSE excluded.revision END, \
                           purged_at_ms = excluded.purged_at_ms",
                    )
                    .bind(owner)
                    .bind(id)
                    .bind(*purged_rev)
                    .bind(now)
                    .execute(&mut *tx)
                    .await?;
                    sqlx::query(
                        "DELETE FROM draft_document_history WHERE owner_user_id = $1 AND document_id = $2",
                    )
                    .bind(owner)
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
                    sqlx::query(
                        "DELETE FROM draft_document_trash_meta WHERE owner_user_id = $1 AND document_id = $2",
                    )
                    .bind(owner)
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
                    sqlx::query(
                        "DELETE FROM draft_documents WHERE owner_user_id = $1 AND id = $2",
                    )
                    .bind(owner)
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
                }
                tx.commit().await?;
                already_purged.extend(doomed);
                already_purged
            }};
        }
        let purged = match &self.backend {
            StorageBackend::Postgres(db) => purge_tx!(
                db,
                "SELECT id, trashed_at_ms FROM session_folders \
                 WHERE owner_user_id IS NULL OR owner_user_id = $1 FOR UPDATE"
            ),
            StorageBackend::Sqlite(db) => purge_tx!(
                db,
                "SELECT id, trashed_at_ms FROM session_folders \
                 WHERE owner_user_id IS NULL OR owner_user_id = $1"
            ),
        };
        Ok(Ok(purged))
    }

    /// Every deleted draft across all owners for the Trash retention sweep:
    /// `(owner_user_id, id, trashed_at_ms)`. Any legacy deleted draft lacking
    /// `draft_document_trash_meta` persists `now_ms` first, granting one full
    /// retention window instead of judging by stale `updated_at_ms`.
    pub async fn list_trashed_drafts_for_retention(&self) -> Result<Vec<(String, String, i64)>> {
        macro_rules! scan {
            ($db:expr, $cond:literal) => {{
                sqlx::query_as::<_, (String, String, Option<i64>)>(concat!(
                    "SELECT d.owner_user_id, d.id, m.trashed_at_ms \
                     FROM draft_documents d \
                     LEFT JOIN draft_document_trash_meta m \
                       ON m.owner_user_id = d.owner_user_id AND m.document_id = d.id \
                     WHERE ",
                    $cond,
                ))
                .fetch_all(&$db.pool)
                .await?
            }};
        }
        let rows = match &self.backend {
            StorageBackend::Postgres(db) => {
                scan!(db, "(d.metadata::jsonb->>'deleted') = 'true'")
            }
            StorageBackend::Sqlite(db) => scan!(db, "json_extract(d.metadata, '$.deleted') = 1"),
        };
        let now = chrono::Utc::now().timestamp_millis();
        let mut out = Vec::with_capacity(rows.len());
        for (owner, id, trashed_at) in rows {
            if let Some(at) = trashed_at {
                out.push((owner, id, at));
                continue;
            }
            macro_rules! seed_one {
                ($db:expr) => {{
                    sqlx::query(
                        "INSERT INTO draft_document_trash_meta (owner_user_id, document_id, trashed_at_ms) \
                         VALUES ($1, $2, $3) ON CONFLICT (owner_user_id, document_id) DO NOTHING",
                    )
                    .bind(&owner)
                    .bind(&id)
                    .bind(now)
                    .execute(&$db.pool)
                    .await?;
                    sqlx::query_scalar::<_, i64>(
                        "SELECT trashed_at_ms FROM draft_document_trash_meta \
                         WHERE owner_user_id = $1 AND document_id = $2",
                    )
                    .bind(&owner)
                    .bind(&id)
                    .fetch_optional(&$db.pool)
                    .await?
                }};
            }
            let persisted = match &self.backend {
                StorageBackend::Postgres(db) => seed_one!(db),
                StorageBackend::Sqlite(db) => seed_one!(db),
            };
            if let Some(at) = persisted {
                out.push((owner, id, at));
            }
        }
        Ok(out)
    }

    /// Permanently purge one expired trashed draft only if it is still deleted
    /// at the exact persisted `expected_trashed_at_ms` judged by retention.
    /// Returns `Some(purged_revision)` when deleted.
    pub async fn purge_expired_draft(
        &self,
        owner: &str,
        id: &str,
        expected_trashed_at_ms: i64,
    ) -> Result<Option<i64>> {
        let now = chrono::Utc::now().timestamp_millis();
        macro_rules! purge_one {
            ($db:expr) => {{
                let mut tx = $db.pool.begin().await?;
                sqlx::query(
                    "INSERT INTO draft_document_owners (owner_user_id, revision) VALUES ($1, 1) \
                     ON CONFLICT (owner_user_id) DO UPDATE SET revision = draft_document_owners.revision + 1",
                )
                .bind(owner)
                .execute(&mut *tx)
                .await?;
                let row: Option<(String, Option<i64>)> = sqlx::query_as(
                    "SELECT d.metadata, m.trashed_at_ms \
                     FROM draft_documents d \
                     LEFT JOIN draft_document_trash_meta m \
                       ON m.owner_user_id = d.owner_user_id AND m.document_id = d.id \
                     WHERE d.owner_user_id = $1 AND d.id = $2",
                )
                .bind(owner)
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
                let Some((meta, trashed_at)) = row else {
                    return Ok(None);
                };
                let doc: DraftDocument = serde_json::from_str(&meta)?;
                if !doc.deleted || trashed_at != Some(expected_trashed_at_ms) {
                    return Ok(None);
                }
                let purged_rev = doc.revision + 1;
                sqlx::query(
                    "INSERT INTO draft_document_tombstones \
                     (owner_user_id, document_id, revision, purged_at_ms) \
                     VALUES ($1, $2, $3, $4) \
                     ON CONFLICT (owner_user_id, document_id) DO UPDATE SET \
                       revision = CASE WHEN draft_document_tombstones.revision > excluded.revision \
                         THEN draft_document_tombstones.revision ELSE excluded.revision END, \
                       purged_at_ms = excluded.purged_at_ms",
                )
                .bind(owner)
                .bind(id)
                .bind(purged_rev)
                .bind(now)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "DELETE FROM draft_document_history WHERE owner_user_id = $1 AND document_id = $2",
                )
                .bind(owner)
                .bind(id)
                .execute(&mut *tx)
                .await?;
                sqlx::query(
                    "DELETE FROM draft_document_trash_meta WHERE owner_user_id = $1 AND document_id = $2",
                )
                .bind(owner)
                .bind(id)
                .execute(&mut *tx)
                .await?;
                sqlx::query("DELETE FROM draft_documents WHERE owner_user_id = $1 AND id = $2")
                    .bind(owner)
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
                Some(purged_rev)
            }};
        }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => purge_one!(db),
            StorageBackend::Sqlite(db) => purge_one!(db),
        })
    }

    /// Atomically move every live draft directly inside `folder_id` up to
    /// `destination` under the owner's writer lock. Used by folder-only Trash
    /// so no concurrent or locally unloaded draft can ever be swept into Trash.
    pub async fn reparent_drafts_out_of_folder(
        &self,
        owner: &str,
        folder_id: &str,
        destination: Option<&str>,
    ) -> Result<Vec<DraftDocument>> {
        let now = chrono::Utc::now().timestamp_millis();
        macro_rules! reparent {
            ($db:expr) => {{
                let mut tx = $db.pool.begin().await?;
                sqlx::query(
                    "INSERT INTO draft_document_owners (owner_user_id, revision) VALUES ($1, 1) \
                     ON CONFLICT (owner_user_id) DO UPDATE SET revision = draft_document_owners.revision + 1",
                )
                .bind(owner)
                .execute(&mut *tx)
                .await?;
                let rows: Vec<String> = sqlx::query_scalar(
                    "SELECT metadata FROM draft_documents WHERE owner_user_id = $1 ORDER BY id",
                )
                .bind(owner)
                .fetch_all(&mut *tx)
                .await?;
                let mut moved = Vec::new();
                for raw in rows {
                    let meta: DraftDocument = serde_json::from_str(&raw)?;
                    if meta.deleted
                        || meta.kind != DraftKind::Document
                        || meta.parent_id.as_deref() != Some(folder_id)
                    {
                        continue;
                    }
                    let full: String = sqlx::query_scalar(
                        "SELECT value FROM draft_documents WHERE owner_user_id = $1 AND id = $2",
                    )
                    .bind(owner)
                    .bind(&meta.id)
                    .fetch_one(&mut *tx)
                    .await?;
                    let mut doc: DraftDocument = serde_json::from_str(&full)?;
                    doc.parent_id = destination.map(str::to_owned);
                    doc.revision += 1;
                    doc.metadata_revision += 1;
                    doc.updated_at_ms = now;
                    let stored = legacy_stored(&doc);
                    let mut stored_meta = stored.clone();
                    stored_meta.body.clear();
                    stored_meta.attachments.clear();
                    sqlx::query(
                        "UPDATE draft_documents SET value = $3, metadata = $4 \
                         WHERE owner_user_id = $1 AND id = $2",
                    )
                    .bind(owner)
                    .bind(&doc.id)
                    .bind(serde_json::to_string(&stored)?)
                    .bind(serde_json::to_string(&stored_meta)?)
                    .execute(&mut *tx)
                    .await?;
                    moved.push(doc);
                }
                tx.commit().await?;
                moved
            }};
        }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => reparent!(db),
            StorageBackend::Sqlite(db) => reparent!(db),
        })
    }

    /// Look up the imported shared `session_folders.id` for a legacy draft folder.
    pub async fn draft_folder_import_id(
        &self,
        owner: &str,
        document_id: &str,
    ) -> Result<Option<String>> {
        macro_rules! read {
            ($db:expr) => {
                sqlx::query_scalar::<_, String>(
                    "SELECT folder_id FROM draft_workspace_folder_imports \
                     WHERE owner_user_id = $1 AND document_id = $2",
                )
                .bind(owner)
                .bind(document_id)
                .fetch_optional(&$db.pool)
                .await?
            };
        }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => read!(db),
            StorageBackend::Sqlite(db) => read!(db),
        })
    }
}

macro_rules! sync_trash_meta_tx {
    ($tx:expr, $owner:expr, $next:expr, $change:expr, $now:expr, $raw_content_json:expr) => {
        if $next.deleted {
            sqlx::query(
                "INSERT INTO draft_document_trash_meta \
                 (owner_user_id, document_id, trashed_at_ms, source_session_id, raw_content_json) \
                 VALUES ($1, $2, $3, $4, $5) \
                 ON CONFLICT (owner_user_id, document_id) DO UPDATE SET \
                   trashed_at_ms = excluded.trashed_at_ms, \
                   source_session_id = excluded.source_session_id, \
                   raw_content_json = COALESCE(excluded.raw_content_json, draft_document_trash_meta.raw_content_json)",
            )
            .bind($owner)
            .bind(&$next.id)
            .bind($next.trashed_at_ms.unwrap_or($now))
            .bind($next.source_session_id.as_deref())
            .bind($raw_content_json)
            .execute(&mut *$tx)
            .await?;
        } else {
            sqlx::query("DELETE FROM draft_document_trash_meta WHERE owner_user_id = $1 AND document_id = $2")
                .bind($owner).bind(&$next.id).execute(&mut *$tx).await?;
            if matches!($change, $crate::store::draft_documents::DraftChange::Restore) && $next.kind == $crate::store::draft_documents::DraftKind::Folder {
                let folder_id = format!(
                    "f-draft-{}",
                    crate::digest::hex_sha256(&serde_json::to_vec(&($owner, &$next.id))?)
                );
                sqlx::query(
                    "INSERT INTO draft_workspace_folder_imports (owner_user_id, document_id, folder_id) \
                     VALUES ($1, $2, $3) ON CONFLICT (owner_user_id, document_id) DO NOTHING",
                )
                .bind($owner)
                .bind(&$next.id)
                .bind(&folder_id)
                .execute(&mut *$tx)
                .await?;
            }
        }
    };
}
pub(super) use sync_trash_meta_tx;
