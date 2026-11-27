//! Independent documents. A durable operation receipt and a document revision
//! commit together. Separate content/metadata clocks let moving a document
//! coexist with writing it, while two writers never silently overwrite.

use super::{StorageBackend, Store};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

pub mod atomic_transfers;
mod trash;
pub use atomic_transfers::*;
pub use trash::{
    DraftArchiveOptions, DraftMutationOutcome, DraftTombstone, DraftTrashRestoreRecord,
    PurgeDraftError, PurgeDraftTarget, RestoreReceiptRef,
};

const MAX_BODY: usize = 2 * 1024 * 1024;
const MAX_ENTRIES: usize = 10_000;
/// Recovery snapshots per document.
const HISTORY_LIMIT: i64 = 30;
/// Snapshot spacing, as Obsidian's File recovery: autosave writes every few
/// hundred milliseconds of typing, so a snapshot per write would leave only
/// the last minute in the history.
const HISTORY_INTERVAL_MS: i64 = 5 * 60 * 1000;

/// Whether the content a write replaces becomes a recovery snapshot: the
/// first one, one that rested unchanged for an interval (the text before an
/// editing session resumed), or one an interval after the newest snapshot
/// (periodic snapshots while writing continuously).
fn keeps_snapshot(now: i64, replaced_at: i64, newest_snapshot_at: Option<i64>) -> bool {
    newest_snapshot_at.is_none_or(|newest| {
        now - replaced_at >= HISTORY_INTERVAL_MS || replaced_at - newest >= HISTORY_INTERVAL_MS
    })
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DraftDocument {
    pub id: String,
    pub kind: DraftKind,
    pub title: String,
    pub parent_id: Option<String>,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub attachments: Vec<serde_json::Value>,
    pub revision: i64,
    pub body_revision: i64,
    pub metadata_revision: i64,
    pub updated_at_ms: i64,
    pub deleted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trashed_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_session_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DraftKind {
    Document,
    Folder,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DraftChange {
    Create {
        kind: DraftKind,
        title: String,
        parent_id: Option<String>,
        body: String,
        #[serde(default)]
        attachments: Vec<serde_json::Value>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        trashed: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source_session_id: Option<String>,
    },
    Write {
        body: String,
        #[serde(default)]
        attachments: Vec<serde_json::Value>,
    },
    /// A text edit that retains attachments, including on idempotent replay.
    WriteText {
        body: String,
    },
    Rename {
        title: String,
    },
    Move {
        parent_id: Option<String>,
    },
    Trash,
    Restore,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DraftMutation {
    pub operation_id: String,
    pub document_id: String,
    pub expected_revision: i64,
    pub change: DraftChange,
}

/// One recovery snapshot without its content: what the History list shows
/// before the user opens a version.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct DraftSnapshotSummary {
    pub revision: i64,
    pub body_revision: i64,
    pub updated_at_ms: i64,
    pub characters: usize,
    pub attachments: usize,
    /// The opening of the text, enough to recognize a version.
    pub preview: String,
}

const SNAPSHOT_PREVIEW_CHARS: usize = 160;

impl From<&DraftDocument> for DraftSnapshotSummary {
    fn from(document: &DraftDocument) -> Self {
        Self {
            revision: document.revision,
            body_revision: document.body_revision,
            updated_at_ms: document.updated_at_ms,
            characters: document.body.chars().count(),
            attachments: document.attachments.len(),
            preview: document.body.chars().take(SNAPSHOT_PREVIEW_CHARS).collect(),
        }
    }
}

#[derive(Debug)]
pub enum DraftResult {
    Applied(DraftDocument),
    Conflict(Option<DraftDocument>),
    Invalid(String),
    Purged { id: String, revision: i64 },
}

/// Strip fields added after the base Controller so `draft_documents.value`,
/// `draft_documents.metadata`, and `draft_document_history.value` stay 100%
/// readable if the Controller rolls back.
pub(super) fn legacy_stored(doc: &DraftDocument) -> DraftDocument {
    let mut stored = doc.clone();
    stored.trashed_at_ms = None;
    stored.source_session_id = None;
    stored
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn normalized_title(title: &str) -> Option<String> {
    let title = title.trim();
    (!title.is_empty() && title.chars().count() <= 160 && !title.chars().any(char::is_control))
        .then(|| title.to_owned())
}

fn apply_create(
    next: &mut DraftDocument,
    entries: &[DraftDocument],
    shared: &[crate::session_folders::SessionFolder],
    now: i64,
    change: &DraftChange,
) -> Result<(), String> {
    let DraftChange::Create {
        kind,
        title,
        parent_id,
        body,
        attachments,
        trashed,
        source_session_id,
    } = change
    else {
        return Ok(());
    };
    if entries.len() >= MAX_ENTRIES {
        return Err("Draft library is full".to_owned());
    }
    if source_session_id.as_deref().is_some_and(|id| !valid_id(id)) {
        return Err("Invalid source session".to_owned());
    }
    next.kind = *kind;
    next.title.clone_from(title);
    next.parent_id = if *trashed {
        nearest_live_parent(parent_id.as_deref(), entries, shared)
    } else {
        parent_id.clone()
    };
    next.body.clone_from(body);
    next.attachments.clone_from(attachments);
    next.deleted = *trashed;
    next.trashed_at_ms = trashed.then_some(now);
    next.source_session_id.clone_from(source_session_id);
    next.body_revision = 1;
    next.metadata_revision = 1;
    Ok(())
}

fn apply(
    entries: &[DraftDocument],
    mutation: &DraftMutation,
    now: i64,
    shared: &[crate::session_folders::SessionFolder],
) -> DraftResult {
    use DraftChange::{Create, Move, Rename, Restore, Trash, Write, WriteText};
    if !valid_id(&mutation.document_id)
        || !valid_id(&mutation.operation_id)
        || mutation.expected_revision < 0
    {
        return DraftResult::Invalid("Invalid document operation".to_owned());
    }
    let current = entries.iter().find(|d| d.id == mutation.document_id);
    let expected = match (&mutation.change, current) {
        (Create { .. }, _) => 0,
        (Write { .. } | WriteText { .. }, Some(d)) => d.body_revision,
        (Move { .. } | Rename { .. }, Some(d)) => d.metadata_revision,
        (_, Some(d)) => d.revision,
        (_, None) => return DraftResult::Conflict(None),
    };
    if mutation.expected_revision != expected
        || matches!(mutation.change, Create { .. }) && current.is_some()
    {
        return DraftResult::Conflict(current.cloned());
    }
    if current.is_some_and(|d| d.deleted) && !matches!(mutation.change, Restore) {
        return DraftResult::Conflict(current.cloned());
    }
    let mut next = current.cloned().unwrap_or_else(|| DraftDocument {
        id: mutation.document_id.clone(),
        kind: DraftKind::Document,
        title: String::new(),
        parent_id: None,
        body: String::new(),
        attachments: vec![],
        revision: 0,
        body_revision: 0,
        metadata_revision: 0,
        updated_at_ms: now,
        deleted: false,
        trashed_at_ms: None,
        source_session_id: None,
    });
    match &mutation.change {
        Create { .. } => {
            if let Err(err) = apply_create(&mut next, entries, shared, now, &mutation.change) {
                return DraftResult::Invalid(err);
            }
        }
        Write { body, .. } | WriteText { body } => {
            if next.kind != DraftKind::Document {
                return DraftResult::Invalid("Folders have no text".to_owned());
            }
            next.body.clone_from(body);
            if let Write { attachments, .. } = &mutation.change {
                next.attachments.clone_from(attachments);
            }
            next.body_revision += 1;
        }
        Rename { title } => {
            next.title.clone_from(title);
            next.metadata_revision += 1;
        }
        Move { parent_id } => {
            next.parent_id.clone_from(parent_id);
            next.metadata_revision += 1;
        }
        Trash => {
            if entries
                .iter()
                .any(|d| !d.deleted && d.parent_id.as_deref() == Some(&next.id))
            {
                return DraftResult::Invalid(
                    "Move the folder's contents before deleting it".to_owned(),
                );
            }
            next.deleted = true;
            next.trashed_at_ms = Some(now);
        }
        Restore => {
            next.deleted = false;
            next.trashed_at_ms = None;
            next.source_session_id = None;
            let resolved = nearest_live_parent(next.parent_id.as_deref(), entries, shared);
            if resolved != next.parent_id {
                next.parent_id = resolved;
                next.metadata_revision += 1;
            }
        }
    }
    validate_next(entries, next, now, shared, &mutation.change)
}

/// Nothing new is filed into Trash (a trashed folder's whole subtree is
/// trashed); a Draft already there may still be edited or moved out.
/// Nearest live ancestor folder (walking past trashed or missing folders).
fn nearest_live_parent(
    parent: Option<&str>,
    entries: &[DraftDocument],
    shared: &[crate::session_folders::SessionFolder],
) -> Option<String> {
    let mut cursor = parent;
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = cursor {
        if !seen.insert(id) {
            return None;
        }
        if let Some(folder) = shared.iter().find(|f| f.id == id) {
            if folder.trashed_at_ms.is_none() {
                return Some(folder.id.clone());
            }
            cursor = folder.parent.as_deref();
            continue;
        }
        if let Some(folder) = entries
            .iter()
            .find(|d| d.id == id && d.kind == DraftKind::Folder)
        {
            if !folder.deleted {
                return Some(folder.id.clone());
            }
            cursor = folder.parent_id.as_deref();
            continue;
        }
        return None;
    }
    None
}

fn files_into_trash(
    change: &DraftChange,
    next: &DraftDocument,
    shared: &[crate::session_folders::SessionFolder],
) -> bool {
    if matches!(change, DraftChange::Create { trashed: true, .. }) {
        return false;
    }
    matches!(
        change,
        DraftChange::Create { .. } | DraftChange::Move { .. }
    ) && next.parent_id.as_deref().is_some_and(|parent| {
        shared
            .iter()
            .any(|folder| folder.id == parent && folder.trashed_at_ms.is_some())
    })
}

fn validate_next(
    entries: &[DraftDocument],
    mut next: DraftDocument,
    now: i64,
    shared: &[crate::session_folders::SessionFolder],
    change: &DraftChange,
) -> DraftResult {
    if files_into_trash(change, &next, shared) {
        return DraftResult::Invalid("The destination folder is in Trash".to_owned());
    }
    let Some(title) = normalized_title(&next.title) else {
        return DraftResult::Invalid("Use a title of 1–160 characters".to_owned());
    };
    next.title = title;
    if next.body.len() > MAX_BODY
        || next.body.contains('\0')
        || next.kind == DraftKind::Folder && !next.body.is_empty()
    {
        return DraftResult::Invalid("Document text is invalid or exceeds 2 MiB".to_owned());
    }
    if next.attachments.len() > 100
        || serde_json::to_vec(&next.attachments).map_or(true, |v| v.len() > 8 * 1024 * 1024)
        || next.attachments.iter().any(|a| {
            a.get("id")
                .and_then(serde_json::Value::as_str)
                .is_none_or(|id| !valid_id(id))
                || a.get("pending").and_then(serde_json::Value::as_bool) == Some(true)
        })
    {
        return DraftResult::Invalid(
            "Attachments are invalid, unfinished or exceed 8 MiB".to_owned(),
        );
    }
    let mut parent = next.parent_id.as_deref();
    let mut ancestors = std::collections::HashSet::new();
    while let Some(id) = parent {
        if id == next.id || !ancestors.insert(id) {
            return DraftResult::Invalid("A folder cannot be moved inside itself".to_owned());
        }
        if let Some(folder) = shared.iter().find(|folder| folder.id == id) {
            parent = folder.parent.as_deref();
            continue;
        }
        let Some(folder) = entries
            .iter()
            .find(|d| d.id == id && !d.deleted && d.kind == DraftKind::Folder)
        else {
            return DraftResult::Invalid("The destination folder is unavailable".to_owned());
        };
        parent = folder.parent_id.as_deref();
    }
    next.revision += 1;
    next.updated_at_ms = now;
    DraftResult::Applied(next)
}

impl Store {
    /// Bounded metadata-only keyset page. Filtering tombstones in SQL keeps
    /// a page full even when earlier rows were trashed, without loading the
    /// owner's entire document index into the Controller for each MCP call.
    pub async fn draft_documents_page(
        &self,
        owner: &str,
        after: Option<&str>,
        limit: i64,
        include_deleted: bool,
    ) -> Result<Vec<DraftDocument>> {
        anyhow::ensure!((1..=200).contains(&limit), "invalid document page limit");
        macro_rules! read {
            ($db:expr, $deleted:literal) => {
                sqlx::query_as::<_, (String, Option<i64>, Option<i64>, Option<String>)>(concat!(
                    "SELECT d.metadata, m.revision, m.trashed_at_ms, m.source_session_id \
                     FROM draft_documents d \
                     LEFT JOIN draft_document_trash_meta m \
                       ON m.owner_user_id = d.owner_user_id AND m.document_id = d.id \
                     WHERE d.owner_user_id = $1 AND d.id > $2 AND ($3 OR ",
                    $deleted,
                    ") ORDER BY d.id LIMIT $4"
                ))
                .bind(owner)
                .bind(after.unwrap_or(""))
                .bind(include_deleted)
                .bind(limit)
                .fetch_all(&$db.pool)
                .await?
            };
        }
        let rows = match &self.backend {
            StorageBackend::Postgres(db) => read!(db, "(d.metadata::jsonb->>'deleted') = 'false'"),
            StorageBackend::Sqlite(db) => read!(db, "json_extract(d.metadata, '$.deleted') = 0"),
        };
        let mut docs: Vec<DraftDocument> = Vec::with_capacity(rows.len());
        let mut meta_revs: Vec<Option<i64>> = Vec::with_capacity(rows.len());
        for (raw, meta_rev, trashed_at, source_session) in rows {
            let mut doc: DraftDocument = serde_json::from_str(&raw)?;
            if doc.deleted && meta_rev == Some(doc.revision) {
                doc.trashed_at_ms = trashed_at;
                doc.source_session_id = source_session;
            }
            docs.push(doc);
            meta_revs.push(meta_rev);
        }
        trash::reconcile_tombstone_collisions(self, owner, &mut docs).await?;
        let mut paired: Vec<(DraftDocument, Option<i64>)> =
            docs.into_iter().zip(meta_revs).collect();
        trash::sync_revision_bound_trash_meta(self, owner, &mut paired).await?;
        Ok(paired.into_iter().map(|(doc, _)| doc).collect())
    }

    pub async fn draft_documents(&self, owner: &str) -> Result<Vec<DraftDocument>> {
        macro_rules! read {
            ($db:expr) => {
                sqlx::query_as::<_, (String, Option<i64>, Option<i64>, Option<String>)>(
                    "SELECT d.metadata, m.revision, m.trashed_at_ms, m.source_session_id \
                     FROM draft_documents d \
                     LEFT JOIN draft_document_trash_meta m \
                       ON m.owner_user_id = d.owner_user_id AND m.document_id = d.id \
                     WHERE d.owner_user_id = $1 ORDER BY d.id",
                )
                .bind(owner)
                .fetch_all(&$db.pool)
                .await?
            };
        }
        let rows = match &self.backend {
            StorageBackend::Postgres(db) => read!(db),
            StorageBackend::Sqlite(db) => read!(db),
        };
        let mut docs: Vec<DraftDocument> = Vec::with_capacity(rows.len());
        let mut meta_revs: Vec<Option<i64>> = Vec::with_capacity(rows.len());
        for (raw, meta_rev, trashed_at, source_session) in rows {
            let mut doc: DraftDocument = serde_json::from_str(&raw)?;
            if doc.deleted && meta_rev == Some(doc.revision) {
                doc.trashed_at_ms = trashed_at;
                doc.source_session_id = source_session;
            }
            docs.push(doc);
            meta_revs.push(meta_rev);
        }
        trash::reconcile_tombstone_collisions(self, owner, &mut docs).await?;
        let mut paired: Vec<(DraftDocument, Option<i64>)> =
            docs.into_iter().zip(meta_revs).collect();
        trash::sync_revision_bound_trash_meta(self, owner, &mut paired).await?;
        Ok(paired.into_iter().map(|(doc, _)| doc).collect())
    }

    pub async fn draft_document(&self, owner: &str, id: &str) -> Result<Option<DraftDocument>> {
        macro_rules! read {
            ($db:expr) => {
                sqlx::query_as::<_, (String, Option<i64>, Option<i64>, Option<String>)>(
                    "SELECT d.value, m.revision, m.trashed_at_ms, m.source_session_id \
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
        let Some((raw, meta_rev, trashed_at, source_session)) = row else {
            return Ok(None);
        };
        let mut doc: DraftDocument = serde_json::from_str(&raw)?;
        if doc.deleted && meta_rev == Some(doc.revision) {
            doc.trashed_at_ms = trashed_at;
            doc.source_session_id = source_session;
        }
        trash::reconcile_tombstone_collisions(self, owner, std::slice::from_mut(&mut doc)).await?;
        let mut paired = [(doc, meta_rev)];
        trash::sync_revision_bound_trash_meta(self, owner, &mut paired).await?;
        Ok(Some(paired.into_iter().next().unwrap().0))
    }

    /// The History list: every kept snapshot, newest first, without content.
    pub async fn draft_history_index(
        &self,
        owner: &str,
        id: &str,
    ) -> Result<Vec<DraftSnapshotSummary>> {
        Ok(self
            .draft_history(owner, id)
            .await?
            .iter()
            .map(DraftSnapshotSummary::from)
            .collect())
    }

    /// One kept snapshot with its content.
    pub async fn draft_history_snapshot(
        &self,
        owner: &str,
        id: &str,
        revision: i64,
    ) -> Result<Option<DraftDocument>> {
        macro_rules! read { ($db:expr) => {
            sqlx::query_scalar::<_, String>("SELECT value FROM draft_document_history WHERE owner_user_id = $1 AND document_id = $2 AND revision = $3")
                .bind(owner).bind(id).bind(revision).fetch_optional(&$db.pool).await?
        }; }
        let value = match &self.backend {
            StorageBackend::Postgres(db) => read!(db),
            StorageBackend::Sqlite(db) => read!(db),
        };
        value
            .map(|v| serde_json::from_str(&v).map_err(Into::into))
            .transpose()
    }

    pub async fn draft_history(&self, owner: &str, id: &str) -> Result<Vec<DraftDocument>> {
        macro_rules! read { ($db:expr) => {
            sqlx::query_scalar::<_, String>("SELECT value FROM draft_document_history WHERE owner_user_id = $1 AND document_id = $2 ORDER BY revision DESC LIMIT $3")
                .bind(owner).bind(id).bind(HISTORY_LIMIT).fetch_all(&$db.pool).await?
        }; }
        let values = match &self.backend {
            StorageBackend::Postgres(db) => read!(db),
            StorageBackend::Sqlite(db) => read!(db),
        };
        values
            .iter()
            .map(|v| serde_json::from_str(v).map_err(Into::into))
            .collect()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub async fn mutate_draft_document(
        &self,
        owner: &str,
        mutation: &DraftMutation,
    ) -> Result<DraftResult> {
        let shared = self
            .load_session_folders()
            .await?
            .into_iter()
            .filter(|folder| folder.owner_user_id.as_deref().is_none_or(|id| id == owner))
            .collect::<Vec<_>>();
        self.mutate_draft_document_with_options(
            owner,
            mutation,
            &shared,
            &DraftArchiveOptions {
                allow_session_provenance: true,
                ..Default::default()
            },
        )
        .await
        .map(|o| o.result)
    }

    /// Receipts survive a lost response and do not depend on the code sandbox.
    /// The row commits in the same transaction as its document mutation.
    pub async fn draft_operation_document(
        &self,
        owner: &str,
        operation: &str,
    ) -> Result<Option<String>> {
        macro_rules! read { ($db:expr) => {
            sqlx::query_scalar::<_,String>("SELECT document_id FROM draft_document_operations WHERE owner_user_id = $1 AND operation_id = $2")
                .bind(owner).bind(operation).fetch_optional(&$db.pool).await
        }; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => read!(db)?,
            StorageBackend::Sqlite(db) => read!(db)?,
        })
    }

    pub async fn mutate_draft_document_in_workspace(
        &self,
        owner: &str,
        mutation: &DraftMutation,
        shared: &[crate::session_folders::SessionFolder],
    ) -> Result<DraftResult> {
        Ok(self
            .mutate_draft_document_with_options(
                owner,
                mutation,
                shared,
                &DraftArchiveOptions::default(),
            )
            .await?
            .result)
    }

    #[allow(clippy::too_many_lines)]
    pub async fn mutate_draft_document_with_options(
        &self,
        owner: &str,
        mutation: &DraftMutation,
        shared: &[crate::session_folders::SessionFolder],
        opts: &DraftArchiveOptions<'_>,
    ) -> Result<DraftMutationOutcome> {
        if !opts.allow_session_provenance
            && matches!(
                mutation.change,
                DraftChange::Create {
                    source_session_id: Some(_),
                    ..
                }
            )
        {
            return Ok(DraftMutationOutcome {
                result: DraftResult::Invalid(
                    "source_session_id is reserved for archived session drafts".to_owned(),
                ),
                imported_folder: None,
            });
        }
        let request_hash = match opts.request_hash_override {
            Some(override_hash) => override_hash.to_owned(),
            None => crate::digest::hex_sha256(&serde_json::to_vec(mutation)?),
        };
        macro_rules! write { ($db:expr, $newest_snapshot_at:expr, $insert_folder_sql:literal) => {{
            let mut tx = $db.pool.begin().await?;
            sqlx::query("INSERT INTO draft_document_owners (owner_user_id, revision) VALUES ($1, 1) ON CONFLICT (owner_user_id) DO UPDATE SET revision = draft_document_owners.revision + 1")
                .bind(owner).execute(&mut *tx).await?;
            if let Some((purged_hash, purged_id, purged_rev)) = sqlx::query_as::<_, (String, String, i64)>(
                "SELECT request_hash, document_id, purged_revision FROM draft_document_purged_ops WHERE owner_user_id = $1 AND operation_id = $2",
            )
            .bind(owner).bind(&mutation.operation_id).fetch_optional(&mut *tx).await?
            {
                if purged_hash != request_hash {
                    return Ok(DraftMutationOutcome { result: DraftResult::Invalid("Operation id was already used for different content".to_owned()), imported_folder: None });
                }
                return Ok(DraftMutationOutcome { result: DraftResult::Purged { id: purged_id, revision: purged_rev }, imported_folder: None });
            }
            let previous: Option<(String, String)> = sqlx::query_as("SELECT request_hash, document_id FROM draft_document_operations WHERE owner_user_id = $1 AND operation_id = $2")
                .bind(owner).bind(&mutation.operation_id).fetch_optional(&mut *tx).await?;
            let rows: Vec<(String, Option<i64>, Option<i64>, Option<String>)> = sqlx::query_as(
                "SELECT d.metadata, m.revision, m.trashed_at_ms, m.source_session_id FROM draft_documents d LEFT JOIN draft_document_trash_meta m ON m.owner_user_id = d.owner_user_id AND m.document_id = d.id WHERE d.owner_user_id = $1",
            ).bind(owner).fetch_all(&mut *tx).await?;
            let mut entries: Vec<DraftDocument> = rows.into_iter().map(|(v, mrev, ts, src)| {
                let mut doc: DraftDocument = serde_json::from_str(&v)?;
                if doc.deleted && mrev == Some(doc.revision) { doc.trashed_at_ms = ts; doc.source_session_id = src; }
                Ok::<_, anyhow::Error>(doc)
            }).collect::<Result<_>>()?;
            let target: Option<(String, Option<i64>, Option<i64>, Option<String>, Option<String>)> = sqlx::query_as(
                "SELECT d.value, m.revision, m.trashed_at_ms, m.source_session_id, m.raw_content_json FROM draft_documents d LEFT JOIN draft_document_trash_meta m ON m.owner_user_id = d.owner_user_id AND m.document_id = d.id WHERE d.owner_user_id = $1 AND d.id = $2",
            ).bind(owner).bind(&mutation.document_id).fetch_optional(&mut *tx).await?;
            let mut target_raw_content: Option<String> = None;
            if let Some((raw, mrev, ts, src, raw_blocks)) = target {
                let mut doc: DraftDocument = serde_json::from_str(&raw)?;
                let pre_rev = doc.revision;
                if let Some(healed) = trash::reconcile_doc_tombstone_tx!(tx, owner, &mutation.document_id) {
                    doc.revision = doc.revision.max(healed.revision);
                    doc.body_revision = doc.body_revision.max(healed.body_revision);
                    doc.metadata_revision = doc.metadata_revision.max(healed.metadata_revision);
                }
                if doc.deleted && mrev == Some(pre_rev) {
                    doc.trashed_at_ms = ts; doc.source_session_id = src; target_raw_content = raw_blocks;
                }
                if let Some(entry) = entries.iter_mut().find(|d| d.id == doc.id) { *entry = doc; }
            }
            if let Some((hash, id)) = previous {
                if hash != request_hash { return Ok(DraftMutationOutcome { result: DraftResult::Invalid("Operation id was already used for different content".to_owned()), imported_folder: None }); }
                if let Some(document) = entries.iter().find(|d| d.id == id) {
                    return Ok(DraftMutationOutcome { result: DraftResult::Applied(document.clone()), imported_folder: None });
                }
                if let Some(tomb_rev) = sqlx::query_scalar::<_, i64>("SELECT revision FROM draft_document_tombstones WHERE owner_user_id = $1 AND document_id = $2")
                    .bind(owner).bind(&id).fetch_optional(&mut *tx).await?
                {
                    return Ok(DraftMutationOutcome { result: DraftResult::Purged { id, revision: tomb_rev }, imported_folder: None });
                }
                bail!("draft operation lost its document");
            }
            if matches!(mutation.change, DraftChange::Create { .. })
                && let Some(tomb_rev) = sqlx::query_scalar::<_, i64>("SELECT revision FROM draft_document_tombstones WHERE owner_user_id = $1 AND document_id = $2")
                    .bind(owner).bind(&mutation.document_id).fetch_optional(&mut *tx).await?
            {
                return Ok(DraftMutationOutcome { result: DraftResult::Purged { id: mutation.document_id.clone(), revision: tomb_rev }, imported_folder: None });
            }
            if matches!(mutation.change, DraftChange::Restore)
                && let Some(curr) = entries.iter().find(|d| {
                    d.id == mutation.document_id
                        && d.deleted
                        && d.revision == mutation.expected_revision
                })
                && trash::has_unrepresented_acp_blocks(target_raw_content.as_deref(), &curr.body, &curr.attachments)
            {
                return Ok(DraftMutationOutcome {
                    result: DraftResult::Invalid("This session draft contains session-only attachments (such as resource links). Use \"Restore to session\" instead.".to_owned()),
                    imported_folder: None,
                });
            }
            let mut mapped = mutation.clone();
            let parent = match &mut mapped.change { DraftChange::Create { parent_id, .. } | DraftChange::Move { parent_id } => Some(parent_id), _ => None };
            if let Some(Some(parent)) = parent {
                let destination: Option<String> = sqlx::query_scalar("SELECT folder_id FROM draft_workspace_folder_imports WHERE owner_user_id=$1 AND document_id=$2").bind(owner).bind(&*parent).fetch_optional(&mut *tx).await?;
                if let Some(destination) = destination { *parent = destination; }
            }
            let (db_folder_map, effective_shared) = trash::resolve_tx_folders!(tx, owner, shared);
            let now = chrono::Utc::now().timestamp_millis();
            let mut result = apply(&entries, &mapped, now, &effective_shared);
            if matches!(mapped.change, DraftChange::Restore) && let DraftResult::Applied(ref mut next) = result {
                next.parent_id = trash::resolve_live_restore_parent(next.parent_id.clone(), &db_folder_map, &effective_shared);
            }
            let mut imported_folder = None;
            if let DraftResult::Applied(ref next) = result {
                if let Some(prev) = entries.iter().find(|d| d.id == next.id && (d.body != next.body || d.attachments != next.attachments) && d.kind == DraftKind::Document) {
                    let newest: Option<i64> = sqlx::query_scalar($newest_snapshot_at).bind(owner).bind(&next.id).fetch_optional(&mut *tx).await?;
                    if keeps_snapshot(now, prev.updated_at_ms, newest) {
                        let stored_prev = legacy_stored(prev);
                        sqlx::query("INSERT INTO draft_document_history (owner_user_id, document_id, revision, value) VALUES ($1, $2, $3, $4)")
                            .bind(owner).bind(&next.id).bind(prev.revision).bind(serde_json::to_string(&stored_prev)?).execute(&mut *tx).await?;
                        sqlx::query("DELETE FROM draft_document_history WHERE owner_user_id = $1 AND document_id = $2 AND revision NOT IN (SELECT revision FROM draft_document_history WHERE owner_user_id = $1 AND document_id = $2 ORDER BY revision DESC LIMIT $3)")
                            .bind(owner).bind(&next.id).bind(HISTORY_LIMIT).execute(&mut *tx).await?;
                    }
                }
                let stored_next = legacy_stored(next);
                let mut metadata = serde_json::to_value(&stored_next)?;
                metadata.as_object_mut().expect("document object").remove("body");
                metadata.as_object_mut().expect("document object").remove("attachments");
                sqlx::query("INSERT INTO draft_documents (owner_user_id, id, value, metadata) VALUES ($1, $2, $3, $4) ON CONFLICT (owner_user_id, id) DO UPDATE SET value = excluded.value, metadata = excluded.metadata")
                    .bind(owner).bind(&next.id).bind(serde_json::to_string(&stored_next)?).bind(serde_json::to_string(&metadata)?).execute(&mut *tx).await?;
                imported_folder = trash::sync_trash_meta_tx!(tx, owner, next, mapped.change, now, opts, $insert_folder_sql);
                sqlx::query("INSERT INTO draft_document_operations (owner_user_id, operation_id, request_hash, document_id) VALUES ($1, $2, $3, $4)")
                    .bind(owner).bind(&mutation.operation_id).bind(&request_hash).bind(&next.id).execute(&mut *tx).await?;
                tx.commit().await?;
            }
            DraftMutationOutcome { result, imported_folder }
        }}; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => write!(
                db,
                "SELECT (value::json->>'updated_at_ms')::bigint FROM draft_document_history WHERE owner_user_id = $1 AND document_id = $2 ORDER BY revision DESC LIMIT 1",
                "INSERT INTO session_folders (id, owner_user_id, name, parent_id, position, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, to_timestamp($6::bigint::double precision / 1000), to_timestamp($6::bigint::double precision / 1000)) ON CONFLICT (id) DO NOTHING"
            ),
            StorageBackend::Sqlite(db) => write!(
                db,
                "SELECT json_extract(value, '$.updated_at_ms') FROM draft_document_history WHERE owner_user_id = $1 AND document_id = $2 ORDER BY revision DESC LIMIT 1",
                "INSERT INTO session_folders (id, owner_user_id, name, parent_id, position, created_at_ms, updated_at_ms) VALUES ($1, $2, $3, $4, $5, $6, $6) ON CONFLICT (id) DO NOTHING"
            ),
        })
    }
}

#[cfg(test)]
mod tests;
