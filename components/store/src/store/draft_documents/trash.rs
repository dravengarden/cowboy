//! Rollback-safe revision-bound Draft Trash metadata, atomic optimistic purge,
//! retention sweeps, tombstones, purged-op receipts, and folder-only reparenting.

use super::{DraftDocument, DraftKind, DraftResult, StorageBackend, Store, legacy_stored};
use crate::session_folders::SessionFolder;
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

/// Optional server-only archive metadata passed when archiving a session queue
/// draft to Trash.
#[derive(Clone, Debug, Default)]
pub struct DraftArchiveOptions<'a> {
    pub raw_content_json: Option<&'a str>,
    pub source_origin_json: Option<&'a str>,
    pub source_is_recovery: bool,
    pub request_hash_override: Option<&'a str>,
    pub allow_session_provenance: bool,
}

/// Outcome of a draft mutation including any newly imported shared folder
/// created atomically on a deleted->live Restore of a legacy `DraftKind::Folder`.
#[derive(Debug)]
pub struct DraftMutationOutcome {
    pub result: DraftResult,
    pub imported_folder: Option<SessionFolder>,
}

/// Full Trash archive record loaded for `restore-to-session`.
#[derive(Clone, Debug)]
pub struct DraftTrashRestoreRecord {
    pub document: DraftDocument,
    pub raw_content_json: Option<String>,
    pub source_origin_json: Option<String>,
    pub source_is_recovery: bool,
}

/// Recorded permanent-deletion tombstone (optionally carrying a winning
/// session-restore receipt for idempotent replay after a lost response).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraftTombstone {
    pub revision: i64,
    pub purged_at_ms: i64,
    pub restore_op_id: Option<String>,
    pub restored_row_id: Option<String>,
    pub restore_session_id: Option<String>,
    pub restore_expected_revision: Option<i64>,
    pub restore_expected_trashed_at_ms: Option<i64>,
}

/// Exact archive-bound restore receipt persisted on `draft_document_tombstones`.
#[derive(Clone, Copy, Debug)]
pub struct RestoreReceiptRef<'a> {
    pub op_id: &'a str,
    pub restored_row_id: &'a str,
    pub session_id: &'a str,
    pub expected_revision: i64,
    pub expected_trashed_at_ms: i64,
}

/// True when `raw_content_json` contains non-text ACP blocks (such as
/// `resource_link`) that are not represented in the standalone document's
/// `attachments`, so converting to a standalone draft via Restore would drop them.
pub(super) fn has_unrepresented_acp_blocks(
    raw_content_json: Option<&str>,
    body: &str,
    attachments: &[serde_json::Value],
) -> bool {
    let Some(raw) = raw_content_json else {
        return false;
    };
    let Ok(blocks) = serde_json::from_str::<Vec<serde_json::Value>>(raw) else {
        return true;
    };
    let mut text_blocks: Vec<&str> = Vec::new();
    let mut non_text: Vec<&serde_json::Value> = Vec::new();
    for block in &blocks {
        let Some(obj) = block.as_object() else {
            return true;
        };
        match obj.get("type").and_then(serde_json::Value::as_str) {
            Some("text") => {
                if obj.len() != 2 {
                    return true;
                }
                let Some(t) = obj.get("text").and_then(serde_json::Value::as_str) else {
                    return true;
                };
                text_blocks.push(t);
            }
            Some("image" | "resource") => non_text.push(block),
            _ => return true,
        }
    }
    match text_blocks.as_slice() {
        [] => {
            if !body.is_empty() {
                return true;
            }
        }
        [only] => {
            if *only != body {
                return true;
            }
        }
        _ => return true,
    }
    let mut remaining: Vec<Option<&serde_json::Value>> =
        attachments.iter().map(|att| att.get("block")).collect();
    if remaining.len() != non_text.len() || remaining.iter().any(Option::is_none) {
        return true;
    }
    for block in non_text {
        let Some(pos) = remaining.iter().position(|slot| *slot == Some(block)) else {
            return true;
        };
        remaining.swap_remove(pos);
    }
    false
}

