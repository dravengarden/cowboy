//! Independent documents. A durable operation receipt and a document revision
//! commit together. Separate content/metadata clocks let moving a document
//! coexist with writing it, while two writers never silently overwrite.

use super::{StorageBackend, Store};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

const MAX_BODY: usize = 2 * 1024 * 1024;
const MAX_ENTRIES: usize = 10_000;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct DraftDocument {
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
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DraftKind {
    Document,
    Folder,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum DraftChange {
    Create {
        kind: DraftKind,
        title: String,
        parent_id: Option<String>,
        body: String,
        #[serde(default)]
        attachments: Vec<serde_json::Value>,
    },
    Write {
        body: String,
        #[serde(default)]
        attachments: Vec<serde_json::Value>,
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
pub(crate) struct DraftMutation {
    pub operation_id: String,
    pub document_id: String,
    pub expected_revision: i64,
    pub change: DraftChange,
}

#[derive(Debug)]
pub(crate) enum DraftResult {
    Applied(DraftDocument),
    Conflict(Option<DraftDocument>),
    Invalid(String),
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

fn apply(
    entries: &[DraftDocument],
    mutation: &DraftMutation,
    now: i64,
    shared: &[crate::session_folders::SessionFolder],
) -> DraftResult {
    use DraftChange::{Create, Move, Rename, Restore, Trash, Write};
    if !valid_id(&mutation.document_id)
        || !valid_id(&mutation.operation_id)
        || mutation.expected_revision < 0
    {
        return DraftResult::Invalid("Invalid document operation".to_owned());
    }
    let current = entries.iter().find(|d| d.id == mutation.document_id);
    let expected = match (&mutation.change, current) {
        (Create { .. }, _) => 0,
        (Write { .. }, Some(d)) => d.body_revision,
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
    });
    match &mutation.change {
        Create {
            kind,
            title,
            parent_id,
            body,
            attachments,
        } => {
            if entries.len() >= MAX_ENTRIES {
                return DraftResult::Invalid("Draft library is full".to_owned());
            }
            next.kind = *kind;
            next.title.clone_from(title);
            next.parent_id.clone_from(parent_id);
            next.body.clone_from(body);
            next.attachments.clone_from(attachments);
            next.body_revision = 1;
            next.metadata_revision = 1;
        }
        Write { body, attachments } => {
            if next.kind != DraftKind::Document {
                return DraftResult::Invalid("Folders have no text".to_owned());
            }
            next.body.clone_from(body);
            next.attachments.clone_from(attachments);
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
        }
        Restore => {
            next.deleted = false;
            if next.parent_id.as_ref().is_some_and(|id| {
                !entries
                    .iter()
                    .any(|d| d.id == *id && !d.deleted && d.kind == DraftKind::Folder)
                    && !shared.iter().any(|folder| folder.id == *id)
            }) {
                next.parent_id = None;
                next.metadata_revision += 1;
            }
        }
    }
    validate_next(entries, next, now, shared)
}

fn validate_next(
    entries: &[DraftDocument],
    mut next: DraftDocument,
    now: i64,
    shared: &[crate::session_folders::SessionFolder],
) -> DraftResult {
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
    pub(crate) async fn draft_documents(&self, owner: &str) -> Result<Vec<DraftDocument>> {
        macro_rules! read {
            ($db:expr) => {
                sqlx::query_scalar::<_, String>(
                    "SELECT metadata FROM draft_documents WHERE owner_user_id = $1 ORDER BY id",
                )
                .bind(owner)
                .fetch_all(&$db.pool)
                .await?
            };
        }
        let values = match &self.backend {
            StorageBackend::Postgres(db) => read!(db),
            StorageBackend::Sqlite(db) => read!(db),
        };
        values
            .iter()
            .map(|v| serde_json::from_str(v).map_err(Into::into))
            .collect()
    }

    pub(crate) async fn draft_document(
        &self,
        owner: &str,
        id: &str,
    ) -> Result<Option<DraftDocument>> {
        macro_rules! read {
            ($db:expr) => {
                sqlx::query_scalar::<_, String>(
                    "SELECT value FROM draft_documents WHERE owner_user_id = $1 AND id = $2",
                )
                .bind(owner)
                .bind(id)
                .fetch_optional(&$db.pool)
                .await?
            };
        }
        let value = match &self.backend {
            StorageBackend::Postgres(db) => read!(db),
            StorageBackend::Sqlite(db) => read!(db),
        };
        value
            .map(|v| serde_json::from_str(&v).map_err(Into::into))
            .transpose()
    }

    pub(crate) async fn draft_history(&self, owner: &str, id: &str) -> Result<Vec<DraftDocument>> {
        macro_rules! read { ($db:expr) => {
            sqlx::query_scalar::<_, String>("SELECT value FROM draft_document_history WHERE owner_user_id = $1 AND document_id = $2 ORDER BY revision DESC LIMIT 30")
                .bind(owner).bind(id).fetch_all(&$db.pool).await?
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

    #[cfg(test)]
    pub(crate) async fn mutate_draft_document(
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
        self.mutate_draft_document_in_workspace(owner, mutation, &shared)
            .await
    }

    pub(crate) async fn mutate_draft_document_in_workspace(
        &self,
        owner: &str,
        mutation: &DraftMutation,
        shared: &[crate::session_folders::SessionFolder],
    ) -> Result<DraftResult> {
        let request_hash = crate::admin::hex_sha256(&serde_json::to_vec(mutation)?);
        macro_rules! write { ($db:expr) => {{
            let mut tx = $db.pool.begin().await?;
            // The first statement acquires the owner's writer lock on BOTH
            // backends, before any read (including SQLite's snapshot).
            sqlx::query("INSERT INTO draft_document_owners (owner_user_id, revision) VALUES ($1, 1) ON CONFLICT (owner_user_id) DO UPDATE SET revision = draft_document_owners.revision + 1")
                .bind(owner).execute(&mut *tx).await?;
            let previous: Option<(String, String)> = sqlx::query_as("SELECT request_hash, document_id FROM draft_document_operations WHERE owner_user_id = $1 AND operation_id = $2")
                .bind(owner).bind(&mutation.operation_id).fetch_optional(&mut *tx).await?;
            let values: Vec<String> = sqlx::query_scalar("SELECT metadata FROM draft_documents WHERE owner_user_id = $1")
                .bind(owner).fetch_all(&mut *tx).await?;
            let mut entries: Vec<DraftDocument> = values.iter().map(|v| serde_json::from_str(v)).collect::<std::result::Result<_, _>>()?;
            let target: Option<String> = sqlx::query_scalar("SELECT value FROM draft_documents WHERE owner_user_id = $1 AND id = $2")
                .bind(owner).bind(&mutation.document_id).fetch_optional(&mut *tx).await?;
            if let Some(target) = target {
                let target: DraftDocument = serde_json::from_str(&target)?;
                if let Some(entry) = entries.iter_mut().find(|d| d.id == target.id) { *entry = target; }
            }
            if let Some((hash, id)) = previous {
                if hash != request_hash { return Ok(DraftResult::Invalid("Operation id was already used for different content".to_owned())); }
                let Some(document) = entries.iter().find(|d| d.id == id) else { bail!("draft operation lost its document"); };
                return Ok(DraftResult::Applied(document.clone()));
            }
            let mut mapped = mutation.clone();
            let parent = match &mut mapped.change { DraftChange::Create { parent_id, .. } | DraftChange::Move { parent_id } => Some(parent_id), _ => None };
            if let Some(Some(parent)) = parent {
                let destination: Option<String> = sqlx::query_scalar("SELECT folder_id FROM draft_workspace_folder_imports WHERE owner_user_id=$1 AND document_id=$2").bind(owner).bind(&*parent).fetch_optional(&mut *tx).await?;
                if let Some(destination) = destination { *parent = destination; }
            }
            let result = apply(&entries, &mapped, chrono::Utc::now().timestamp_millis(), shared);
            if let DraftResult::Applied(ref next) = result {
                if let Some(previous) = entries.iter().find(|d| d.id == next.id && (d.body != next.body || d.attachments != next.attachments) && d.kind == DraftKind::Document) {
                    sqlx::query("INSERT INTO draft_document_history (owner_user_id, document_id, revision, value) VALUES ($1, $2, $3, $4)")
                        .bind(owner).bind(&next.id).bind(previous.revision).bind(serde_json::to_string(previous)?).execute(&mut *tx).await?;
                    sqlx::query("DELETE FROM draft_document_history WHERE owner_user_id = $1 AND document_id = $2 AND revision NOT IN (SELECT revision FROM draft_document_history WHERE owner_user_id = $1 AND document_id = $2 ORDER BY revision DESC LIMIT 30)")
                        .bind(owner).bind(&next.id).execute(&mut *tx).await?;
                }
                let mut metadata = serde_json::to_value(next)?;
                metadata.as_object_mut().expect("document object").remove("body");
                metadata.as_object_mut().expect("document object").remove("attachments");
                sqlx::query("INSERT INTO draft_documents (owner_user_id, id, value, metadata) VALUES ($1, $2, $3, $4) ON CONFLICT (owner_user_id, id) DO UPDATE SET value = excluded.value, metadata = excluded.metadata")
                    .bind(owner).bind(&next.id).bind(serde_json::to_string(next)?).bind(serde_json::to_string(&metadata)?).execute(&mut *tx).await?;
                sqlx::query("INSERT INTO draft_document_operations (owner_user_id, operation_id, request_hash, document_id) VALUES ($1, $2, $3, $4)")
                    .bind(owner).bind(&mutation.operation_id).bind(&request_hash).bind(&next.id).execute(&mut *tx).await?;
                tx.commit().await?;
            }
            result
        }}; }
        Ok(match &self.backend {
            StorageBackend::Postgres(db) => write!(db),
            StorageBackend::Sqlite(db) => write!(db),
        })
    }
}

#[cfg(test)]
mod tests;
