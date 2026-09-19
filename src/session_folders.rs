//! Sessions-sidebar folders: the typed truth behind the `"folders"` sync state
//! (`docs/sessions-folders.md`). A folder belongs to one product user, nests
//! through `parent`, is ordered among its siblings by `position`, and may bind
//! one project label so the client files that project's sessions with no
//! explicit placement. `placement` holds only explicit moves; the effective
//! folder of a session is derived on the client, so a renamed, nested or
//! removed folder never strands a session.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::core::merge_session_order;

/// Longest accepted folder name, in characters.
pub const FOLDER_NAME_MAX_CHARS: usize = 80;
/// Longest accepted folder id or project label, in bytes.
const FOLDER_KEY_MAX_BYTES: usize = 200;
/// Folders one owner may hold; the sidebar is a shelf, not a file system.
pub const FOLDER_CAP: usize = 512;

/// Placement value for "explicitly at the top level". Distinct from having no
/// placement at all: a session of a project-bound folder files itself there
/// unless the user moved it out, and "out to the top level" must stick.
pub const TOP_LEVEL: &str = "";

/// The owner slot of a folder: a product user id, or `None` while product auth
/// is disabled (every folder is then shared by the single local user).
pub type FolderOwner = Option<String>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionFolder {
    pub id: String,
    /// Serialized as `owner` so the fan-out projection can filter by principal
    /// before stripping it; never persisted through this representation.
    #[serde(rename = "owner", default, skip_serializing_if = "Option::is_none")]
    pub owner_user_id: FolderOwner,
    pub name: String,
    pub parent: Option<String>,
    pub position: i64,
    pub project: Option<String>,
}

/// Who is mutating: the product principal reduced to what folders care about.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FolderActor {
    /// Owner slot new folders are created under.
    pub user_id: FolderOwner,
    /// Owner-role grant: may touch any folder regardless of its owner.
    pub sees_all: bool,
}

impl FolderActor {
    /// The single local user while product auth is disabled.
    #[must_use]
    pub fn local() -> Self {
        Self {
            user_id: None,
            sees_all: true,
        }
    }
}

/// What a mutation changed, for the write-behind persistence queue.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct FolderEffects {
    /// The owner whose whole folder set must be rewritten.
    pub replaced_owner: Option<FolderOwner>,
    /// Explicit placements to persist on the session rows (`None` = root).
    pub placements: Vec<(String, Option<String>)>,
}

#[derive(Debug, Default, Clone)]
pub struct SessionFolders {
    folders: Vec<SessionFolder>,
    placement: HashMap<String, String>,
}

fn valid_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= FOLDER_KEY_MAX_BYTES
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
}

fn key_arg(args: &serde_json::Value, key: &str) -> Result<String, String> {
    let value = args
        .get(key)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("missing {key}"))?;
    if !valid_key(value) {
        return Err(format!("invalid {key}"));
    }
    Ok(value.to_owned())
}

fn optional_key_arg(args: &serde_json::Value, key: &str) -> Result<Option<String>, String> {
    match args.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(value)) if valid_key(value) => Ok(Some(value.clone())),
        Some(_) => Err(format!("invalid {key}")),
    }
}

fn name_arg(args: &serde_json::Value) -> Result<String, String> {
    let name = args
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .trim();
    if name.is_empty() {
        return Err("folder name cannot be empty".to_owned());
    }
    if name.chars().count() > FOLDER_NAME_MAX_CHARS {
        return Err("folder name is too long".to_owned());
    }
    if name.chars().any(char::is_control) {
        return Err("folder name contains control characters".to_owned());
    }
    Ok(name.to_owned())
}

fn project_arg(args: &serde_json::Value) -> Result<Option<String>, String> {
    match args.get("project") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(value)) => {
            let value = value.trim();
            if value.is_empty() {
                return Ok(None);
            }
            if value.len() > FOLDER_KEY_MAX_BYTES || value.chars().any(char::is_control) {
                return Err("invalid project".to_owned());
            }
            Ok(Some(value.to_owned()))
        }
        Some(_) => Err("invalid project".to_owned()),
    }
}