/// Reconcile deleted drafts whose `draft_document_trash_meta` is missing or
/// bound to an older `revision` (e.g. after a rolled-back Controller restored,
/// edited, and re-trashed the document). Runs under the owner's writer lock so
/// discovery time is persisted once and stale raw blocks are cleared.
pub(super) async fn sync_revision_bound_trash_meta(
    store: &Store,
    owner: &str,
    rows: &mut [(DraftDocument, Option<i64>)],
) -> Result<()> {
    let mismatched: Vec<(String, i64)> = rows
        .iter()
        .filter(|(doc, meta_rev)| doc.deleted && *meta_rev != Some(doc.revision))
        .map(|(doc, _)| (doc.id.clone(), doc.revision))
        .collect();
    if mismatched.is_empty() {
        return Ok(());
    }
    let now = chrono::Utc::now().timestamp_millis();
    macro_rules! heal {
        ($db:expr) => {{
            let mut tx = $db.pool.begin().await?;
            sqlx::query(
                "INSERT INTO draft_document_owners (owner_user_id, revision) VALUES ($1, 1) \
                 ON CONFLICT (owner_user_id) DO UPDATE SET revision = draft_document_owners.revision + 1",
            )
            .bind(owner)
            .execute(&mut *tx)
            .await?;
            let mut resolved = HashMap::with_capacity(mismatched.len());
            for (id, observed_rev) in &mismatched {
                let Some(raw_doc) = sqlx::query_scalar::<_, String>(
                    "SELECT value FROM draft_documents WHERE owner_user_id = $1 AND id = $2",
                )
                .bind(owner)
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?
                else {
                    continue;
                };
                let cur: DraftDocument = serde_json::from_str(&raw_doc)?;
                if !cur.deleted {
                    continue;
                }
                let current_meta: Option<(i64, i64)> = sqlx::query_as(
                    "SELECT revision, trashed_at_ms FROM draft_document_trash_meta \
                     WHERE owner_user_id = $1 AND document_id = $2",
                )
                .bind(owner)
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
                let ts = if let Some((existing_rev, existing_ts)) = current_meta
                    && existing_rev == cur.revision
                {
                    existing_ts
                } else {
                    sqlx::query(
                        "INSERT INTO draft_document_trash_meta \
                         (owner_user_id, document_id, revision, trashed_at_ms, source_session_id, \
                          raw_content_json, source_origin_json, source_is_recovery) \
                         VALUES ($1, $2, $3, $4, NULL, NULL, NULL, 0) \
                         ON CONFLICT (owner_user_id, document_id) DO UPDATE SET \
                           revision = excluded.revision, \
                           trashed_at_ms = excluded.trashed_at_ms, \
                           source_session_id = NULL, \
                           raw_content_json = NULL, \
                           source_origin_json = NULL, \
                           source_is_recovery = 0",
                    )
                    .bind(owner)
                    .bind(id)
                    .bind(cur.revision)
                    .bind(now)
                    .execute(&mut *tx)
                    .await?;
                    now
                };
                if cur.revision == *observed_rev {
                    resolved.insert(id.clone(), ts);
                }
            }
            tx.commit().await?;
            resolved
        }};
    }
    let resolved = match &store.backend {
        StorageBackend::Postgres(db) => heal!(db),
        StorageBackend::Sqlite(db) => heal!(db),
    };
    for (doc, meta_rev) in rows.iter_mut() {
        if doc.deleted && *meta_rev != Some(doc.revision) {
            doc.trashed_at_ms = resolved.get(&doc.id).copied();
            doc.source_session_id = None;
            *meta_rev = Some(doc.revision);
        }
    }
    Ok(())
}

