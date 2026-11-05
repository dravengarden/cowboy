//! Scoped configuration edits use the same typed truth and write-behind queue
//! as browser sync. Receipts acknowledge acceptance, never DB commitment.
use super::Hub;
use crate::review_directory::{Change, override_value, parse_change};
use crate::session_folders::{FolderActor, defaults::DefaultKey};
use serde_json::json;

#[derive(Default)]
pub(super) struct Receipts {
    entries: std::collections::VecDeque<(String, String)>,
}

impl Receipts {
    fn get(&mut self, operation: &str) -> Option<String> {
        let index = self.entries.iter().position(|(id, _)| id == operation)?;
        let entry = self.entries.remove(index)?;
        let value = entry.1.clone();
        self.entries.push_back(entry);
        Some(value)
    }

    fn insert(&mut self, operation: String, fingerprint: String) {
        if self.entries.len() >= 4096 {
            self.entries.pop_front();
        }
        self.entries.push_back((operation, fingerprint));
    }
}

impl Hub {
    pub fn review_default_receipt(&self, operation: &str) -> bool {
        self.inner
            .review_default_receipts
            .lock()
            .get(operation)
            .is_some()
    }

    pub fn review_directory_update(
        &self,
        actor: &FolderActor,
        session_id: &str,
        operation: &str,
        input: &Change,
    ) -> Result<(), &'static str> {
        if !crate::review_directory::is_scope(&input.scope) {
            return Err("unknown_scope");
        }
        let directory = parse_change(&input.change)?;
        let _guard = self.inner.sync_apply_gate.lock();
        if self.session_info(session_id).is_none_or(|s| {
            s.meta.system || s.meta.closing || s.meta.owner_user_id != actor.user_id
        }) {
            return Err("permission_denied");
        }
        // Both spellings hash as the legacy scope, matching retained receipts.
        let replay = crate::review_directory::Change {
            scope: crate::review_directory::LEGACY_SCOPE.to_owned(),
            folder_id: input.folder_id.clone(),
            expected: input.expected.clone(),
            change: input.change.clone(),
        };
        let fingerprint =
            crate::admin::hex_sha256(&serde_json::to_vec(&replay).map_err(|_| "invalid_argument")?);
        {
            let mut receipts = self.inner.review_default_receipts.lock();
            if let Some(previous) = receipts.get(operation) {
                return if previous == fingerprint {
                    Ok(())
                } else {
                    Err("operation_conflict")
                };
            }
        }
        // Once the bounded fingerprint has expired, the ordinary sync dedupe
        // still knows this operation existed. Never acknowledge different input
        // as a new successful edit merely because its fingerprint was evicted.
        if self
            .inner
            .sync
            .lock()
            .values()
            .any(|entry| entry.seen.contains(&format!("zed:{operation}")))
        {
            return Err("receipt_expired");
        }
        let (state, mutation, args, current) = if let Some(id) = &input.folder_id {
            // resolve_defaults enforces the folder's owner, including ancestors.
            self.folder_defaults(actor, id).map_err(|_| "not_found")?;
            let folders = self
                .inner
                .folders
                .lock()
                .folders_of(actor.user_id.as_deref());
            let folder = folders.iter().find(|f| f.id == *id).ok_or("not_found")?;
            let current = override_value(folder.defaults.get(&DefaultKey::ReviewDirectory));
            let args = match &directory {
                Some(value) => json!({"id":id,"values":{"review_directory":value}}),
                None => json!({"id":id,"values":{},"inherit":["review_directory"]}),
            };
            ("folders".to_owned(), "defaults", args, current)
        } else {
            let sessions = self.inner.sessions.lock();
            let session = sessions
                .get(session_id)
                .filter(|s| {
                    !s.meta.system && !s.meta.closing && s.meta.owner_user_id == actor.user_id
                })
                .ok_or("permission_denied")?;
            (
                format!("mobile-review:{session_id}"),
                "setDirectory",
                input.change.clone(),
                override_value(session.mobile_review.directory.as_ref()),
            )
        };
        if current != input.expected {
            return Err("revision_conflict");
        }
        self.sync_apply_as_locked(actor, &state, format!("zed:{operation}"), mutation, &args)
            .map_err(|_| "invalid_argument")?;
        self.inner
            .review_default_receipts
            .lock()
            .insert(operation.to_owned(), fingerprint);
        Ok(())
    }
}

impl Hub {
    /// Whether Review may run the worktree's configured language servers for
    /// this session: the owner's "Project language servers" directory default,
    /// inherited live like the Review directory. Unset or null means on, the
    /// visible migrated default; only an explicit `"off"` restricts. An unknown
    /// session is restricted.
    #[must_use]
    pub fn language_servers_allowed(&self, session_id: &str) -> bool {
        let sessions = self.inner.sessions.lock();
        let Some(session) = sessions.get(session_id) else {
            return false;
        };
        let actor = FolderActor {
            user_id: session.meta.owner_user_id.clone(),
            sees_all: false,
        };
        let project = super::session_continuation::sidebar_project(&session.meta);
        let folders = self.inner.folders.lock();
        let Some(folder) = folders.display_folder(&actor, session_id, project) else {
            return true;
        };
        folders
            .resolve_defaults(&actor, &folder)
            .map_or(true, |resolved| {
                !matches!(
                    resolved.values.get(&DefaultKey::LanguageServers),
                    Some(Some(value)) if value == "off"
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::Receipts;
    #[test]
    fn recent_receipts_survive_pressure_and_new_writes_never_lock_out() {
        let mut receipts = Receipts::default();
        for index in 0..4096 {
            receipts.insert(index.to_string(), index.to_string());
        }
        assert_eq!(receipts.get("0").as_deref(), Some("0"));
        receipts.insert("new".into(), "new".into());
        assert_eq!(receipts.get("1"), None);
        assert_eq!(receipts.get("0").as_deref(), Some("0"));
        assert_eq!(receipts.get("new").as_deref(), Some("new"));
        assert_eq!(receipts.entries.len(), 4096);
    }
}
