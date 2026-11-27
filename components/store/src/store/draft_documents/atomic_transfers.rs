//! Single-transaction cross-table transfers for session drafts and folder-only
//! Trash/Undo so neither side is ever committed or visible without the other.

use super::trash::{
    DraftArchiveOptions, RestoreReceiptRef, purge_doomed_draft_tx, sync_trash_meta_tx,
};
use super::{
    DraftChange, DraftDocument, DraftMutation, DraftResult, StorageBackend, Store, apply,
    legacy_stored,
};
use crate::model::QueuedMessage;
use crate::session_folders::{FolderOwner, SessionFolder};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// Inputs for archiving one session queue draft to Trash and removing it from
/// `sessions.drafts` inside a single SQL transaction.
pub struct AtomicArchiveDraftArgs<'a> {
    pub owner: &'a str,
    pub session_id: &'a str,
    pub mutation: &'a DraftMutation,
    pub shared_folders: &'a [SessionFolder],
    pub opts: &'a DraftArchiveOptions<'a>,
    pub next_queue: &'a [QueuedMessage],
    pub next_drafts: &'a [QueuedMessage],
}

/// Inputs for restoring one trashed session draft into `sessions.drafts` and
/// purging its archive + recording its receipt inside a single SQL transaction.
pub struct AtomicRestoreDraftArgs<'a> {
    pub owner: &'a str,
    pub session_id: &'a str,
    pub draft_id: &'a str,
    pub operation_id: &'a str,
    pub expected_revision: i64,
    pub expected_trashed_at_ms: i64,
    pub restored_row_id: &'a str,
    pub next_queue: &'a [QueuedMessage],
    pub next_drafts: &'a [QueuedMessage],
}

/// Outcome of `Store::atomic_restore_session_draft`.
#[derive(Debug)]
pub enum AtomicRestoreOutcome {
    Committed { purged_revision: i64 },
    AlreadyRestored { restored_row_id: Option<String> },
    Purged,
    NotFound,
    Conflict(String),
}

/// Server-recorded session move inside a folder-only Trash receipt.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MovedSessionReceipt {
    pub session_id: String,
    pub was_implicit: bool,
}

/// Server-recorded draft move inside a folder-only Trash receipt.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MovedDraftReceipt {
    pub owner: String,
    pub id: String,
    pub revision: i64,
    pub metadata_revision: i64,
}

/// Server-owned payload stored in `folder_trash_only_receipts`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FolderTrashOnlyReceiptPayload {
    pub receipt_id: String,
    pub folder_id: String,
    pub owner: FolderOwner,
    pub trashed_at_ms: i64,
    pub destination: Option<String>,
    pub previous_project: Option<String>,
    pub moved_folders: Vec<String>,
    pub moved_sessions: Vec<MovedSessionReceipt>,
    pub moved_drafts: Vec<MovedDraftReceipt>,
}

/// Result of `Store::atomic_folder_trash_only`.
pub struct AtomicFolderTrashOutcome {
    pub receipt: FolderTrashOnlyReceiptPayload,
    pub moved_draft_docs: Vec<(String, DraftDocument)>,
}

fn strip_nul(v: &mut serde_json::Value) {
    match v {
        serde_json::Value::String(s) if s.contains('\0') => *s = s.replace('\0', ""),
        serde_json::Value::Array(a) => a.iter_mut().for_each(strip_nul),
        serde_json::Value::Object(m) => m.values_mut().for_each(strip_nul),
        _ => {}
    }
}

macro_rules! update_session_pending_tx {
    ($tx:expr, $session_id:expr, $queue:expr, $drafts:expr, $now:expr, postgres) => {{
        let mut queue_json = serde_json::to_value($queue).context("serialize queue")?;
        let mut drafts_json = serde_json::to_value($drafts).context("serialize drafts")?;
        strip_nul(&mut queue_json);
        strip_nul(&mut drafts_json);
        sqlx::query(
            "UPDATE sessions SET queue = $1, drafts = $2, updated_at = now() WHERE id = $3",
        )
        .bind(&queue_json)
        .bind(&drafts_json)
        .bind($session_id)
        .execute(&mut *$tx)
        .await?;
    }};
    ($tx:expr, $session_id:expr, $queue:expr, $drafts:expr, $now:expr, sqlite) => {{
        let mut queue_json = serde_json::to_value($queue).context("serialize queue")?;
        let mut drafts_json = serde_json::to_value($drafts).context("serialize drafts")?;
        strip_nul(&mut queue_json);
        strip_nul(&mut drafts_json);
        sqlx::query(
            "UPDATE sessions SET queue = ?1, drafts = ?2, updated_at_ms = ?3 WHERE id = ?4",
        )
        .bind(&queue_json)
        .bind(&drafts_json)
        .bind($now)
        .bind($session_id)
        .execute(&mut *$tx)
        .await?;
    }};
}