/// If a rolled-back Controller created a fresh document reusing a tombstoned ID
/// with `doc.revision <= tomb.revision`, advance its revision past the tombstone
/// and clear the stale tombstone under the writer lock so monotonic revisions
/// hold while `draft_document_purged_ops` still rejects pre-purge op replays.
/// Under the owner's writer lock, re-read BOTH `draft_documents` and
/// `draft_document_tombstones` for `id`. Never deletes a tombstone when no
/// surviving document exists (R3-9). When a rollback-recreated document does
/// exist with any clock `<= tomb_rev`, advances all revision clocks past
/// `tomb_rev`, carries matching Trash metadata forward, and only then removes
/// the superseded tombstone (R3-10).
macro_rules! reconcile_doc_tombstone_tx {
    ($tx:expr, $owner:expr, $id:expr) => {{
        let tomb_rev: Option<i64> = sqlx::query_scalar(
            "SELECT revision FROM draft_document_tombstones \
             WHERE owner_user_id = $1 AND document_id = $2",
        )
        .bind($owner)
        .bind($id)
        .fetch_optional(&mut *$tx)
        .await?;
        let mut reconciled: Option<$crate::store::draft_documents::DraftDocument> = None;
        if let Some(t_rev) = tomb_rev
            && let Some(raw) = sqlx::query_scalar::<_, String>(
                "SELECT value FROM draft_documents WHERE owner_user_id = $1 AND id = $2",
            )
            .bind($owner)
            .bind($id)
            .fetch_optional(&mut *$tx)
            .await?
        {
            let mut doc: $crate::store::draft_documents::DraftDocument =
                serde_json::from_str(&raw)?;
            let old_rev = doc.revision;
            if doc.revision <= t_rev || doc.body_revision <= t_rev || doc.metadata_revision <= t_rev
            {
                let floor = t_rev + 1;
                doc.revision = doc.revision.max(floor);
                doc.body_revision = doc.body_revision.max(doc.revision);
                doc.metadata_revision = doc.metadata_revision.max(doc.revision);
                let stored = $crate::store::draft_documents::legacy_stored(&doc);
                let mut stored_meta = stored.clone();
                stored_meta.body.clear();
                stored_meta.attachments.clear();
                sqlx::query(
                    "UPDATE draft_documents SET value = $3, metadata = $4 \
                     WHERE owner_user_id = $1 AND id = $2",
                )
                .bind($owner)
                .bind($id)
                .bind(serde_json::to_string(&stored)?)
                .bind(serde_json::to_string(&stored_meta)?)
                .execute(&mut *$tx)
                .await?;
                if doc.deleted {
                    sqlx::query(
                        "UPDATE draft_document_trash_meta SET revision = $3 \
                         WHERE owner_user_id = $1 AND document_id = $2 AND revision = $4",
                    )
                    .bind($owner)
                    .bind($id)
                    .bind(doc.revision)
                    .bind(old_rev)
                    .execute(&mut *$tx)
                    .await?;
                }
            }
            sqlx::query(
                "DELETE FROM draft_document_tombstones \
                 WHERE owner_user_id = $1 AND document_id = $2",
            )
            .bind($owner)
            .bind($id)
            .execute(&mut *$tx)
            .await?;
            reconciled = Some(doc);
        }
        reconciled
    }};
}
pub(super) use reconcile_doc_tombstone_tx;

pub(super) async fn reconcile_tombstone_collisions(
    store: &Store,
    owner: &str,
    docs: &mut [DraftDocument],
) -> Result<()> {
    if docs.is_empty() {
        return Ok(());
    }
    macro_rules! check_and_fix {
        ($db:expr) => {{
            let tombs: Vec<(String, i64)> = sqlx::query_as(
                "SELECT document_id, revision FROM draft_document_tombstones WHERE owner_user_id = $1",
            )
            .bind(owner)
            .fetch_all(&$db.pool)
            .await?;
            if tombs.is_empty() {
                return Ok(());
            }
            let tomb_map: HashMap<String, i64> = tombs.into_iter().collect();
            let collisions: Vec<String> = docs
                .iter()
                .filter(|doc| tomb_map.contains_key(&doc.id))
                .map(|doc| doc.id.clone())
                .collect();
            if collisions.is_empty() {
                return Ok(());
            }
            let mut tx = $db.pool.begin().await?;
            sqlx::query(
                "INSERT INTO draft_document_owners (owner_user_id, revision) VALUES ($1, 1) \
                 ON CONFLICT (owner_user_id) DO UPDATE SET revision = draft_document_owners.revision + 1",
            )
            .bind(owner)
            .execute(&mut *tx)
            .await?;
            let mut bumped = HashMap::new();
            for id in collisions {
                if let Some(healed) = reconcile_doc_tombstone_tx!(tx, owner, &id) {
                    bumped.insert(id, healed);
                }
            }
            tx.commit().await?;
            bumped
        }};
    }
    let bumped = match &store.backend {
        StorageBackend::Postgres(db) => check_and_fix!(db),
        StorageBackend::Sqlite(db) => check_and_fix!(db),
    };
    for doc in docs.iter_mut() {
        if let Some(healed) = bumped.get(&doc.id) {
            doc.revision = doc.revision.max(healed.revision);
            doc.body_revision = doc.body_revision.max(healed.body_revision);
            doc.metadata_revision = doc.metadata_revision.max(healed.metadata_revision);
        }
    }
    Ok(())
}