impl SessionFolders {
    /// Replace the folder set (startup restore), keeping placements.
    pub fn set_folders(&mut self, mut folders: Vec<SessionFolder>) {
        folders.sort_by(|a, b| a.position.cmp(&b.position).then_with(|| a.id.cmp(&b.id)));
        self.folders = folders;
    }

    /// Record a restored explicit placement.
    pub fn restore_placement(&mut self, session_id: String, folder_id: String) {
        self.placement.insert(session_id, folder_id);
    }

    #[cfg(test)]
    fn folders(&self) -> &[SessionFolder] {
        &self.folders
    }

    /// Every folder in one owner slot, in sibling order.
    #[must_use]
    pub fn folders_of(&self, owner: Option<&str>) -> Vec<SessionFolder> {
        self.folders
            .iter()
            .filter(|folder| folder.owner_user_id.as_deref() == owner)
            .cloned()
            .collect()
    }

    /// Drop a deleted session's explicit placement (its row is gone).
    pub fn forget_session(&mut self, session_id: &str) {
        self.placement.remove(session_id);
    }

    /// The `"folders"` sync value: every folder (with its owner, for the
    /// projection) and every placement that still points at a live folder or
    /// at the explicit top level.
    #[must_use]
    pub fn value(&self) -> serde_json::Value {
        let ids: HashSet<&str> = self.folders.iter().map(|f| f.id.as_str()).collect();
        let mut placement: Vec<(&String, &String)> = self
            .placement
            .iter()
            .filter(|(_, folder)| *folder == TOP_LEVEL || ids.contains(folder.as_str()))
            .collect();
        placement.sort();
        serde_json::json!({
            "folders": self.folders,
            "placement": placement
                .into_iter()
                .map(|(session, folder)| (session.clone(), serde_json::Value::String(folder.clone())))
                .collect::<serde_json::Map<String, serde_json::Value>>(),
        })
    }

    fn find(&self, id: &str) -> Option<&SessionFolder> {
        self.folders.iter().find(|folder| folder.id == id)
    }

    fn can_touch(&self, actor: &FolderActor, folder: &SessionFolder) -> bool {
        actor.sees_all || folder.owner_user_id == actor.user_id
    }

    /// A folder the actor may mutate, by id.
    fn touchable(&self, actor: &FolderActor, id: &str) -> Result<&SessionFolder, String> {
        let folder = self.find(id).ok_or("unknown folder")?;
        if !self.can_touch(actor, folder) {
            return Err("not allowed to change this folder".to_owned());
        }
        Ok(folder)
    }

    /// `Some(parent)` must exist and belong to the actor; `None` is the root.
    fn resolve_parent(
        &self,
        actor: &FolderActor,
        parent: Option<&str>,
    ) -> Result<Option<String>, String> {
        match parent {
            None => Ok(None),
            Some(id) => {
                self.touchable(actor, id)
                    .map_err(|_| "unknown parent folder".to_owned())?;
                Ok(Some(id.to_owned()))
            }
        }
    }

    fn is_descendant(&self, candidate: Option<&str>, ancestor: &str) -> bool {
        let mut cursor = candidate;
        // Bounded by the folder count so a corrupt loop can never spin.
        for _ in 0..=self.folders.len() {
            let Some(id) = cursor else {
                return false;
            };
            if id == ancestor {
                return true;
            }
            cursor = self.find(id).and_then(|folder| folder.parent.as_deref());
        }
        false
    }

    fn next_position(&self, owner: Option<&str>, parent: Option<&str>) -> i64 {
        self.folders
            .iter()
            .filter(|folder| {
                folder.owner_user_id.as_deref() == owner && folder.parent.as_deref() == parent
            })
            .map(|folder| folder.position.saturating_add(1))
            .max()
            .unwrap_or(0)
    }