macro_rules! upsert_folders_by_owner_tx {
    ($tx:expr, $folders_by_owner:expr, $now:expr, postgres) => {{
        for (owner_slot, folders) in $folders_by_owner {
            let previous: HashMap<String, String> = sqlx::query_as::<_, (String, String)>(
                "SELECT id, defaults_json FROM session_folders \
                 WHERE owner_user_id IS NOT DISTINCT FROM $1 FOR UPDATE",
            )
            .bind(owner_slot.as_deref())
            .fetch_all(&mut *$tx)
            .await?
            .into_iter()
            .collect();
            let keep: Vec<String> = folders.iter().map(|f| f.id.clone()).collect();
            sqlx::query(
                "DELETE FROM session_folders \
                 WHERE owner_user_id IS NOT DISTINCT FROM $1 AND id <> ALL($2)",
            )
            .bind(owner_slot.as_deref())
            .bind(&keep)
            .execute(&mut *$tx)
            .await?;
            for folder in folders {
                sqlx::query(
                    "INSERT INTO session_folders \
                     (id, owner_user_id, name, parent_id, project, position, defaults_json, trashed_at_ms) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
                     ON CONFLICT (id) DO UPDATE SET owner_user_id = excluded.owner_user_id, \
                       name = excluded.name, parent_id = excluded.parent_id, \
                       project = excluded.project, position = excluded.position, \
                       defaults_json = excluded.defaults_json, trashed_at_ms = excluded.trashed_at_ms, \
                       updated_at = now()",
                )
                .bind(&folder.id)
                .bind(folder.owner_user_id.as_deref())
                .bind(&folder.name)
                .bind(folder.parent.as_deref())
                .bind(folder.project.as_deref())
                .bind(folder.position)
                .bind(crate::session_folders::defaults::stored_json(
                    &folder.defaults,
                    previous.get(&folder.id).map(String::as_str),
                )?)
                .bind(folder.trashed_at_ms)
                .execute(&mut *$tx)
                .await?;
            }
        }
    }};
    ($tx:expr, $folders_by_owner:expr, $now:expr, sqlite) => {{
        for (owner_slot, folders) in $folders_by_owner {
            let existing: HashMap<String, String> = sqlx::query_as::<_, (String, String)>(
                "SELECT id, defaults_json FROM session_folders WHERE owner_user_id IS ?1",
            )
            .bind(owner_slot.as_deref())
            .fetch_all(&mut *$tx)
            .await?
            .into_iter()
            .collect();
            for stale in existing
                .keys()
                .filter(|id| !folders.iter().any(|f| &f.id == *id))
            {
                sqlx::query("DELETE FROM session_folders WHERE id = ?1")
                    .bind(stale)
                    .execute(&mut *$tx)
                    .await?;
            }
            for folder in folders {
                sqlx::query(
                    "INSERT INTO session_folders \
                     (id, owner_user_id, name, parent_id, project, position, created_at_ms, updated_at_ms, defaults_json, trashed_at_ms) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?8, ?9) \
                     ON CONFLICT (id) DO UPDATE SET owner_user_id = excluded.owner_user_id, \
                       name = excluded.name, parent_id = excluded.parent_id, \
                       project = excluded.project, position = excluded.position, \
                       defaults_json = excluded.defaults_json, trashed_at_ms = excluded.trashed_at_ms, \
                       updated_at_ms = excluded.updated_at_ms",
                )
                .bind(&folder.id)
                .bind(folder.owner_user_id.as_deref())
                .bind(&folder.name)
                .bind(folder.parent.as_deref())
                .bind(folder.project.as_deref())
                .bind(folder.position)
                .bind($now)
                .bind(crate::session_folders::defaults::stored_json(
                    &folder.defaults,
                    existing.get(&folder.id).map(String::as_str),
                )?)
                .bind(folder.trashed_at_ms)
                .execute(&mut *$tx)
                .await?;
            }
        }
    }};
}