macro_rules! purge_doomed_draft_tx {
    ($tx:expr, $owner:expr, $id:expr, $purged_rev:expr, $now:expr, $receipt:expr) => {{
        let receipt_ref: Option<$crate::store::draft_documents::RestoreReceiptRef<'_>> = $receipt;
        sqlx::query(
            "INSERT INTO draft_document_purged_ops \
             (owner_user_id, operation_id, request_hash, document_id, purged_revision) \
             SELECT owner_user_id, operation_id, request_hash, document_id, $3 \
             FROM draft_document_operations \
             WHERE owner_user_id = $1 AND document_id = $2 \
             ON CONFLICT (owner_user_id, operation_id) DO NOTHING",
        )
        .bind($owner)
        .bind($id)
        .bind($purged_rev)
        .execute(&mut *$tx)
        .await?;
        sqlx::query(
            "INSERT INTO draft_document_tombstones \
             (owner_user_id, document_id, revision, purged_at_ms, restore_op_id, restored_row_id, \
              restore_session_id, restore_expected_revision, restore_expected_trashed_at_ms) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
             ON CONFLICT (owner_user_id, document_id) DO UPDATE SET \
               revision = CASE WHEN draft_document_tombstones.revision > excluded.revision \
                 THEN draft_document_tombstones.revision ELSE excluded.revision END, \
               purged_at_ms = excluded.purged_at_ms, \
               restore_op_id = COALESCE(excluded.restore_op_id, draft_document_tombstones.restore_op_id), \
               restored_row_id = COALESCE(excluded.restored_row_id, draft_document_tombstones.restored_row_id), \
               restore_session_id = COALESCE(excluded.restore_session_id, draft_document_tombstones.restore_session_id), \
               restore_expected_revision = COALESCE(excluded.restore_expected_revision, draft_document_tombstones.restore_expected_revision), \
               restore_expected_trashed_at_ms = COALESCE(excluded.restore_expected_trashed_at_ms, draft_document_tombstones.restore_expected_trashed_at_ms)",
        )
        .bind($owner)
        .bind($id)
        .bind($purged_rev)
        .bind($now)
        .bind(receipt_ref.map(|r| r.op_id))
        .bind(receipt_ref.map(|r| r.restored_row_id))
        .bind(receipt_ref.map(|r| r.session_id))
        .bind(receipt_ref.map(|r| r.expected_revision))
        .bind(receipt_ref.map(|r| r.expected_trashed_at_ms))
        .execute(&mut *$tx)
        .await?;
        sqlx::query(
            "DELETE FROM draft_document_history WHERE owner_user_id = $1 AND document_id = $2",
        )
        .bind($owner)
        .bind($id)
        .execute(&mut *$tx)
        .await?;
        sqlx::query(
            "DELETE FROM draft_document_trash_meta WHERE owner_user_id = $1 AND document_id = $2",
        )
        .bind($owner)
        .bind($id)
        .execute(&mut *$tx)
        .await?;
        sqlx::query("DELETE FROM draft_documents WHERE owner_user_id = $1 AND id = $2")
            .bind($owner)
            .bind($id)
            .execute(&mut *$tx)
            .await?;
    }};
}
pub(super) use purge_doomed_draft_tx;