    fn project_taken(&self, owner: Option<&str>, project: &str, except: &str) -> bool {
        self.folders.iter().any(|folder| {
            folder.id != except
                && folder.owner_user_id.as_deref() == owner
                && folder.project.as_deref() == Some(project)
        })
    }

    /// Apply one client mutation. Validation runs against the current state
    /// before anything changes, so a rejected mutation leaves no trace.
    ///
    /// # Errors
    /// A user-facing reason when the mutation is malformed or not allowed.
    pub fn apply(
        &mut self,
        actor: &FolderActor,
        mutation: &str,
        args: &serde_json::Value,
    ) -> Result<FolderEffects, String> {
        match mutation {
            "create" => self.create(actor, args),
            "rename" => {
                let id = key_arg(args, "id")?;
                let name = name_arg(args)?;
                let owner = self.touchable(actor, &id)?.owner_user_id.clone();
                if let Some(folder) = self.folders.iter_mut().find(|folder| folder.id == id) {
                    folder.name = name;
                }
                Ok(FolderEffects {
                    replaced_owner: Some(owner),
                    placements: Vec::new(),
                })
            }
            "move" => self.move_folder(actor, args),
            "reorder" => self.reorder(actor, args),
            "bind" => {
                let id = key_arg(args, "id")?;
                let project = project_arg(args)?;
                let owner = self.touchable(actor, &id)?.owner_user_id.clone();
                if let Some(project) = project.as_deref()
                    && self.project_taken(owner.as_deref(), project, &id)
                {
                    return Err("another folder already holds this project".to_owned());
                }
                if let Some(folder) = self.folders.iter_mut().find(|folder| folder.id == id) {
                    folder.project = project;
                }
                Ok(FolderEffects {
                    replaced_owner: Some(owner),
                    placements: Vec::new(),
                })
            }
            "place" => self.place(actor, args),
            "remove" => self.remove(actor, args),
            _ => Err(format!("unknown folders mutation {mutation}")),
        }
    }

    /// Whether `create` with these arguments already produced exactly this
    /// folder for this actor. The sync arbiter's mutation-id dedupe does not
    /// survive a Controller restart, so a client that resends its durable
    /// outbox would otherwise hit "folder id already exists" forever. Any
    /// difference (owner, name, parent, project) is a genuine conflict and
    /// still fails through the normal path.
    pub(crate) fn is_replayed_create(&self, actor: &FolderActor, args: &serde_json::Value) -> bool {
        let (Ok(id), Ok(name), Ok(parent), Ok(project)) = (
            key_arg(args, "id"),
            name_arg(args),
            optional_key_arg(args, "parent"),
            project_arg(args),
        ) else {
            return false;
        };
        self.find(&id).is_some_and(|folder| {
            folder.owner_user_id == actor.user_id
                && folder.name == name
                && folder.parent == parent
                && folder.project == project
        })
    }

    fn create(
        &mut self,
        actor: &FolderActor,
        args: &serde_json::Value,
    ) -> Result<FolderEffects, String> {
        let id = key_arg(args, "id")?;
        let name = name_arg(args)?;
        let parent = self.resolve_parent(actor, optional_key_arg(args, "parent")?.as_deref())?;
        let project = project_arg(args)?;
        if self.find(&id).is_some() {
            return Err("folder id already exists".to_owned());
        }
        let owner = actor.user_id.clone();
        if self.folders_of(owner.as_deref()).len() >= FOLDER_CAP {
            return Err("too many folders".to_owned());
        }
        if let Some(project) = project.as_deref()
            && self.project_taken(owner.as_deref(), project, &id)
        {
            return Err("another folder already holds this project".to_owned());
        }
        let position = self.next_position(owner.as_deref(), parent.as_deref());
        self.folders.push(SessionFolder {
            id,
            owner_user_id: owner.clone(),
            name,
            parent,
            position,
            project,
        });
        Ok(FolderEffects {
            replaced_owner: Some(owner),
            placements: Vec::new(),
        })
    }

