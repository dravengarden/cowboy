//! Sessions Trash (`docs/sessions-folders.md#trash`): a reversible delete.
//!
//! A trashed session keeps its transcript, worktree and native thread; it only
//! leaves the live list, accepts no work and never launches a worker. A
//! trashed folder keeps its place in the tree so restore rebuilds it. Both are
//! deleted permanently by the Controller's retention sweep or on request.
use std::collections::HashSet;

use super::{Hub, Status, StoreWrite};
use crate::session_folders::{FolderActor, FolderOwner};

/// The one refusal every wake or work path returns for a trashed session.
pub const TRASHED_SESSION_REFUSAL: &str = "This session is in Trash. Restore it to continue.";

/// A trashed session for the retention sweep: id, owner, trashed-at ms.
pub(crate) type TrashedSession = (String, Option<String>, i64);

/// Outcome of a Trash move, for the caller that owns worker lifecycles.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Trashed {
    pub sessions: Vec<String>,
    pub folders: Vec<String>,
}

/// Outcome of a restore. `notes` explain any place or binding adjustment.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Restored {
    pub sessions: Vec<String>,
    pub folders: Vec<String>,
    pub notes: Vec<String>,
}

/// Why a session cannot move to Trash now, or `None` when it can.
fn trash_refusal(meta: &super::SessionMeta, in_flight: bool) -> Option<&'static str> {
    if meta.closing {
        Some("is being deleted")
    } else if meta.system {
        Some("is a system session")
    } else if in_flight || meta.status == Status::Busy {
        Some("is working; stop it first")
    } else if meta.background_tasks > 0 {
        Some("has background work running; stop it first")
    } else {
        None
    }
}

impl Hub {
    /// Whether the session is in Trash. Unknown sessions are not.
    #[must_use]
    pub fn session_is_trashed(&self, session_id: &str) -> bool {
        self.inner
            .sessions
            .lock()
            .get(session_id)
            .is_some_and(|session| session.meta.trashed_at_ms.is_some())
    }

    /// Move sessions and folders to Trash at `now_ms`, atomically. A folder
    /// takes every live folder below it and every session filed anywhere in
    /// it, each pinned to its current folder so the structure survives a later
    /// project rebinding. Nothing changes when any affected session cannot
    /// stop now; the error names the first one.
    ///
    /// # Errors
    /// An unknown, foreign or already trashed item, or a session that is
    /// working, closing or a system session.
    pub(crate) fn trash_items(
        &self,
        actor: &FolderActor,
        folder_ids: &[String],
        session_ids: &[String],
        now_ms: i64,
    ) -> Result<Trashed, String> {
        let _guard = self.inner.sync_apply_gate.lock();
        let (trashed, pins, folders_by_owner) = {
            let mut sessions = self.inner.sessions.lock();
            let mut folders = self.inner.folders.lock();
            let mut candidate = folders.clone();
            let (trashed_folders, owners) = candidate.trash(actor, folder_ids, now_ms)?;
            let inside: HashSet<&str> = trashed_folders.iter().map(String::as_str).collect();
            let mut targets: Vec<(String, Option<String>)> = Vec::new();
            for id in session_ids {
                let session = sessions.get(id).ok_or("Session no longer exists")?;
                if session.meta.trashed_at_ms.is_some() {
                    continue;
                }
                let folder = folders.display_folder(
                    actor,
                    id,
                    super::session_continuation::sidebar_project(&session.meta),
                );
                targets.push((id.clone(), folder));
            }
            if !inside.is_empty() {
                for (id, session) in sessions.iter() {
                    let managed = session
                        .meta
                        .execution_binding
                        .as_ref()
                        .is_some_and(|binding| binding.managed().is_some());
                    if session.meta.trashed_at_ms.is_some()
                        || managed
                        || targets.iter().any(|(target, _)| target == id)
                    {
                        continue;
                    }
                    let folder = folders.display_folder(
                        actor,
                        id,
                        super::session_continuation::sidebar_project(&session.meta),
                    );
                    if folder
                        .as_deref()
                        .is_some_and(|folder| inside.contains(folder))
                    {
                        targets.push((id.clone(), folder));
                    }
                }
            }
            for (id, _) in &targets {
                let session = &sessions[id];
                if !actor.sees_all && session.meta.owner_user_id != actor.user_id {
                    return Err("not allowed to change this session".into());
                }
                if let Some(reason) = trash_refusal(&session.meta, session.in_flight) {
                    return Err(format!("\"{}\" {reason}", session.meta.title));
                }
            }
            for (id, _) in &targets {
                if let Some(session) = sessions.get_mut(id) {
                    session.meta.trashed_at_ms = Some(now_ms);
                }
            }
            *folders = candidate;
            // Only filed sessions are pinned: an unfiled one keeps following
            // its project binding after restore.
            let filed: Vec<_> = targets
                .iter()
                .filter(|(_, folder)| folder.is_some())
                .cloned()
                .collect();
            let pins = folders.pin(&filed);
            let folders_by_owner: Vec<_> = owners
                .iter()
                .map(|owner| (owner.clone(), folders.folders_of(owner.as_deref())))
                .collect();
            (
                Trashed {
                    sessions: targets.into_iter().map(|(id, _)| id).collect(),
                    folders: trashed_folders,
                },
                pins,
                folders_by_owner,
            )
        };
        // Folders and placements first: a crash before the session write
        // leaves sessions live, never a trashed session outside its folder.
        self.persist_trash(
            &trashed.sessions,
            Some(now_ms),
            pins,
            folders_by_owner,
            false,
        );
        Ok(trashed)
    }