impl Store {
    /// Look up a permanent-deletion tombstone for `(owner, id)`.
    pub async fn draft_tombstone(&self, owner: &str, id: &str) -> Result<Option<DraftTombstone>> {
        macro_rules! read {
            ($db:expr) => {
                sqlx::query_as::<
                    _,
                    (
                        i64,
                        i64,
                        Option<String>,
                        Option<String>,
                        Option<String>,
                        Option<i64>,
                        Option<i64>,
                    ),
                >(
                    "SELECT revision, purged_at_ms, restore_op_id, restored_row_id, \
                            restore_session_id, restore_expected_revision, restore_expected_trashed_at_ms \
                     FROM draft_document_tombstones WHERE owner_user_id = $1 AND document_id = $2",
                )
                .bind(owner)
                .bind(id)
                .fetch_optional(&$db.pool)
                .await?
                .map(
                    |(
                        revision,
                        purged_at_ms,
                        restore_op_id,
                        restored_row_id,
                        restore_session_id,
                        restore_expected_revision,
                        restore_expected_trashed_at_ms,
                    )| DraftTombstone {
                        revision,
                        purged_at_ms,
                        restore_op_id,
                        restored_row_id,
                        restore_session_id,
                        restore_expected_revision,
                        restore_expected_trashed_at_ms,
                    },
                )
            };
        }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => read!(db),
            StorageBackend::Sqlite(db) => read!(db),
        })
    }

    pub async fn draft_trash_for_restore(
        &self,
        owner: &str,
        id: &str,
    ) -> Result<Option<DraftTrashRestoreRecord>> {
        macro_rules! read {
            ($db:expr) => {
                sqlx::query_as::<_, (String, Option<i64>, Option<i64>, Option<String>, Option<String>, Option<String>, Option<i32>)>(
                    "SELECT d.value, m.revision, m.trashed_at_ms, m.source_session_id, \
                            m.raw_content_json, m.source_origin_json, m.source_is_recovery \
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
        let Some((value, meta_rev, trashed_at, source_session, raw_content, source_origin, is_rec)) =
            row
        else {
            return Ok(None);
        };
        let mut doc: DraftDocument = serde_json::from_str(&value)?;
        let rev_matches = doc.deleted && meta_rev == Some(doc.revision);
        if rev_matches {
            doc.trashed_at_ms = trashed_at;
            doc.source_session_id = source_session;
        }
        let mut pair = [(doc, if rev_matches { meta_rev } else { None })];
        sync_revision_bound_trash_meta(self, owner, &mut pair).await?;
        let (doc, _) = pair.into_iter().next().unwrap();
        Ok(Some(DraftTrashRestoreRecord {
            document: doc,
            raw_content_json: if rev_matches { raw_content } else { None },
            source_origin_json: if rev_matches { source_origin } else { None },
            source_is_recovery: rev_matches && is_rec.unwrap_or(0) != 0,
        }))
    }

    pub async fn purge_trashed_drafts(
        &self,
        owner: &str,
        targets: &[PurgeDraftTarget],
    ) -> Result<std::result::Result<Vec<(String, i64)>, PurgeDraftError>> {
        self.purge_trashed_drafts_with_restore_receipt(owner, targets, None)
            .await
    }

    /// Permanently delete trashed drafts whose observed `revision` and
    /// revision-bound `trashed_at_ms` still match inside one serialized tx.
    #[allow(clippy::too_many_lines)] // dual-backend atomic transaction macro
    pub async fn purge_trashed_drafts_with_restore_receipt(
        &self,
        owner: &str,
        targets: &[PurgeDraftTarget],
        restore_receipt: Option<RestoreReceiptRef<'_>>,
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
                    let row: Option<(String, Option<i64>, Option<i64>)> = sqlx::query_as(
                        "SELECT d.metadata, m.revision, m.trashed_at_ms \
                         FROM draft_documents d \
                         LEFT JOIN draft_document_trash_meta m \
                           ON m.owner_user_id = d.owner_user_id AND m.document_id = d.id \
                         WHERE d.owner_user_id = $1 AND d.id = $2",
                    )
                    .bind(owner)
                    .bind(&target.id)
                    .fetch_optional(&mut *tx)
                    .await?;
                    let Some((meta, meta_rev, meta_trashed_at)) = row else {
                        if restore_receipt.is_none()
                            && let Some(tomb_rev) = sqlx::query_scalar::<_, i64>(
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
                        meta_rev == Some(doc.revision)
                            && meta_trashed_at == Some(target.expected_trashed_at_ms)
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
                    purge_doomed_draft_tx!(tx, owner, id, *purged_rev, now, restore_receipt);
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
    /// `(owner_user_id, id, trashed_at_ms)`. Any legacy deleted draft or
    /// rollback-mismatched draft has its revision-bound timestamp healed first
    /// under the owner's writer lock.
    pub async fn list_trashed_drafts_for_retention(&self) -> Result<Vec<(String, String, i64)>> {
        macro_rules! scan {
            ($db:expr, $cond:literal) => {{
                sqlx::query_as::<_, (String, String, Option<i64>, Option<i64>)>(concat!(
                    "SELECT d.owner_user_id, d.metadata, m.revision, m.trashed_at_ms \
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
        let mut by_owner: HashMap<String, Vec<(DraftDocument, Option<i64>)>> = HashMap::new();
        for (owner, raw, meta_rev, trashed_at) in rows {
            let mut doc: DraftDocument = serde_json::from_str(&raw)?;
            if !doc.deleted {
                continue;
            }
            if meta_rev == Some(doc.revision) {
                doc.trashed_at_ms = trashed_at;
            }
            by_owner.entry(owner).or_default().push((doc, meta_rev));
        }
        let mut out = Vec::new();
        for (owner, mut items) in by_owner {
            sync_revision_bound_trash_meta(self, &owner, &mut items).await?;
            for (doc, _) in items {
                if let Some(at) = doc.trashed_at_ms {
                    out.push((owner.clone(), doc.id, at));
                }
            }
        }
        Ok(out)
    }

    /// Permanently purge one expired trashed draft only if it is still deleted
    /// at the exact revision-bound `expected_trashed_at_ms`.
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
                let row: Option<(String, Option<i64>, Option<i64>)> = sqlx::query_as(
                    "SELECT d.metadata, m.revision, m.trashed_at_ms \
                     FROM draft_documents d \
                     LEFT JOIN draft_document_trash_meta m \
                       ON m.owner_user_id = d.owner_user_id AND m.document_id = d.id \
                     WHERE d.owner_user_id = $1 AND d.id = $2",
                )
                .bind(owner)
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
                let Some((meta, meta_rev, trashed_at)) = row else {
                    return Ok(None);
                };
                let doc: DraftDocument = serde_json::from_str(&meta)?;
                if !doc.deleted
                    || meta_rev != Some(doc.revision)
                    || trashed_at != Some(expected_trashed_at_ms)
                {
                    return Ok(None);
                }
                let purged_rev = doc.revision + 1;
                sqlx::query(
                    "INSERT INTO draft_document_purged_ops \
                     (owner_user_id, operation_id, request_hash, document_id, purged_revision) \
                     SELECT owner_user_id, operation_id, request_hash, document_id, $3 \
                     FROM draft_document_operations \
                     WHERE owner_user_id = $1 AND document_id = $2 \
                     ON CONFLICT (owner_user_id, operation_id) DO NOTHING",
                )
                .bind(owner)
                .bind(id)
                .bind(purged_rev)
                .execute(&mut *tx)
                .await?;
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
    /// `destination`. When `owner_scope` is `None` (an unowned shared folder),
    /// reparents across every owner in `draft_document_owners`.
    pub async fn reparent_drafts_out_of_folder(
        &self,
        owner_scope: Option<&str>,
        folder_id: &str,
        destination: Option<&str>,
    ) -> Result<Vec<(String, DraftDocument)>> {
        let now = chrono::Utc::now().timestamp_millis();
        macro_rules! reparent {
            ($db:expr) => {{
                let mut tx = $db.pool.begin().await?;
                let owners: Vec<String> = if let Some(owner) = owner_scope {
                    vec![owner.to_owned()]
                } else {
                    sqlx::query_scalar(
                        "SELECT owner_user_id FROM draft_document_owners ORDER BY owner_user_id",
                    )
                    .fetch_all(&mut *tx)
                    .await?
                };
                let mut moved = Vec::new();
                for owner in owners {
                    sqlx::query(
                        "INSERT INTO draft_document_owners (owner_user_id, revision) VALUES ($1, 1) \
                         ON CONFLICT (owner_user_id) DO UPDATE SET revision = draft_document_owners.revision + 1",
                    )
                    .bind(&owner)
                    .execute(&mut *tx)
                    .await?;
                    let rows: Vec<String> = sqlx::query_scalar(
                        "SELECT metadata FROM draft_documents WHERE owner_user_id = $1 ORDER BY id",
                    )
                    .bind(&owner)
                    .fetch_all(&mut *tx)
                    .await?;
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
                        .bind(&owner)
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
                        .bind(&owner)
                        .bind(&doc.id)
                        .bind(serde_json::to_string(&stored)?)
                        .bind(serde_json::to_string(&stored_meta)?)
                        .execute(&mut *tx)
                        .await?;
                        moved.push((owner.clone(), doc));
                    }
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

    /// Move back only those drafts from a folder-only Undo whose `parent_id`
    /// and `metadata_revision` still match the original reparent.
    pub async fn undo_reparent_drafts_to_folder(
        &self,
        folder_id: &str,
        expected_parent: Option<&str>,
        targets: &[(String, String, i64)],
    ) -> Result<Vec<(String, DraftDocument)>> {
        if targets.is_empty() {
            return Ok(Vec::new());
        }
        let now = chrono::Utc::now().timestamp_millis();
        let mut by_owner: std::collections::BTreeMap<&str, Vec<(&str, i64)>> =
            std::collections::BTreeMap::new();
        for (owner, doc_id, meta_rev) in targets {
            by_owner
                .entry(owner.as_str())
                .or_default()
                .push((doc_id.as_str(), *meta_rev));
        }
        macro_rules! undo_reparent {
            ($db:expr) => {{
                let mut tx = $db.pool.begin().await?;
                let mut moved = Vec::new();
                for (owner, items) in &by_owner {
                    sqlx::query(
                        "INSERT INTO draft_document_owners (owner_user_id, revision) VALUES ($1, 1) \
                         ON CONFLICT (owner_user_id) DO UPDATE SET revision = draft_document_owners.revision + 1",
                    )
                    .bind(*owner)
                    .execute(&mut *tx)
                    .await?;
                    for &(doc_id, expected_meta_rev) in items {
                        let Some(full) = sqlx::query_scalar::<_, String>(
                            "SELECT value FROM draft_documents WHERE owner_user_id = $1 AND id = $2",
                        )
                        .bind(*owner)
                        .bind(doc_id)
                        .fetch_optional(&mut *tx)
                        .await?
                        else {
                            continue;
                        };
                        let mut doc: DraftDocument = serde_json::from_str(&full)?;
                        if doc.deleted
                            || doc.parent_id.as_deref() != expected_parent
                            || doc.metadata_revision != expected_meta_rev
                        {
                            continue;
                        }
                        doc.parent_id = Some(folder_id.to_owned());
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
                        .bind(*owner)
                        .bind(&doc.id)
                        .bind(serde_json::to_string(&stored)?)
                        .bind(serde_json::to_string(&stored_meta)?)
                        .execute(&mut *tx)
                        .await?;
                        moved.push(((*owner).to_owned(), doc));
                    }
                }
                tx.commit().await?;
                moved
            }};
        }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => undo_reparent!(db),
            StorageBackend::Sqlite(db) => undo_reparent!(db),
        })
    }
}

macro_rules! sync_trash_meta_tx {
    ($tx:expr, $owner:expr, $next:expr, $change:expr, $now:expr, $opts:expr, $insert_folder_sql:literal) => {{
        let mut imported_folder: Option<$crate::session_folders::SessionFolder> = None;
        if $next.deleted {
            sqlx::query(
                "INSERT INTO draft_document_trash_meta \
                 (owner_user_id, document_id, revision, trashed_at_ms, source_session_id, \
                  raw_content_json, source_origin_json, source_is_recovery) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
                 ON CONFLICT (owner_user_id, document_id) DO UPDATE SET \
                   revision = excluded.revision, \
                   trashed_at_ms = excluded.trashed_at_ms, \
                   source_session_id = excluded.source_session_id, \
                   raw_content_json = excluded.raw_content_json, \
                   source_origin_json = excluded.source_origin_json, \
                   source_is_recovery = excluded.source_is_recovery",
            )
            .bind($owner)
            .bind(&$next.id)
            .bind($next.revision)
            .bind($next.trashed_at_ms.unwrap_or($now))
            .bind($next.source_session_id.as_deref())
            .bind($opts.raw_content_json)
            .bind($opts.source_origin_json)
            .bind(i32::from($opts.source_is_recovery))
            .execute(&mut *$tx)
            .await?;
        } else {
            sqlx::query(
                "DELETE FROM draft_document_trash_meta WHERE owner_user_id = $1 AND document_id = $2",
            )
            .bind($owner)
            .bind(&$next.id)
            .execute(&mut *$tx)
            .await?;
            if matches!($change, $crate::store::draft_documents::DraftChange::Restore)
                && $next.kind == $crate::store::draft_documents::DraftKind::Folder
            {
                let folder_id = format!(
                    "f-draft-{}",
                    $crate::digest::hex_sha256(&serde_json::to_vec(&($owner, &$next.id))?)
                );
                let mut resolved_parent: Option<String> = None;
                if let Some(raw_parent) = $next.parent_id.as_deref() {
                    let mapped_parent: Option<String> = sqlx::query_scalar(
                        "SELECT folder_id FROM draft_workspace_folder_imports \
                         WHERE owner_user_id = $1 AND document_id = $2",
                    )
                    .bind($owner)
                    .bind(raw_parent)
                    .fetch_optional(&mut *$tx)
                    .await?;
                    let candidate = mapped_parent.as_deref().unwrap_or(raw_parent);
                    let live: Option<Option<i64>> = sqlx::query_scalar(
                        "SELECT trashed_at_ms FROM session_folders \
                         WHERE id = $1 AND (owner_user_id IS NULL OR owner_user_id = $2)",
                    )
                    .bind(candidate)
                    .bind($owner)
                    .fetch_optional(&mut *$tx)
                    .await?;
                    if live == Some(None) {
                        resolved_parent = Some(candidate.to_owned());
                    }
                }
                let position: i64 = sqlx::query_scalar(
                    "SELECT COALESCE(MAX(position), -1) + 1 FROM session_folders",
                )
                .fetch_one(&mut *$tx)
                .await?;
                let res = sqlx::query($insert_folder_sql)
                    .bind(&folder_id)
                    .bind($owner)
                    .bind(&$next.title)
                    .bind(resolved_parent.as_deref())
                    .bind(position)
                    .bind($now)
                    .execute(&mut *$tx)
                    .await?;
                sqlx::query(
                    "INSERT INTO draft_workspace_folder_imports (owner_user_id, document_id, folder_id) \
                     VALUES ($1, $2, $3) ON CONFLICT (owner_user_id, document_id) DO NOTHING",
                )
                .bind($owner)
                .bind(&$next.id)
                .bind(&folder_id)
                .execute(&mut *$tx)
                .await?;
                if res.rows_affected() > 0 {
                    imported_folder = Some($crate::session_folders::SessionFolder {
                        id: folder_id,
                        owner_user_id: Some($owner.to_owned()),
                        name: $next.title.clone(),
                        parent: resolved_parent,
                        project: None,
                        position,
                        defaults: Default::default(),
                        trashed_at_ms: None,
                    });
                }
            }
        }
        imported_folder
    }};
}
pub(super) use sync_trash_meta_tx;

macro_rules! resolve_tx_folders {
    ($tx:expr, $owner:expr, $shared:expr) => {{
        let db_folder_rows: Vec<(String, Option<String>, Option<i64>)> = sqlx::query_as(
            "SELECT id, parent_id, trashed_at_ms FROM session_folders \
             WHERE owner_user_id IS NULL OR owner_user_id = $1",
        )
        .bind($owner)
        .fetch_all(&mut *$tx)
        .await?;
        let db_folder_map: std::collections::HashMap<String, (Option<String>, Option<i64>)> =
            db_folder_rows
                .into_iter()
                .map(|(fid, pid, ts)| (fid, (pid, ts)))
                .collect();
        let mut effective_shared: Vec<$crate::session_folders::SessionFolder> = $shared
            .iter()
            .map(|f| {
                let mut copy = f.clone();
                if let Some((pid, ts)) = db_folder_map.get(&f.id) {
                    copy.parent = pid.clone();
                    copy.trashed_at_ms = *ts;
                }
                copy
            })
            .collect();
        for (fid, (pid, ts)) in &db_folder_map {
            if !effective_shared.iter().any(|f| f.id == *fid) {
                effective_shared.push($crate::session_folders::SessionFolder {
                    id: fid.clone(),
                    owner_user_id: Some($owner.to_owned()),
                    name: String::new(),
                    parent: pid.clone(),
                    project: None,
                    position: 0,
                    defaults: std::collections::BTreeMap::new(),
                    trashed_at_ms: *ts,
                });
            }
        }
        (db_folder_map, effective_shared)
    }};
}
pub(super) use resolve_tx_folders;

pub(super) fn resolve_live_restore_parent(
    parent_id: Option<String>,
    db_folder_map: &std::collections::HashMap<String, (Option<String>, Option<i64>)>,
    effective_shared: &[crate::session_folders::SessionFolder],
) -> Option<String> {
    let mut cursor = parent_id;
    let mut seen = std::collections::HashSet::new();
    while let Some(fid) = cursor.clone() {
        if !seen.insert(fid.clone()) {
            return None;
        }
        if let Some((pid, ts)) = db_folder_map.get(&fid) {
            if ts.is_some() {
                cursor.clone_from(pid);
                continue;
            }
            break;
        }
        if let Some(sf) = effective_shared.iter().find(|f| f.id == fid) {
            if sf.trashed_at_ms.is_some() {
                cursor.clone_from(&sf.parent);
                continue;
            }
            break;
        }
        break;
    }
    cursor
}