    fn move_folder(
        &mut self,
        actor: &FolderActor,
        args: &serde_json::Value,
    ) -> Result<FolderEffects, String> {
        let id = key_arg(args, "id")?;
        let parent = self.resolve_parent(actor, optional_key_arg(args, "parent")?.as_deref())?;
        let owner = self.touchable(actor, &id)?.owner_user_id.clone();
        if self.is_descendant(parent.as_deref(), &id) {
            return Err("a folder cannot move into itself".to_owned());
        }
        let position = self.next_position(owner.as_deref(), parent.as_deref());
        if let Some(folder) = self.folders.iter_mut().find(|folder| folder.id == id) {
            folder.parent = parent;
            folder.position = position;
        }
        Ok(FolderEffects {
            replaced_owner: Some(owner),
            placements: Vec::new(),
        })
    }

    fn reorder(
        &mut self,
        actor: &FolderActor,
        args: &serde_json::Value,
    ) -> Result<FolderEffects, String> {
        let parent = self.resolve_parent(actor, optional_key_arg(args, "parent")?.as_deref())?;
        let submitted: Vec<String> = args
            .get("order")
            .and_then(serde_json::Value::as_array)
            .ok_or("reorder: missing order")?
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect();
        // Siblings are scoped to one owner: a submitted id from another owner's
        // tree is simply not among the names being permuted.
        let owner = actor.user_id.clone();
        let mut siblings: Vec<&SessionFolder> = self
            .folders
            .iter()
            .filter(|folder| folder.parent == parent && folder.owner_user_id == owner)
            .collect();
        siblings.sort_by_key(|folder| (folder.position, folder.id.clone()));
        let existing: Vec<String> = siblings.iter().map(|folder| folder.id.clone()).collect();
        let existing_set: HashSet<&str> = existing.iter().map(String::as_str).collect();
        let submitted: Vec<String> = submitted
            .into_iter()
            .filter(|id| existing_set.contains(id.as_str()))
            .collect();
        let merged = merge_session_order(&existing, &submitted);
        for (index, id) in merged.iter().enumerate() {
            if let Some(folder) = self.folders.iter_mut().find(|folder| &folder.id == id) {
                folder.position = i64::try_from(index).unwrap_or(i64::MAX);
            }
        }
        self.folders
            .sort_by(|a, b| a.position.cmp(&b.position).then_with(|| a.id.cmp(&b.id)));
        Ok(FolderEffects {
            replaced_owner: Some(owner),
            placements: Vec::new(),
        })
    }

    fn place(
        &mut self,
        actor: &FolderActor,
        args: &serde_json::Value,
    ) -> Result<FolderEffects, String> {
        let folder = optional_key_arg(args, "folder")?;
        if let Some(id) = folder.as_deref() {
            self.touchable(actor, id)?;
        }
        let session_ids: Vec<String> = args
            .get("session_ids")
            .and_then(serde_json::Value::as_array)
            .ok_or("place: missing session_ids")?
            .iter()
            .filter_map(|v| v.as_str())
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
            .collect();
        // `folder: null` is an explicit top-level placement, not "unplaced":
        // it must override a project binding that would file the session.
        let target = folder.unwrap_or_else(|| TOP_LEVEL.to_owned());
        let mut placements = Vec::with_capacity(session_ids.len());
        for session_id in session_ids {
            self.placement.insert(session_id.clone(), target.clone());
            placements.push((session_id, Some(target.clone())));
        }
        Ok(FolderEffects {
            replaced_owner: None,
            placements,
        })
    }