    /// Restore sessions and folders from Trash. A folder brings back
    /// everything below it; a session whose folder is gone or still in Trash
    /// returns to the nearest live ancestor, or the top level.
    ///
    /// # Errors
    /// An unknown, foreign or live item.
    pub(crate) fn restore_items(
        &self,
        actor: &FolderActor,
        folder_ids: &[String],
        session_ids: &[String],
    ) -> Result<Restored, String> {
        let _guard = self.inner.sync_apply_gate.lock();
        let (restored, pins, folders_by_owner) = {
            let mut sessions = self.inner.sessions.lock();
            let mut folders = self.inner.folders.lock();
            let mut candidate = folders.clone();
            let (restored_folders, owners, mut notes) = candidate.restore(actor, folder_ids)?;
            let mut targets: Vec<String> = Vec::new();
            for id in session_ids {
                let session = sessions.get(id).ok_or("Session no longer exists")?;
                if session.meta.trashed_at_ms.is_none() {
                    return Err(format!("\"{}\" is not in Trash", session.meta.title));
                }
                targets.push(id.clone());
            }
            for (id, session) in sessions.iter() {
                if session.meta.trashed_at_ms.is_some()
                    && !targets.contains(id)
                    && candidate
                        .placement_of(id)
                        .is_some_and(|folder| restored_folders.iter().any(|r| r == folder))
                {
                    targets.push(id.clone());
                }
            }
            // Every target, chosen or carried by its folder, must belong to
            // the actor, and none may already be on its way to deletion: a
            // restore must never report success for a session about to vanish.
            for id in &targets {
                let meta = &sessions[id].meta;
                if !actor.sees_all && meta.owner_user_id != actor.user_id {
                    return Err("not allowed to change this session".into());
                }
                if meta.closing {
                    return Err(format!("\"{}\" is being deleted permanently", meta.title));
                }
            }
            let mut pins = Vec::new();
            for id in &targets {
                let placed = candidate.placement_of(id).map(str::to_owned);
                if let Some(folder) = placed.filter(|folder| !folder.is_empty())
                    && candidate.nearest_live(Some(&folder)).as_deref() != Some(folder.as_str())
                {
                    let target = candidate.nearest_live(Some(&folder));
                    pins.push((id.clone(), target));
                }
            }
            let moved = pins.len();
            let pins = candidate.pin(&pins);
            if moved > 0 && folder_ids.is_empty() && session_ids.len() == 1 {
                notes.push("Its folder is not available, so it was restored to the nearest available folder.".into());
            } else if moved > 0 {
                notes.push(format!(
                    "{moved} sessions were restored to the nearest available folder."
                ));
            }
            for id in &targets {
                if let Some(session) = sessions.get_mut(id) {
                    session.meta.trashed_at_ms = None;
                }
            }
            *folders = candidate;
            let folders_by_owner: Vec<(FolderOwner, _)> = owners
                .iter()
                .map(|owner| (owner.clone(), folders.folders_of(owner.as_deref())))
                .collect();
            (
                Restored {
                    sessions: targets,
                    folders: restored_folders,
                    notes,
                },
                pins,
                folders_by_owner,
            )
        };
        // Sessions leave Trash in storage before their folders do: a crash in
        // between must never leave a restored folder holding sessions that
        // retention would still delete.
        self.persist_trash(&restored.sessions, None, pins, folders_by_owner, true);
        Ok(restored)
    }