macro_rules! update_pins_tx {
    ($tx:expr, $pins:expr, $now:expr, postgres) => {{
        for (session_id, folder_id) in $pins {
            sqlx::query("UPDATE sessions SET folder_id = $1, updated_at = now() WHERE id = $2")
                .bind(folder_id.as_deref())
                .bind(session_id)
                .execute(&mut *$tx)
                .await?;
        }
    }};
    ($tx:expr, $pins:expr, $now:expr, sqlite) => {{
        for (session_id, folder_id) in $pins {
            sqlx::query("UPDATE sessions SET folder_id = ?1, updated_at_ms = ?2 WHERE id = ?3")
                .bind(folder_id.as_deref())
                .bind($now)
                .bind(session_id)
                .execute(&mut *$tx)
                .await?;
        }
    }};
}

macro_rules! reparent_folder_drafts_tx {
    ($tx:expr, $owner_scope:expr, $folder_id:expr, $destination:expr, $now:expr) => {{
        let owners: Vec<String> = if let Some(owner) = $owner_scope {
            vec![owner.to_owned()]
        } else {
            sqlx::query_scalar::<_, String>(
                "SELECT DISTINCT owner_user_id FROM draft_documents ORDER BY owner_user_id",
            )
            .fetch_all(&mut *$tx)
            .await?
        };
        let mut moved = Vec::new();
        for owner in owners {
            sqlx::query(
                "INSERT INTO draft_document_owners (owner_user_id, revision) VALUES ($1, 1) \
                 ON CONFLICT (owner_user_id) DO UPDATE SET revision = draft_document_owners.revision + 1",
            )
            .bind(&owner)
            .execute(&mut *$tx)
            .await?;
            let rows: Vec<(String, String)> = sqlx::query_as(
                "SELECT id, value FROM draft_documents WHERE owner_user_id = $1",
            )
            .bind(&owner)
            .fetch_all(&mut *$tx)
            .await?;
            for (doc_id, raw) in rows {
                let mut doc: DraftDocument = serde_json::from_str(&raw)?;
                if doc.deleted || doc.parent_id.as_deref() != Some($folder_id) {
                    continue;
                }
                doc.parent_id = $destination.map(str::to_owned);
                doc.revision += 1;
                doc.metadata_revision += 1;
                doc.updated_at_ms = $now;
                let stored = legacy_stored(&doc);
                let mut stored_meta = stored.clone();
                stored_meta.body.clear();
                stored_meta.attachments.clear();
                sqlx::query(
                    "UPDATE draft_documents SET value = $3, metadata = $4 \
                     WHERE owner_user_id = $1 AND id = $2",
                )
                .bind(&owner)
                .bind(&doc_id)
                .bind(serde_json::to_string(&stored)?)
                .bind(serde_json::to_string(&stored_meta)?)
                .execute(&mut *$tx)
                .await?;
                moved.push((owner.clone(), doc));
            }
        }
        moved
    }};
}

macro_rules! undo_reparent_folder_drafts_tx {
    ($tx:expr, $folder_id:expr, $expected_parent:expr, $targets:expr, $now:expr) => {{
        let mut by_owner: BTreeMap<&str, Vec<(&str, i64)>> = BTreeMap::new();
        for t in $targets {
            by_owner
                .entry(t.owner.as_str())
                .or_default()
                .push((t.id.as_str(), t.metadata_revision));
        }
        let mut restored = Vec::new();
        for (owner, items) in by_owner {
            sqlx::query(
                "INSERT INTO draft_document_owners (owner_user_id, revision) VALUES ($1, 1) \
                 ON CONFLICT (owner_user_id) DO UPDATE SET revision = draft_document_owners.revision + 1",
            )
            .bind(owner)
            .execute(&mut *$tx)
            .await?;
            for (doc_id, expected_meta_rev) in items {
                let Some(raw) = sqlx::query_scalar::<_, String>(
                    "SELECT value FROM draft_documents WHERE owner_user_id = $1 AND id = $2",
                )
                .bind(owner)
                .bind(doc_id)
                .fetch_optional(&mut *$tx)
                .await?
                else {
                    continue;
                };
                let mut doc: DraftDocument = serde_json::from_str(&raw)?;
                if doc.deleted
                    || doc.parent_id.as_deref() != $expected_parent
                    || doc.metadata_revision != expected_meta_rev
                {
                    continue;
                }
                doc.parent_id = Some($folder_id.to_owned());
                doc.revision += 1;
                doc.metadata_revision += 1;
                doc.updated_at_ms = $now;
                let stored = legacy_stored(&doc);
                let mut stored_meta = stored.clone();
                stored_meta.body.clear();
                stored_meta.attachments.clear();
                sqlx::query(
                    "UPDATE draft_documents SET value = $3, metadata = $4 \
                     WHERE owner_user_id = $1 AND id = $2",
                )
                .bind(owner)
                .bind(doc_id)
                .bind(serde_json::to_string(&stored)?)
                .bind(serde_json::to_string(&stored_meta)?)
                .execute(&mut *$tx)
                .await?;
                restored.push((owner.to_owned(), doc));
            }
        }
        restored
    }};
}