    fn remove(
        &mut self,
        actor: &FolderActor,
        args: &serde_json::Value,
    ) -> Result<FolderEffects, String> {
        let id = key_arg(args, "id")?;
        let removed = self.touchable(actor, &id)?.clone();
        let owner = removed.owner_user_id.clone();
        let parent = removed.parent.clone();
        // Children step up one level, after the parent's existing siblings, in
        // their current order; nothing is deleted but the folder itself.
        let mut next = self.next_position(owner.as_deref(), parent.as_deref());
        let mut children: Vec<&mut SessionFolder> = self
            .folders
            .iter_mut()
            .filter(|folder| folder.parent.as_deref() == Some(id.as_str()))
            .collect();
        children.sort_by_key(|folder| (folder.position, folder.id.clone()));
        for child in children {
            child.parent = parent.clone();
            child.position = next;
            next = next.saturating_add(1);
        }
        self.folders.retain(|folder| folder.id != id);
        let mut placements = Vec::new();
        let mut moved: Vec<String> = self
            .placement
            .iter()
            .filter(|(_, folder)| **folder == id)
            .map(|(session, _)| session.clone())
            .collect();
        moved.sort();
        let target = parent.clone().unwrap_or_else(|| TOP_LEVEL.to_owned());
        for session_id in moved {
            self.placement.insert(session_id.clone(), target.clone());
            placements.push((session_id, Some(target.clone())));
        }
        self.folders
            .sort_by(|a, b| a.position.cmp(&b.position).then_with(|| a.id.cmp(&b.id)));
        Ok(FolderEffects {
            replaced_owner: Some(owner),
            placements,
        })
    }
}

