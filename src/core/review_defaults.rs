//! Scoped configuration edits use the same typed truth and write-behind queue
//! as browser sync. Receipts acknowledge acceptance, never DB commitment.
use super::Hub;
use crate::review_directory::{Change, override_value, parse_change};
use crate::session_folders::{FolderActor, defaults::DefaultKey};
use serde_json::json;

impl Hub {
    pub(crate) fn review_default_receipt(&self, operation: &str) -> bool {
        self.inner
            .review_default_receipts
            .lock()
            .contains_key(operation)
    }

    pub(crate) fn review_directory_update(
        &self,
        actor: &FolderActor,
        session_id: &str,
        operation: &str,
        input: &Change,
    ) -> Result<(), &'static str> {
        if input.scope != "zed" {
            return Err("unknown_scope");
        }
        let directory = parse_change(&input.change)?;
        let _guard = self.inner.sync_apply_gate.lock();
        if self.session_info(session_id).is_none_or(|s| {
            s.meta.system || s.meta.closing || s.meta.owner_user_id != actor.user_id
        }) {
            return Err("permission_denied");
        }
        let fingerprint =
            crate::admin::hex_sha256(&serde_json::to_vec(input).map_err(|_| "invalid_argument")?);
        {
            let receipts = self.inner.review_default_receipts.lock();
            if let Some(previous) = receipts.get(operation) {
                return if previous == &fingerprint {
                    Ok(())
                } else {
                    Err("operation_conflict")
                };
            }
            if receipts.len() >= 4096 {
                return Err("capacity");
            }
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