macro_rules! atomic_archive_tx {
    ($db:expr, $args:expr, $now:expr, $request_hash:expr, $insert_folder_sql:literal, $dialect:ident) => {{
        let mut tx = $db.pool.begin().await?;
        sqlx::query(
            "INSERT INTO draft_document_owners (owner_user_id, revision) VALUES ($1, 1) \
             ON CONFLICT (owner_user_id) DO UPDATE SET revision = draft_document_owners.revision + 1",
        )
        .bind($args.owner)
        .execute(&mut *tx)
        .await?;
        if let Some((purged_hash, purged_id, purged_rev)) =
            sqlx::query_as::<_, (String, String, i64)>(
                "SELECT request_hash, document_id, purged_revision \
                 FROM draft_document_purged_ops WHERE owner_user_id = $1 AND operation_id = $2",
            )
            .bind($args.owner)
            .bind(&$args.mutation.operation_id)
            .fetch_optional(&mut *tx)
            .await?
        {
            if purged_hash != $request_hash {
                return Ok(DraftResult::Invalid(
                    "Operation id was already used for different content".to_owned(),
                ));
            }
            return Ok(DraftResult::Purged {
                id: purged_id,
                revision: purged_rev,
            });
        }
        let previous: Option<(String, String)> = sqlx::query_as(
            "SELECT request_hash, document_id FROM draft_document_operations \
             WHERE owner_user_id = $1 AND operation_id = $2",
        )
        .bind($args.owner)
        .bind(&$args.mutation.operation_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some((hash, id)) = previous {
            if hash != $request_hash {
                return Ok(DraftResult::Invalid(
                    "Operation id was already used for different content".to_owned(),
                ));
            }
            if let Some((raw, mrev, ts, src)) =
                sqlx::query_as::<_, (String, Option<i64>, Option<i64>, Option<String>)>(
                    "SELECT d.value, m.revision, m.trashed_at_ms, m.source_session_id \
                     FROM draft_documents d \
                     LEFT JOIN draft_document_trash_meta m \
                       ON m.owner_user_id = d.owner_user_id AND m.document_id = d.id \
                     WHERE d.owner_user_id = $1 AND d.id = $2",
                )
                .bind($args.owner)
                .bind(&id)
                .fetch_optional(&mut *tx)
                .await?
            {
                let mut doc: DraftDocument = serde_json::from_str(&raw)?;
                if doc.deleted && mrev == Some(doc.revision) {
                    doc.trashed_at_ms = ts;
                    doc.source_session_id = src;
                }
                update_session_pending_tx!(
                    tx,
                    $args.session_id,
                    $args.next_queue,
                    $args.next_drafts,
                    $now,
                    $dialect
                );
                tx.commit().await?;
                return Ok(DraftResult::Applied(doc));
            }
            bail!("draft archive operation lost its document");
        }
        if matches!($args.mutation.change, DraftChange::Create { .. })
            && let Some(tomb_rev) = sqlx::query_scalar::<_, i64>(
                "SELECT revision FROM draft_document_tombstones \
                 WHERE owner_user_id = $1 AND document_id = $2",
            )
            .bind($args.owner)
            .bind(&$args.mutation.document_id)
            .fetch_optional(&mut *tx)
            .await?
        {
            return Ok(DraftResult::Purged {
                id: $args.mutation.document_id.clone(),
                revision: tomb_rev,
            });
        }
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT metadata FROM draft_documents WHERE owner_user_id = $1")
                .bind($args.owner)
                .fetch_all(&mut *tx)
                .await?;
        let entries: Vec<DraftDocument> = rows
            .into_iter()
            .map(|(v,)| Ok::<_, anyhow::Error>(serde_json::from_str(&v)?))
            .collect::<Result<_>>()?;
        let result = apply(&entries, $args.mutation, $now, $args.shared_folders);
        if let DraftResult::Applied(ref next) = result {
            let stored_next = legacy_stored(next);
            let mut metadata = serde_json::to_value(&stored_next)?;
            metadata.as_object_mut().expect("object").remove("body");
            metadata.as_object_mut().expect("object").remove("attachments");
            sqlx::query(
                "INSERT INTO draft_documents (owner_user_id, id, value, metadata) \
                 VALUES ($1, $2, $3, $4) \
                 ON CONFLICT (owner_user_id, id) DO UPDATE SET \
                   value = excluded.value, metadata = excluded.metadata",
            )
            .bind($args.owner)
            .bind(&next.id)
            .bind(serde_json::to_string(&stored_next)?)
            .bind(serde_json::to_string(&metadata)?)
            .execute(&mut *tx)
            .await?;
            let _ = sync_trash_meta_tx!(
                tx,
                $args.owner,
                next,
                $args.mutation.change,
                $now,
                $args.opts,
                $insert_folder_sql
            );
            sqlx::query(
                "INSERT INTO draft_document_operations \
                 (owner_user_id, operation_id, request_hash, document_id) \
                 VALUES ($1, $2, $3, $4)",
            )
            .bind($args.owner)
            .bind(&$args.mutation.operation_id)
            .bind(&$request_hash)
            .bind(&next.id)
            .execute(&mut *tx)
            .await?;
            update_session_pending_tx!(
                tx,
                $args.session_id,
                $args.next_queue,
                $args.next_drafts,
                $now,
                $dialect
            );
            tx.commit().await?;
        }
        result
    }};
}

macro_rules! atomic_restore_tx {
    ($db:expr, $args:expr, $now:expr, $dialect:ident) => {{
        let mut tx = $db.pool.begin().await?;
        sqlx::query(
            "INSERT INTO draft_document_owners (owner_user_id, revision) VALUES ($1, 1) \
             ON CONFLICT (owner_user_id) DO UPDATE SET revision = draft_document_owners.revision + 1",
        )
        .bind($args.owner)
        .execute(&mut *tx)
        .await?;
        let tomb = sqlx::query_as::<
            _,
            (
                Option<String>,
                Option<String>,
                Option<String>,
                Option<i64>,
                Option<i64>,
            ),
        >(
            "SELECT restore_op_id, restored_row_id, restore_session_id, \
                    restore_expected_revision, restore_expected_trashed_at_ms \
             FROM draft_document_tombstones WHERE owner_user_id = $1 AND document_id = $2",
        )
        .bind($args.owner)
        .bind($args.draft_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some((op_id, row_id, sid, exp_rev, exp_ts)) = tomb {
            if op_id.as_deref() == Some($args.operation_id)
                && sid.as_deref() == Some($args.session_id)
                && exp_rev == Some($args.expected_revision)
                && exp_ts == Some($args.expected_trashed_at_ms)
            {
                return Ok(AtomicRestoreOutcome::AlreadyRestored {
                    restored_row_id: row_id,
                });
            }
            return Ok(AtomicRestoreOutcome::Purged);
        }
        let Some((raw, mrev, ts, src)) =
            sqlx::query_as::<_, (String, Option<i64>, Option<i64>, Option<String>)>(
                "SELECT d.value, m.revision, m.trashed_at_ms, m.source_session_id \
                 FROM draft_documents d \
                 LEFT JOIN draft_document_trash_meta m \
                   ON m.owner_user_id = d.owner_user_id AND m.document_id = d.id \
                 WHERE d.owner_user_id = $1 AND d.id = $2",
            )
            .bind($args.owner)
            .bind($args.draft_id)
            .fetch_optional(&mut *tx)
            .await?
        else {
            return Ok(AtomicRestoreOutcome::NotFound);
        };
        let mut doc: DraftDocument = serde_json::from_str(&raw)?;
        if doc.deleted && mrev == Some(doc.revision) {
            doc.trashed_at_ms = ts;
            doc.source_session_id = src;
        } else {
            doc.trashed_at_ms = None;
            doc.source_session_id = None;
        }
        if !doc.deleted
            || doc.revision != $args.expected_revision
            || doc.trashed_at_ms != Some($args.expected_trashed_at_ms)
            || doc.source_session_id.as_deref() != Some($args.session_id)
        {
            return Ok(AtomicRestoreOutcome::Conflict(
                "Draft changed or was already restored elsewhere; refresh and try again".to_owned(),
            ));
        }
        let purged_revision = doc.revision + 1;
        let receipt = RestoreReceiptRef {
            op_id: $args.operation_id,
            restored_row_id: $args.restored_row_id,
            session_id: $args.session_id,
            expected_revision: $args.expected_revision,
            expected_trashed_at_ms: $args.expected_trashed_at_ms,
        };
        purge_doomed_draft_tx!(tx, $args.owner, $args.draft_id, purged_revision, $now, Some(receipt));
        update_session_pending_tx!(
            tx,
            $args.session_id,
            $args.next_queue,
            $args.next_drafts,
            $now,
            $dialect
        );
        tx.commit().await?;
        AtomicRestoreOutcome::Committed { purged_revision }
    }};
}

impl Store {
    /// Atomically archive a session queue draft to Trash AND persist the
    /// session's updated `queue`/`drafts` inside one SQL transaction (R3-4).
    pub async fn atomic_archive_session_draft(
        &self,
        args: AtomicArchiveDraftArgs<'_>,
    ) -> Result<DraftResult> {
        let now = chrono::Utc::now().timestamp_millis();
        let request_hash = match args.opts.request_hash_override {
            Some(h) => h.to_owned(),
            None => crate::digest::hex_sha256(&serde_json::to_vec(args.mutation)?),
        };
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => atomic_archive_tx!(
                db,
                args,
                now,
                request_hash,
                "INSERT INTO session_folders (id, owner_user_id, name, parent_id, position, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, to_timestamp($6::bigint::double precision / 1000), to_timestamp($6::bigint::double precision / 1000)) ON CONFLICT (id) DO NOTHING",
                postgres
            ),
            StorageBackend::Sqlite(db) => atomic_archive_tx!(
                db,
                args,
                now,
                request_hash,
                "INSERT INTO session_folders (id, owner_user_id, name, parent_id, position, created_at_ms, updated_at_ms) VALUES ($1, $2, $3, $4, $5, $6, $6) ON CONFLICT (id) DO NOTHING",
                sqlite
            ),
        })
    }

    /// Atomically insert a restored draft into `sessions.drafts`, purge its
    /// Trash archive, and persist the exact restore receipt on its tombstone
    /// inside one SQL transaction (R3-4, R3-17).
    pub async fn atomic_restore_session_draft(
        &self,
        args: AtomicRestoreDraftArgs<'_>,
    ) -> Result<AtomicRestoreOutcome> {
        let now = chrono::Utc::now().timestamp_millis();
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => atomic_restore_tx!(db, args, now, postgres),
            StorageBackend::Sqlite(db) => atomic_restore_tx!(db, args, now, sqlite),
        })
    }

    /// Atomically commit folder-only Trash across `session_folders` (all
    /// affected owners), `sessions.folder_id` pins, `draft_documents`
    /// reparenting, and a server-owned undo receipt in ONE SQL tx (R3-6, R3-7, R3-21).
    #[allow(clippy::too_many_arguments)]
    pub async fn atomic_folder_trash_only(
        &self,
        receipt_id: &str,
        folder_id: &str,
        owner_scope: Option<&str>,
        trashed_at_ms: i64,
        destination: Option<&str>,
        previous_project: Option<&str>,
        moved_folders: &[String],
        moved_sessions: &[MovedSessionReceipt],
        folders_by_owner: &[(FolderOwner, Vec<SessionFolder>)],
        pins: &[(String, Option<String>)],
    ) -> Result<AtomicFolderTrashOutcome> {
        let now = chrono::Utc::now().timestamp_millis();
        macro_rules! run_tx {
            ($db:expr, $dialect:ident) => {{
                let mut tx = $db.pool.begin().await?;
                let moved_draft_docs =
                    reparent_folder_drafts_tx!(tx, owner_scope, folder_id, destination, now);
                upsert_folders_by_owner_tx!(tx, folders_by_owner, now, $dialect);
                update_pins_tx!(tx, pins, now, $dialect);
                let receipt = FolderTrashOnlyReceiptPayload {
                    receipt_id: receipt_id.to_owned(),
                    folder_id: folder_id.to_owned(),
                    owner: owner_scope.map(str::to_owned),
                    trashed_at_ms,
                    destination: destination.map(str::to_owned),
                    previous_project: previous_project.map(str::to_owned),
                    moved_folders: moved_folders.to_vec(),
                    moved_sessions: moved_sessions.to_vec(),
                    moved_drafts: moved_draft_docs
                        .iter()
                        .map(|(doc_owner, doc)| MovedDraftReceipt {
                            owner: doc_owner.clone(),
                            id: doc.id.clone(),
                            revision: doc.revision,
                            metadata_revision: doc.metadata_revision,
                        })
                        .collect(),
                };
                sqlx::query(
                    "INSERT INTO folder_trash_only_receipts \
                     (receipt_id, owner_user_id, folder_id, trashed_at_ms, payload_json, undone_at_ms) \
                     VALUES ($1, $2, $3, $4, $5, NULL)",
                )
                .bind(&receipt.receipt_id)
                .bind(receipt.owner.as_deref())
                .bind(&receipt.folder_id)
                .bind(receipt.trashed_at_ms)
                .bind(serde_json::to_string(&receipt)?)
                .execute(&mut *tx)
                .await?;
                tx.commit().await?;
                AtomicFolderTrashOutcome {
                    receipt,
                    moved_draft_docs,
                }
            }};
        }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => run_tx!(db, postgres),
            StorageBackend::Sqlite(db) => run_tx!(db, sqlite),
        })
    }

    /// Read one unconsumed server-owned folder-only Trash receipt.
    pub async fn load_folder_trash_only_receipt(
        &self,
        receipt_id: &str,
    ) -> Result<Option<(FolderTrashOnlyReceiptPayload, Option<i64>)>> {
        macro_rules! read {
            ($db:expr) => {{
                sqlx::query_as::<_, (String, Option<i64>)>(
                    "SELECT payload_json, undone_at_ms FROM folder_trash_only_receipts \
                     WHERE receipt_id = $1",
                )
                .bind(receipt_id)
                .fetch_optional(&$db.pool)
                .await?
                .map(|(json, undone)| {
                    Ok::<_, anyhow::Error>((serde_json::from_str(&json)?, undone))
                })
                .transpose()?
            }};
        }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => read!(db),
            StorageBackend::Sqlite(db) => read!(db),
        })
    }

    /// Atomically undo a server-owned folder-only Trash receipt across
    /// `session_folders`, `sessions.folder_id`, `draft_documents`, and the
    /// receipt's `undone_at_ms` marker in ONE SQL transaction (R3-6, R3-7).
    pub async fn atomic_undo_folder_trash_only(
        &self,
        receipt: &FolderTrashOnlyReceiptPayload,
        folders_by_owner: &[(FolderOwner, Vec<SessionFolder>)],
        pins: &[(String, Option<String>)],
    ) -> Result<std::result::Result<Vec<(String, DraftDocument)>, String>> {
        let now = chrono::Utc::now().timestamp_millis();
        macro_rules! run_tx {
            ($db:expr, $dialect:ident) => {{
                let mut tx = $db.pool.begin().await?;
                let marked = sqlx::query(
                    "UPDATE folder_trash_only_receipts SET undone_at_ms = $2 \
                     WHERE receipt_id = $1 AND folder_id = $3 AND undone_at_ms IS NULL",
                )
                .bind(&receipt.receipt_id)
                .bind(now)
                .bind(&receipt.folder_id)
                .execute(&mut *tx)
                .await?;
                if marked.rows_affected() == 0 {
                    return Ok(Err("This folder Trash action was already undone".into()));
                }
                upsert_folders_by_owner_tx!(tx, folders_by_owner, now, $dialect);
                update_pins_tx!(tx, pins, now, $dialect);
                let restored_drafts = undo_reparent_folder_drafts_tx!(
                    tx,
                    &receipt.folder_id,
                    receipt.destination.as_deref(),
                    &receipt.moved_drafts,
                    now
                );
                tx.commit().await?;
                Ok(Ok(restored_drafts))
            }};
        }
        match &self.backend {
            StorageBackend::Postgres(db) => run_tx!(db, postgres),
            StorageBackend::Sqlite(db) => run_tx!(db, sqlite),
        }
    }
}