/// Project the `"folders"` sync value down to what one principal may see:
/// folders whose owner passes `can_see` (with the owner slot stripped) and
/// placements of visible sessions into those folders.
#[must_use]
pub fn project_folders_value(
    value: serde_json::Value,
    visible_sessions: &HashSet<String>,
    can_see: impl Fn(Option<&str>) -> bool,
) -> serde_json::Value {
    let Some(object) = value.as_object() else {
        return value;
    };
    let mut kept_ids: HashSet<String> = HashSet::new();
    let folders: Vec<serde_json::Value> = object
        .get("folders")
        .and_then(serde_json::Value::as_array)
        .map(|folders| {
            folders
                .iter()
                .filter(|folder| can_see(folder.get("owner").and_then(serde_json::Value::as_str)))
                .map(|folder| {
                    let mut folder = folder.clone();
                    if let Some(map) = folder.as_object_mut() {
                        map.remove("owner");
                        if let Some(id) = map.get("id").and_then(serde_json::Value::as_str) {
                            kept_ids.insert(id.to_owned());
                        }
                    }
                    folder
                })
                .collect()
        })
        .unwrap_or_default();
    let placement: serde_json::Map<String, serde_json::Value> = object
        .get("placement")
        .and_then(serde_json::Value::as_object)
        .map(|placement| {
            placement
                .iter()
                .filter(|(session, folder)| {
                    visible_sessions.contains(session.as_str())
                        && folder
                            .as_str()
                            .is_some_and(|folder| folder == TOP_LEVEL || kept_ids.contains(folder))
                })
                .map(|(session, folder)| (session.clone(), folder.clone()))
                .collect()
        })
        .unwrap_or_default();
    serde_json::json!({ "folders": folders, "placement": placement })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local() -> FolderActor {
        FolderActor::local()
    }

    fn user(id: &str) -> FolderActor {
        FolderActor {
            user_id: Some(id.to_owned()),
            sees_all: false,
        }
    }

    fn create(
        folders: &mut SessionFolders,
        actor: &FolderActor,
        id: &str,
        name: &str,
        parent: Option<&str>,
    ) -> FolderEffects {
        folders
            .apply(
                actor,
                "create",
                &serde_json::json!({"id": id, "name": name, "parent": parent}),
            )
            .unwrap()
    }

    fn ids_under(folders: &SessionFolders, parent: Option<&str>) -> Vec<String> {
        let mut rows: Vec<&SessionFolder> = folders
            .folders()
            .iter()
            .filter(|folder| folder.parent.as_deref() == parent)
            .collect();
        rows.sort_by_key(|folder| folder.position);
        rows.iter().map(|folder| folder.id.clone()).collect()
    }

    #[test]
    fn create_trims_names_and_appends_after_siblings() {
        let mut folders = SessionFolders::default();
        let effects = create(&mut folders, &local(), "f-a", "  Cowboy ", None);
        assert_eq!(effects.replaced_owner, Some(None));
        create(&mut folders, &local(), "f-b", "Garden", None);
        create(&mut folders, &local(), "f-c", "IME", Some("f-a"));
        assert_eq!(folders.folders()[0].name, "Cowboy");
        assert_eq!(ids_under(&folders, None), vec!["f-a", "f-b"]);
        assert_eq!(ids_under(&folders, Some("f-a")), vec!["f-c"]);
        assert_eq!(
            folders
                .apply(
                    &local(),
                    "create",
                    &serde_json::json!({"id": "f-d", "name": "  "})
                )
                .unwrap_err(),
            "folder name cannot be empty"
        );
        assert_eq!(
            folders
                .apply(
                    &local(),
                    "create",
                    &serde_json::json!({"id": "f-a", "name": "Again"})
                )
                .unwrap_err(),
            "folder id already exists"
        );
        assert_eq!(
            folders
                .apply(
                    &local(),
                    "create",
                    &serde_json::json!({"id": "f-e", "name": "Lost", "parent": "f-missing"})
                )
                .unwrap_err(),
            "unknown parent folder"
        );
        assert_eq!(
            folders
                .apply(
                    &local(),
                    "create",
                    &serde_json::json!({"id": "bad id", "name": "Spaces"})
                )
                .unwrap_err(),
            "invalid id"
        );
    }

    #[test]
    fn move_rejects_cycles_and_appends_in_the_new_parent() {
        let mut folders = SessionFolders::default();
        create(&mut folders, &local(), "f-a", "A", None);
        create(&mut folders, &local(), "f-b", "B", Some("f-a"));
        create(&mut folders, &local(), "f-c", "C", Some("f-b"));
        create(&mut folders, &local(), "f-d", "D", None);
        assert_eq!(
            folders
                .apply(
                    &local(),
                    "move",
                    &serde_json::json!({"id": "f-a", "parent": "f-c"})
                )
                .unwrap_err(),
            "a folder cannot move into itself"
        );
        assert_eq!(
            folders
                .apply(
                    &local(),
                    "move",
                    &serde_json::json!({"id": "f-a", "parent": "f-a"})
                )
                .unwrap_err(),
            "a folder cannot move into itself"
        );
        folders
            .apply(
                &local(),
                "move",
                &serde_json::json!({"id": "f-c", "parent": null}),
            )
            .unwrap();
        assert_eq!(ids_under(&folders, None), vec!["f-a", "f-d", "f-c"]);
        folders
            .apply(
                &local(),
                "move",
                &serde_json::json!({"id": "f-d", "parent": "f-b"}),
            )
            .unwrap();
        assert_eq!(ids_under(&folders, Some("f-b")), vec!["f-d"]);
    }

    #[test]
    fn reorder_permutes_only_submitted_siblings() {
        let mut folders = SessionFolders::default();
        for id in ["f-a", "f-b", "f-c", "f-d"] {
            create(&mut folders, &local(), id, id, None);
        }
        create(&mut folders, &local(), "f-x", "X", Some("f-a"));
        folders
            .apply(
                &local(),
                "reorder",
                &serde_json::json!({"parent": null, "order": ["f-c", "f-a", "f-x", "f-nope"]}),
            )
            .unwrap();
        assert_eq!(ids_under(&folders, None), vec!["f-c", "f-b", "f-a", "f-d"]);
        assert_eq!(ids_under(&folders, Some("f-a")), vec!["f-x"]);
    }

    #[test]
    fn bind_keeps_projects_unique_per_owner() {
        let mut folders = SessionFolders::default();
        create(&mut folders, &local(), "f-a", "A", None);
        create(&mut folders, &local(), "f-b", "B", None);
        folders
            .apply(
                &local(),
                "bind",
                &serde_json::json!({"id": "f-a", "project": " cowboy "}),
            )
            .unwrap();
        assert_eq!(
            folders
                .apply(
                    &local(),
                    "bind",
                    &serde_json::json!({"id": "f-b", "project": "cowboy"})
                )
                .unwrap_err(),
            "another folder already holds this project"
        );
        folders
            .apply(
                &local(),
                "bind",
                &serde_json::json!({"id": "f-a", "project": null}),
            )
            .unwrap();
        folders
            .apply(
                &local(),
                "bind",
                &serde_json::json!({"id": "f-b", "project": "cowboy"}),
            )
            .unwrap();
        assert_eq!(
            folders
                .apply(
                    &local(),
                    "create",
                    &serde_json::json!({"id": "f-c", "name": "C", "project": "cowboy"})
                )
                .unwrap_err(),
            "another folder already holds this project"
        );
        let value = folders.value();
        assert_eq!(value["folders"][0]["project"], serde_json::Value::Null);
        assert_eq!(value["folders"][1]["project"], "cowboy");
    }

    #[test]
    fn place_and_remove_move_sessions_to_the_parent() {
        let mut folders = SessionFolders::default();
        create(&mut folders, &local(), "f-a", "A", None);
        create(&mut folders, &local(), "f-b", "B", Some("f-a"));
        create(&mut folders, &local(), "f-c", "C", Some("f-b"));
        let effects = folders
            .apply(
                &local(),
                "place",
                &serde_json::json!({"session_ids": ["s1", "s2"], "folder": "f-b"}),
            )
            .unwrap();
        assert_eq!(
            effects.placements,
            vec![
                ("s1".to_owned(), Some("f-b".to_owned())),
                ("s2".to_owned(), Some("f-b".to_owned()))
            ]
        );
        folders
            .apply(
                &local(),
                "place",
                &serde_json::json!({"session_ids": ["s3"], "folder": "f-a"}),
            )
            .unwrap();
        let effects = folders
            .apply(
                &local(),
                "place",
                &serde_json::json!({"session_ids": ["s2"], "folder": null}),
            )
            .unwrap();
        // Moving to the top level is an explicit placement, so it survives a
        // project binding that would otherwise file the session.
        assert_eq!(
            effects.placements,
            vec![("s2".to_owned(), Some(String::new()))]
        );
        assert_eq!(folders.value()["placement"]["s1"], "f-b");
        assert_eq!(folders.value()["placement"]["s2"], "");
        assert_eq!(
            folders
                .apply(
                    &local(),
                    "place",
                    &serde_json::json!({"session_ids": ["s1"], "folder": "f-missing"})
                )
                .unwrap_err(),
            "unknown folder"
        );

        let effects = folders
            .apply(&local(), "remove", &serde_json::json!({"id": "f-b"}))
            .unwrap();
        assert_eq!(effects.replaced_owner, Some(None));
        assert_eq!(
            effects.placements,
            vec![("s1".to_owned(), Some("f-a".to_owned()))]
        );
        assert_eq!(ids_under(&folders, Some("f-a")), vec!["f-c"]);
        assert_eq!(folders.value()["placement"]["s1"], "f-a");
        assert_eq!(folders.value()["placement"]["s3"], "f-a");

        let effects = folders
            .apply(&local(), "remove", &serde_json::json!({"id": "f-a"}))
            .unwrap();
        assert_eq!(
            effects.placements,
            vec![
                ("s1".to_owned(), Some(String::new())),
                ("s3".to_owned(), Some(String::new()))
            ]
        );
        assert_eq!(ids_under(&folders, None), vec!["f-c"]);
        assert_eq!(
            folders.value()["placement"],
            serde_json::json!({"s1": "", "s2": "", "s3": ""})
        );
        assert_eq!(
            folders
                .apply(&local(), "remove", &serde_json::json!({"id": "f-a"}))
                .unwrap_err(),
            "unknown folder"
        );
    }

    #[test]
    fn actors_only_touch_their_own_folders_unless_they_see_all() {
        let mut folders = SessionFolders::default();
        create(&mut folders, &user("u1"), "f-u1", "Mine", None);
        create(&mut folders, &user("u2"), "f-u2", "Theirs", None);
        assert_eq!(
            folders
                .apply(
                    &user("u2"),
                    "rename",
                    &serde_json::json!({"id": "f-u1", "name": "X"})
                )
                .unwrap_err(),
            "not allowed to change this folder"
        );
        assert_eq!(
            folders
                .apply(
                    &user("u2"),
                    "create",
                    &serde_json::json!({"id": "f-in", "name": "In", "parent": "f-u1"})
                )
                .unwrap_err(),
            "unknown parent folder"
        );
        // Each owner has their own project namespace.
        folders
            .apply(
                &user("u1"),
                "bind",
                &serde_json::json!({"id": "f-u1", "project": "cowboy"}),
            )
            .unwrap();
        folders
            .apply(
                &user("u2"),
                "bind",
                &serde_json::json!({"id": "f-u2", "project": "cowboy"}),
            )
            .unwrap();
        let owner = FolderActor {
            user_id: Some("admin".to_owned()),
            sees_all: true,
        };
        let effects = folders
            .apply(
                &owner,
                "rename",
                &serde_json::json!({"id": "f-u1", "name": "Renamed"}),
            )
            .unwrap();
        assert_eq!(effects.replaced_owner, Some(Some("u1".to_owned())));
        assert_eq!(folders.folders_of(Some("u1"))[0].name, "Renamed");
    }

    #[test]
    fn projection_hides_other_owners_and_invisible_sessions() {
        let mut folders = SessionFolders::default();
        create(&mut folders, &user("u1"), "f-u1", "Mine", None);
        create(&mut folders, &user("u2"), "f-u2", "Theirs", None);
        create(&mut folders, &local(), "f-shared", "Shared", None);
        folders
            .apply(
                &local(),
                "place",
                &serde_json::json!({"session_ids": ["s-mine", "s-hidden"], "folder": "f-u1"}),
            )
            .unwrap();
        folders
            .apply(
                &local(),
                "place",
                &serde_json::json!({"session_ids": ["s-theirs"], "folder": "f-u2"}),
            )
            .unwrap();
        let visible: HashSet<String> = ["s-mine", "s-theirs"]
            .iter()
            .map(|id| (*id).to_owned())
            .collect();
        let projected = project_folders_value(folders.value(), &visible, |owner| {
            owner.is_none_or(|owner| owner == "u1")
        });
        let ids: Vec<&str> = projected["folders"]
            .as_array()
            .unwrap()
            .iter()
            .map(|folder| folder["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["f-u1", "f-shared"]);
        assert!(projected["folders"][0].get("owner").is_none());
        assert_eq!(projected["placement"]["s-mine"], "f-u1");
        assert!(projected["placement"].get("s-hidden").is_none());
        assert!(projected["placement"].get("s-theirs").is_none());
    }

    #[test]
    fn restore_orders_by_position_and_keeps_only_live_placements_in_value() {
        let mut folders = SessionFolders::default();
        folders.set_folders(vec![
            SessionFolder {
                id: "f-b".to_owned(),
                owner_user_id: None,
                name: "B".to_owned(),
                parent: None,
                position: 1,
                project: None,
            },
            SessionFolder {
                id: "f-a".to_owned(),
                owner_user_id: None,
                name: "A".to_owned(),
                parent: None,
                position: 0,
                project: Some("cowboy".to_owned()),
            },
        ]);
        folders.restore_placement("s1".to_owned(), "f-a".to_owned());
        folders.restore_placement("s2".to_owned(), "f-gone".to_owned());
        let value = folders.value();
        assert_eq!(value["folders"][0]["id"], "f-a");
        assert_eq!(value["folders"][1]["id"], "f-b");
        assert_eq!(value["placement"]["s1"], "f-a");
        assert!(value["placement"].get("s2").is_none());
    }
}
