//! Workspace navigation metadata. Document bodies remain in their owned replicas.
use super::draft_documents::{DraftDocument, DraftKind};
use super::{StorageBackend, Store};
use anyhow::Result;

impl Store {
    /// Import legacy Draft directories once, before the Hub's startup restore.
    /// Stable owner-scoped IDs preserve duplicate names and isolate principals.
    pub(crate) async fn integrate_draft_folders(&self) -> Result<()> {
        macro_rules! integrate { ($db:expr, $insert:expr) => {{
            let mut tx = $db.pool.begin().await?;
            let rows: Vec<(String, String)> = sqlx::query_as("SELECT owner_user_id, metadata FROM draft_documents ORDER BY owner_user_id, id").fetch_all(&mut *tx).await?;
            let mut imports = std::collections::HashMap::new();
            for (owner, value) in &rows {
                let document: DraftDocument = serde_json::from_str(value)?;
                if document.kind != DraftKind::Folder || document.deleted { continue; }
                let id = format!("f-draft-{}", crate::admin::hex_sha256(&serde_json::to_vec(&(owner, &document.id))?));
                imports.insert((owner.clone(), document.id.clone()), id);
            }
            let mut position: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(position), 0) FROM session_folders").fetch_one(&mut *tx).await?;
            for (owner, value) in &rows {
                let document: DraftDocument = serde_json::from_str(value)?;
                let Some(id) = imports.get(&(owner.clone(), document.id.clone())) else { continue; };
                let already: Option<String> = sqlx::query_scalar("SELECT folder_id FROM draft_workspace_folder_imports WHERE owner_user_id = $1 AND document_id = $2").bind(owner).bind(&document.id).fetch_optional(&mut *tx).await?;
                if already.is_some() { continue; }
                position += 1;
                let parent = document.parent_id.as_ref().and_then(|parent| imports.get(&(owner.clone(), parent.clone())));
                sqlx::query($insert).bind(id).bind(owner).bind(&document.title).bind(parent).bind(position).bind(chrono::Utc::now().timestamp_millis()).execute(&mut *tx).await?;
                sqlx::query("INSERT INTO draft_workspace_folder_imports (owner_user_id, document_id, folder_id) VALUES ($1,$2,$3)").bind(owner).bind(&document.id).bind(id).execute(&mut *tx).await?;
            }
            // Only affected full values are loaded; the library scan above is metadata-only.
            for (owner, value) in &rows {
                let metadata: DraftDocument = serde_json::from_str(value)?;
                if metadata.kind != DraftKind::Document { continue; }
                let Some(parent) = metadata.parent_id.as_ref().and_then(|parent| imports.get(&(owner.clone(), parent.clone()))) else { continue; };
                let full: String = sqlx::query_scalar("SELECT value FROM draft_documents WHERE owner_user_id = $1 AND id = $2").bind(owner).bind(&metadata.id).fetch_one(&mut *tx).await?;
                let mut document: DraftDocument = serde_json::from_str(&full)?;
                document.parent_id = Some(parent.clone());
                document.revision += 1;
                document.metadata_revision += 1;
                let mut metadata = document.clone(); metadata.body.clear(); metadata.attachments.clear();
                sqlx::query("UPDATE draft_documents SET value=$3, metadata=$4 WHERE owner_user_id=$1 AND id=$2").bind(owner).bind(&document.id).bind(serde_json::to_string(&document)?).bind(serde_json::to_string(&metadata)?).execute(&mut *tx).await?;
            }
            tx.commit().await?;
        }}; }
        match &self.backend {
            StorageBackend::Postgres(db) => integrate!(
                db,
                "INSERT INTO session_folders (id,owner_user_id,name,parent_id,position,created_at,updated_at) VALUES ($1,$2,$3,$4,$5,to_timestamp($6::bigint::double precision/1000),to_timestamp($6::bigint::double precision/1000))"
            ),
            StorageBackend::Sqlite(db) => integrate!(
                db,
                "INSERT INTO session_folders (id,owner_user_id,name,parent_id,position,created_at_ms,updated_at_ms) VALUES ($1,$2,$3,$4,$5,$6,$6)"
            ),
        }
        Ok(())
    }

    pub(crate) async fn load_workspace_orders(&self) -> Result<Vec<(String, Vec<String>)>> {
        macro_rules! read {
            ($db:expr) => {
                sqlx::query_as::<_, (String, String)>(
                    "SELECT owner_user_id,value FROM workspace_item_order",
                )
                .fetch_all(&$db.pool)
                .await?
            };
        }
        let rows = match &self.backend {
            StorageBackend::Postgres(db) => read!(db),
            StorageBackend::Sqlite(db) => read!(db),
        };
        rows.into_iter()
            .map(|(owner, value)| Ok((owner, serde_json::from_str(&value)?)))
            .collect()
    }
    pub(crate) async fn update_workspace_order(&self, owner: &str, order: &[String]) -> Result<()> {
        let value = serde_json::to_string(order)?;
        macro_rules! write { ($db:expr) => { sqlx::query("INSERT INTO workspace_item_order (owner_user_id,value) VALUES ($1,$2) ON CONFLICT(owner_user_id) DO UPDATE SET value=excluded.value").bind(owner).bind(&value).execute(&$db.pool).await? }; }
        match &self.backend {
            StorageBackend::Postgres(db) => {
                write!(db);
            }
            StorageBackend::Sqlite(db) => {
                write!(db);
            }
        }
        Ok(())
    }
}