    /// Permanently remove trashed folders that hold no session any more.
    /// Sessions are deleted first through their own lifecycle; a folder whose
    /// session deletion is still pending stays for the next attempt.
    pub(crate) fn purge_trashed_folders(
        &self,
        actor: &FolderActor,
        folder_ids: &[String],
    ) -> Vec<String> {
        let _guard = self.inner.sync_apply_gate.lock();
        let (removed, folders_by_owner) = {
            let sessions = self.inner.sessions.lock();
            let mut folders = self.inner.folders.lock();
            let occupied: HashSet<String> = sessions
                .keys()
                .filter_map(|id| folders.placement_of(id).map(str::to_owned))
                .collect();
            let (removed, owners) = folders.purge(actor, folder_ids, &occupied);
            let folders_by_owner: Vec<_> = owners
                .iter()
                .map(|owner| (owner.clone(), folders.folders_of(owner.as_deref())))
                .collect();
            (removed, folders_by_owner)
        };
        if !removed.is_empty() {
            self.persist_trash(&[], None, Vec::new(), folders_by_owner, false);
        }
        removed
    }

    /// Admit a permanent deletion of trashed folders for `actor`.
    pub(crate) fn check_trashed_folders(
        &self,
        actor: &FolderActor,
        folder_ids: &[String],
    ) -> Result<(), String> {
        self.inner.folders.lock().check_trashed(actor, folder_ids)
    }

    /// Claim a trashed session for permanent deletion: it must still be in
    /// Trash (deleted at `expected_trashed_at_ms` when given) and not already
    /// closing. Marks it closing under the session lock, so a concurrent
    /// restore refuses it. The caller deletes it or releases the claim.
    pub(crate) fn claim_trash_deletion(
        &self,
        session_id: &str,
        expected_trashed_at_ms: Option<i64>,
    ) -> bool {
        let claimed = {
            let mut sessions = self.inner.sessions.lock();
            match sessions.get_mut(session_id) {
                Some(session)
                    if !session.meta.closing
                        && session.meta.trashed_at_ms.is_some()
                        && expected_trashed_at_ms
                            .is_none_or(|at| session.meta.trashed_at_ms == Some(at)) =>
                {
                    session.meta.closing = true;
                    true
                }
                _ => false,
            }
        };
        if claimed {
            self.broadcast_sessions();
        }
        claimed
    }

    /// A session's explicit folder placement, if it is filed in a folder.
    pub(crate) fn session_folder_placement(&self, session_id: &str) -> Option<String> {
        self.inner
            .folders
            .lock()
            .placement_of(session_id)
            .filter(|folder| !folder.is_empty())
            .map(str::to_owned)
    }

    /// Every trashed session (id, owner, trashed-at ms) and trashed folder,
    /// for the retention sweep.
    pub(crate) fn trash_contents(
        &self,
    ) -> (
        Vec<TrashedSession>,
        Vec<crate::session_folders::SessionFolder>,
    ) {
        let sessions = self
            .inner
            .sessions
            .lock()
            .values()
            .filter_map(|session| {
                session.meta.trashed_at_ms.map(|at| {
                    (
                        session.meta.id.clone(),
                        session.meta.owner_user_id.clone(),
                        at,
                    )
                })
            })
            .collect();
        (sessions, self.inner.folders.lock().trashed())
    }

    fn persist_trash(
        &self,
        sessions: &[String],
        trashed_at_ms: Option<i64>,
        pins: Vec<(String, Option<String>)>,
        folders_by_owner: Vec<(FolderOwner, Vec<crate::session_folders::SessionFolder>)>,
        sessions_first: bool,
    ) {
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let session_write = (!sessions.is_empty()).then(|| StoreWrite::SetSessionsTrashed {
                session_ids: sessions.to_vec(),
                trashed_at_ms,
            });
            let mut writes: Vec<StoreWrite> = folders_by_owner
                .into_iter()
                .map(
                    |(owner_user_id, folders)| StoreWrite::ReplaceSessionFolders {
                        owner_user_id,
                        folders,
                    },
                )
                .collect();
            if !pins.is_empty() {
                writes.push(StoreWrite::UpdateSessionPlacement { placements: pins });
            }
            if let Some(write) = session_write {
                if sessions_first {
                    writes.insert(0, write);
                } else {
                    writes.push(write);
                }
            }
            for write in writes {
                let _ = tx.send(write);
            }
        }
        self.sync_broadcast("folders", Vec::new());
        if !sessions.is_empty() {
            self.broadcast_sessions();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::SessionOrigin;
    use serde_json::json;

    fn hub(sessions: &[&str]) -> Hub {
        let hub = Hub::new();
        for id in sessions {
            hub.create_local_session(
                (*id).to_owned(),
                "claude-code".to_owned(),
                "/tmp".to_owned(),
                (*id).to_owned(),
                SessionOrigin::Web,
                false,
            );
        }
        hub
    }

    fn folders(hub: &Hub, mutation: &str, id: &str, args: serde_json::Value) {
        hub.sync_apply("folders", id.to_owned(), mutation, &args)
            .unwrap();
    }

    fn placement(hub: &Hub, session: &str) -> serde_json::Value {
        hub.sync_value("folders")["placement"][session].clone()
    }

    fn ids(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn a_folder_takes_its_subtree_and_restores_in_place() {
        let hub = hub(&["filed", "nested", "outside"]);
        folders(
            &hub,
            "create",
            "c1",
            json!({"id": "f-a", "name": "A", "parent": null}),
        );
        folders(
            &hub,
            "create",
            "c2",
            json!({"id": "f-b", "name": "B", "parent": "f-a"}),
        );
        folders(
            &hub,
            "place",
            "p1",
            json!({"session_ids": ["filed"], "folder": "f-a"}),
        );
        folders(
            &hub,
            "place",
            "p2",
            json!({"session_ids": ["nested"], "folder": "f-b"}),
        );
        let actor = FolderActor::local();

        let trashed = hub.trash_items(&actor, &ids(&["f-a"]), &[], 1_000).unwrap();
        assert_eq!(trashed.folders, ids(&["f-a", "f-b"]));
        let mut sessions = trashed.sessions.clone();
        sessions.sort();
        assert_eq!(sessions, ids(&["filed", "nested"]));
        assert!(hub.session_is_trashed("filed"));
        assert!(!hub.session_is_trashed("outside"));
        assert_eq!(
            hub.sync_value("folders")["folders"][0]["trashed_at_ms"],
            1_000
        );
        // A trashed folder accepts nothing new.
        assert!(
            hub.sync_apply(
                "folders",
                "p3".into(),
                "place",
                &json!({"session_ids": ["outside"], "folder": "f-a"})
            )
            .is_err()
        );

        let restored = hub.restore_items(&actor, &ids(&["f-a"]), &[]).unwrap();
        assert_eq!(restored.folders, ids(&["f-a", "f-b"]));
        assert!(!hub.session_is_trashed("filed") && !hub.session_is_trashed("nested"));
        assert_eq!(placement(&hub, "nested"), "f-b");
        assert!(
            hub.sync_value("folders")["folders"][0]
                .get("trashed_at_ms")
                .is_none()
        );
    }

    #[test]
    fn a_working_session_refuses_the_whole_move() {
        let hub = hub(&["idle", "busy"]);
        folders(
            &hub,
            "create",
            "c1",
            json!({"id": "f-a", "name": "A", "parent": null}),
        );
        folders(
            &hub,
            "place",
            "p1",
            json!({"session_ids": ["idle", "busy"], "folder": "f-a"}),
        );
        hub.set_status("busy", Status::Busy, None);
        let error = hub
            .trash_items(&FolderActor::local(), &ids(&["f-a"]), &[], 1)
            .unwrap_err();
        assert!(error.contains("\"busy\" is working"), "{error}");
        assert!(!hub.session_is_trashed("idle"));
        assert!(
            hub.sync_value("folders")["folders"][0]
                .get("trashed_at_ms")
                .is_none()
        );
    }

    #[test]
    fn a_session_restores_to_the_nearest_live_folder() {
        let hub = hub(&["s"]);
        folders(
            &hub,
            "create",
            "c1",
            json!({"id": "f-a", "name": "A", "parent": null}),
        );
        folders(
            &hub,
            "create",
            "c2",
            json!({"id": "f-b", "name": "B", "parent": "f-a"}),
        );
        folders(
            &hub,
            "place",
            "p1",
            json!({"session_ids": ["s"], "folder": "f-b"}),
        );
        let actor = FolderActor::local();
        hub.trash_items(&actor, &ids(&["f-b"]), &[], 5).unwrap();
        let restored = hub.restore_items(&actor, &[], &ids(&["s"])).unwrap();
        assert_eq!(restored.sessions, ids(&["s"]));
        assert_eq!(placement(&hub, "s"), "f-a");
        // The emptied folder stays in Trash until it is purged.
        assert_eq!(
            hub.purge_trashed_folders(&actor, &ids(&["f-b"])),
            ids(&["f-b"])
        );
        assert_eq!(
            hub.sync_value("folders")["folders"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn purge_waits_for_sessions_still_in_a_folder() {
        let hub = hub(&["s"]);
        folders(
            &hub,
            "create",
            "c1",
            json!({"id": "f-a", "name": "A", "parent": null}),
        );
        folders(
            &hub,
            "place",
            "p1",
            json!({"session_ids": ["s"], "folder": "f-a"}),
        );
        let actor = FolderActor::local();
        hub.trash_items(&actor, &ids(&["f-a"]), &[], 5).unwrap();
        assert!(hub.purge_trashed_folders(&actor, &ids(&["f-a"])).is_empty());
        assert!(hub.delete_session("s"));
        assert_eq!(
            hub.purge_trashed_folders(&actor, &ids(&["f-a"])),
            ids(&["f-a"])
        );
    }

    #[test]
    fn an_unfiled_session_keeps_following_its_project() {
        let hub = hub(&["s"]);
        let actor = FolderActor::local();
        hub.trash_items(&actor, &[], &ids(&["s"]), 9).unwrap();
        assert!(placement(&hub, "s").is_null());
        let (sessions, _) = hub.trash_contents();
        assert_eq!(sessions, vec![("s".to_owned(), None, 9)]);
        hub.restore_items(&actor, &[], &ids(&["s"])).unwrap();
        assert!(placement(&hub, "s").is_null());
        assert!(hub.restore_items(&actor, &[], &ids(&["s"])).is_err());
    }

    #[test]
    fn a_claimed_deletion_refuses_restore_and_a_stale_claim_is_skipped() {
        let hub = hub(&["s", "t"]);
        let actor = FolderActor::local();
        hub.trash_items(&actor, &[], &ids(&["s", "t"]), 10).unwrap();
        // Judged at time 10, restored and trashed again at 20: the old
        // judgement can no longer claim it.
        hub.restore_items(&actor, &[], &ids(&["t"])).unwrap();
        hub.trash_items(&actor, &[], &ids(&["t"]), 20).unwrap();
        assert!(!hub.claim_trash_deletion("t", Some(10)));
        assert!(hub.claim_trash_deletion("s", Some(10)));
        assert!(!hub.claim_trash_deletion("s", None));
        let error = hub.restore_items(&actor, &[], &ids(&["s"])).unwrap_err();
        assert!(error.contains("being deleted"), "{error}");
        assert!(hub.session_is_trashed("s"));
    }
}