// Called within the same transaction that removes shared folders. Acquire the
// same per-owner writer lock as content mutations before loading document values.
macro_rules! reparent_workspace_documents {
    ($tx:expr, $owner:expr, $folders:expr) => {{
        let old: Vec<(String, Option<String>)> = sqlx::query_as(
            "SELECT id,parent_id FROM session_folders WHERE owner_user_id IS NOT DISTINCT FROM $1"
        ).bind($owner).fetch_all(&mut *$tx).await?;
        let removed: std::collections::HashMap<String, Option<String>> = old.into_iter()
            .filter(|(id, _)| !$folders.iter().any(|folder| &folder.id == id)).collect();
        if !removed.is_empty() {
            let owners: Vec<String> = sqlx::query_scalar("SELECT owner_user_id FROM draft_document_owners ORDER BY owner_user_id")
                .fetch_all(&mut *$tx).await?;
            for owner in owners {
                if $owner.is_some_and(|scope| scope != owner) { continue; }
                sqlx::query("UPDATE draft_document_owners SET revision=revision+1 WHERE owner_user_id=$1")
                    .bind(&owner).execute(&mut *$tx).await?;
                let metadata: Vec<String> = sqlx::query_scalar("SELECT metadata FROM draft_documents WHERE owner_user_id=$1")
                    .bind(&owner).fetch_all(&mut *$tx).await?;
                for value in metadata {
                    let metadata: $crate::store::draft_documents::DraftDocument = serde_json::from_str(&value)?;
                    if metadata.kind != $crate::store::draft_documents::DraftKind::Document { continue; }
                    let mut parent = metadata.parent_id.clone();
                    let mut seen = std::collections::HashSet::new();
                    while let Some(id) = parent.as_ref().filter(|id| removed.contains_key(*id)) {
                        if !seen.insert(id.clone()) { parent = None; break; }
                        parent = removed.get(id).cloned().flatten();
                    }
                    if parent == metadata.parent_id { continue; }
                    let value: String = sqlx::query_scalar("SELECT value FROM draft_documents WHERE owner_user_id=$1 AND id=$2")
                        .bind(&owner).bind(&metadata.id).fetch_one(&mut *$tx).await?;
                    let mut document: $crate::store::draft_documents::DraftDocument = serde_json::from_str(&value)?;
                    document.parent_id = parent;
                    document.revision += 1;
                    document.metadata_revision += 1;
                    let mut metadata = document.clone(); metadata.body.clear(); metadata.attachments.clear();
                    sqlx::query("UPDATE draft_documents SET value=$3,metadata=$4 WHERE owner_user_id=$1 AND id=$2")
                        .bind(&owner).bind(&document.id).bind(serde_json::to_string(&document)?)
                        .bind(serde_json::to_string(&metadata)?).execute(&mut *$tx).await?;
                }
            }
        }
    }};
}
pub(crate) use reparent_workspace_documents;
