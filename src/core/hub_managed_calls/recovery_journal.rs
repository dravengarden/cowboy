//! Atomic, owner-private local-disk journal for unresolved durable recovery
//! parks and provably unsent completion-notice teardown requeues.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use super::types::{
    DeferredDrainActions, JournalUnsentNotice, NoticeClaims, PendingRecoveryPromptItem,
    UndeliveredRequeue,
};
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct JournalRecoveryItem {
    pub text: String,
    pub content: Vec<serde_json::Value>,
    pub cmid: Option<String>,
    pub reason: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct RecoveryJournalData {
    #[serde(default)]
    pub parks: HashMap<String, Vec<JournalRecoveryItem>>,
    #[serde(default)]
    pub unsent_notices: HashMap<String, Vec<JournalUnsentNotice>>,
}

impl RecoveryJournalData {
    fn is_empty(&self) -> bool {
        self.parks.values().all(Vec::is_empty) && self.unsent_notices.values().all(Vec::is_empty)
    }
}

static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

#[cfg(unix)]
fn current_euid() -> u32 {
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        for line in status.lines() {
            if let Some(rest) = line.strip_prefix("Uid:")
                && let Some(euid) = rest.split_whitespace().nth(1)
                && let Ok(parsed) = euid.parse::<u32>()
            {
                return parsed;
            }
        }
    }
    std::os::unix::fs::MetadataExt::uid(
        &std::fs::metadata("/proc/self")
            .unwrap_or_else(|_| std::fs::metadata(".").expect("current dir metadata")),
    )
}

#[cfg(unix)]
fn validate_private_permissions(path: &Path, expect_dir: bool) -> std::io::Result<()> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
    let meta = std::fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("refusing symlink at {}", path.display()),
        ));
    }
    if expect_dir && !meta.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("expected private directory at {}", path.display()),
        ));
    }
    if !expect_dir && !meta.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("expected regular journal file at {}", path.display()),
        ));
    }
    let euid = current_euid();
    if meta.uid() != euid {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!(
                "refusing journal path {} owned by uid {} (expected {})",
                path.display(),
                meta.uid(),
                euid
            ),
        ));
    }
    let mode = meta.permissions().mode() & 0o777;
    if (mode & 0o077) != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!(
                "refusing journal path {} with group/other permissions {:04o}",
                path.display(),
                mode
            ),
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_private_permissions(_path: &Path, _expect_dir: bool) -> std::io::Result<()> {
    Ok(())
}

fn ensure_private_parent_dir(path: &Path) -> std::io::Result<()> {
    let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) else {
        return Ok(());
    };
    if !parent.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt as _;
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent)?;
        }
        #[cfg(not(unix))]
        std::fs::create_dir_all(parent)?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let meta = std::fs::symlink_metadata(parent)?;
        if !meta.file_type().is_symlink() && meta.is_dir() {
            let mode = meta.permissions().mode() & 0o777;
            if (mode & 0o077) != 0 {
                std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
            }
        }
    }
    validate_private_permissions(parent, true)
}

fn sync_parent_dir(path: &Path) -> std::io::Result<()> {
    let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) else {
        return Ok(());
    };
    std::fs::File::open(parent)?.sync_all()
}

pub(super) fn load_recovery_journal(path: &Path) -> std::io::Result<RecoveryJournalData> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty())
        && parent.exists()
    {
        validate_private_permissions(parent, true)?;
    }
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RecoveryJournalData::default());
        }
        Err(error) => return Err(error),
        Ok(_) => validate_private_permissions(path, false)?,
    }
    let raw = std::fs::read_to_string(path)?;
    if let Ok(v2) = serde_json::from_str::<RecoveryJournalData>(&raw)
        && (raw.contains("\"parks\"") || raw.contains("\"unsent_notices\""))
    {
        return Ok(v2);
    }
    let v1 = serde_json::from_str::<HashMap<String, Vec<JournalRecoveryItem>>>(&raw)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    Ok(RecoveryJournalData {
        parks: v1,
        unsent_notices: HashMap::new(),
    })
}

pub(super) fn save_recovery_journal(
    path: &Path,
    data: &RecoveryJournalData,
) -> std::io::Result<()> {
    if data.is_empty() {
        match std::fs::remove_file(path) {
            Ok(()) => {
                sync_parent_dir(path)?;
                return Ok(());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        }
    }
    ensure_private_parent_dir(path)?;
    let compact = RecoveryJournalData {
        parks: data
            .parks
            .iter()
            .filter(|(_, items)| !items.is_empty())
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        unsent_notices: data
            .unsent_notices
            .iter()
            .filter(|(_, items)| !items.is_empty())
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    };
    let payload = serde_json::to_vec(&compact).map_err(std::io::Error::other)?;
    let seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp_name = format!(
        ".{}.tmp.{}.{}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("recovery-parks.journal.json"),
        std::process::id(),
        seq
    );
    let tmp_path = path.with_file_name(tmp_name);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        opts.mode(0o600);
    }
    let write_res = (|| -> std::io::Result<()> {
        let mut file = opts.open(&tmp_path)?;
        file.write_all(&payload)?;
        file.sync_all()?;
        Ok(())
    })();
    if let Err(error) = write_res {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(error);
    }
    if let Err(error) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(error);
    }
    sync_parent_dir(path)?;
    Ok(())
}

impl Hub {
    pub(super) fn persist_recovery_journal_locked(&self, claims: &NoticeClaims) -> bool {
        let Some(path) = claims.recovery_journal_path.as_ref() else {
            return true;
        };
        let mut parks: HashMap<String, Vec<JournalRecoveryItem>> = HashMap::new();
        for (sid, items) in &claims.pending_recovery_parks {
            for item in items {
                parks
                    .entry(sid.clone())
                    .or_default()
                    .push(JournalRecoveryItem {
                        text: item.text.clone(),
                        content: item.content.clone(),
                        cmid: item.cmid.clone(),
                        reason: item.reason.clone(),
                    });
            }
        }
        for (sid, items) in &claims.staged_batch_parks {
            let entry = parks.entry(sid.clone()).or_default();
            for item in items {
                let dup = item
                    .cmid
                    .as_deref()
                    .is_some_and(|c| entry.iter().any(|e| e.cmid.as_deref() == Some(c)));
                if !dup {
                    entry.push(item.clone());
                }
            }
        }
        let data = RecoveryJournalData {
            parks,
            unsent_notices: claims.pending_unsent_notices.clone(),
        };
        match save_recovery_journal(path, &data) {
            Ok(()) => true,
            Err(error) => {
                tracing::error!(
                    path = %path.display(),
                    %error,
                    "failed to persist durable recovery journal"
                );
                false
            }
        }
    }

    pub fn stage_recovery_journal_batch(
        &self,
        session_id: &str,
        items: &[JournalRecoveryItem],
    ) -> bool {
        if items.is_empty() {
            return true;
        }
        let mut sessions = self.inner.sessions.lock();
        let mut claims = self.inner.notice_claims.lock();
        if let Some(s) = sessions.get_mut(session_id) {
            for item in items {
                if let Some(c) = item.cmid.as_deref().filter(|c| !c.is_empty()) {
                    s.pending_recovery_cmids.insert(c.to_owned());
                }
            }
        }
        let prev_len = {
            let entry = claims
                .staged_batch_parks
                .entry(session_id.to_owned())
                .or_default();
            let prev_len = entry.len();
            for item in items {
                let dup = item
                    .cmid
                    .as_deref()
                    .is_some_and(|c| entry.iter().any(|e| e.cmid.as_deref() == Some(c)));
                if !dup {
                    entry.push(item.clone());
                }
            }
            prev_len
        };
        if !self.persist_recovery_journal_locked(&claims) {
            if let Some(entry) = claims.staged_batch_parks.get_mut(session_id) {
                entry.truncate(prev_len);
                if entry.is_empty() {
                    claims.staged_batch_parks.remove(session_id);
                }
            }
            return false;
        }
        true
    }

    pub fn prune_recovery_journal_batch(&self, session_id: &str, items: &[JournalRecoveryItem]) {
        if items.is_empty() {
            return;
        }
        let mut claims = self.inner.notice_claims.lock();
        if let Some(list) = claims.staged_batch_parks.get_mut(session_id) {
            for target in items {
                if let Some(pos) = list.iter().position(|existing| {
                    target
                        .cmid
                        .as_deref()
                        .map_or(existing == target, |c| existing.cmid.as_deref() == Some(c))
                }) {
                    list.remove(pos);
                }
            }
            if list.is_empty() {
                claims.staged_batch_parks.remove(session_id);
            }
            let _ = self.persist_recovery_journal_locked(&claims);
        }
    }

    pub fn stage_unsent_notices_journal(&self, items: &[JournalUnsentNotice]) -> bool {
        if items.is_empty() {
            return true;
        }
        let mut claims = self.inner.notice_claims.lock();
        let mut added: Vec<(String, String)> = Vec::new();
        for item in items {
            let list = claims
                .pending_unsent_notices
                .entry(item.session_id.clone())
                .or_default();
            if !list.iter().any(|e| e.command_id == item.command_id) {
                list.push(item.clone());
                added.push((item.session_id.clone(), item.command_id.clone()));
            }
        }
        if !self.persist_recovery_journal_locked(&claims) {
            for (sid, cmd_id) in added {
                if let Some(list) = claims.pending_unsent_notices.get_mut(&sid) {
                    list.retain(|e| e.command_id != cmd_id);
                    if list.is_empty() {
                        claims.pending_unsent_notices.remove(&sid);
                    }
                }
            }
            return false;
        }
        true
    }

    pub fn prune_unsent_notice_journal(&self, session_id: &str, command_id: &str) {
        let mut claims = self.inner.notice_claims.lock();
        if let Some(list) = claims.pending_unsent_notices.get_mut(session_id) {
            let before = list.len();
            list.retain(|e| e.command_id != command_id);
            let after = list.len();
            if after == 0 {
                claims.pending_unsent_notices.remove(session_id);
            }
            if before != after {
                let _ = self.persist_recovery_journal_locked(&claims);
            }
        }
    }

    /// Load the full recovery journal atomically BEFORE enabling rewrites or
    /// spawning workers (Finding 3). Corrupt or unreadable files are preserved untouched.
    pub fn attach_and_reconcile_recovery_journal(&self, path: std::path::PathBuf) {
        let loaded = match load_recovery_journal(&path) {
            Ok(data) => data,
            Err(error) => {
                tracing::error!(
                    path = %path.display(),
                    %error,
                    "preserving unreadable or invalid recovery journal without modification"
                );
                return;
            }
        };
        let (park_sessions, unsent_notices) = {
            let mut sessions = self.inner.sessions.lock();
            let mut claims = self.inner.notice_claims.lock();
            let mut to_spawn_parks = Vec::new();
            for (session_id, items) in loaded.parks {
                let Some(s) = sessions.get_mut(&session_id) else {
                    continue;
                };
                for item in items {
                    let already_settled = item.cmid.as_deref().is_some_and(|c| {
                        s.committed_recovery_cmids.contains(c)
                            || s.drafts.iter().any(|m| m.cmid.as_deref() == Some(c))
                    });
                    if already_settled {
                        continue;
                    }
                    if let Some(ref c) = item.cmid
                        && !c.is_empty()
                    {
                        s.pending_recovery_cmids.insert(c.clone());
                    }
                    let queue = claims
                        .pending_recovery_parks
                        .entry(session_id.clone())
                        .or_default();
                    let already_queued = item
                        .cmid
                        .as_deref()
                        .is_some_and(|c| queue.iter().any(|q| q.cmid.as_deref() == Some(c)));
                    if !already_queued {
                        queue.push(PendingRecoveryPromptItem {
                            text: item.text,
                            content: item.content,
                            cmid: item.cmid,
                            reason: item.reason,
                            on_committed: None,
                        });
                    }
                    if claims
                        .active_recovery_park_workers
                        .insert(session_id.clone())
                    {
                        to_spawn_parks.push(session_id.clone());
                    }
                }
            }
            let mut surviving_notices = Vec::new();
            for (session_id, items) in loaded.unsent_notices {
                if !sessions.contains_key(&session_id) {
                    continue;
                }
                let list = claims
                    .pending_unsent_notices
                    .entry(session_id.clone())
                    .or_default();
                for item in items {
                    if !list.iter().any(|e| e.command_id == item.command_id) {
                        list.push(item.clone());
                        surviving_notices.push(item);
                    }
                }
            }
            claims.recovery_journal_path = Some(path);
            let _ = self.persist_recovery_journal_locked(&claims);
            (to_spawn_parks, surviving_notices)
        };
        for sid in park_sessions {
            if self.has_store_writer() {
                let hub = self.clone();
                self.spawn_teardown_requeue(async move {
                    hub.run_durable_recovery_park_loop(sid).await;
                });
            }
        }
        self.reconcile_journal_unsent_notices(unsent_notices);
    }

    fn reconcile_journal_unsent_notices(&self, unsent_notices: Vec<JournalUnsentNotice>) {
        if unsent_notices.is_empty() || !self.has_store_writer() {
            return;
        }
        let hub = self.clone();
        self.spawn_teardown_requeue(async move {
            for item in unsent_notices {
                let Some(_commit_guard) = hub
                    .await_with_shutdown(hub.pending_commit_lock(&item.session_id))
                    .await
                else {
                    break;
                };
                let req = NoticeRequeueRequest {
                    session_id: item.session_id.clone(),
                    text: item.text,
                    content: item.content,
                    cmid: item.completion_id,
                    expected_command_id: Some(item.command_id.clone()),
                    expected_worker_epoch: None,
                    require_worker_epoch_match: false,
                    receipt_key: format!("teardown:{}", item.command_id),
                    require_claim_release: true,
                };
                match hub.preview_and_send_undelivered_notice_requeue(req) {
                    UndeliveredRequeue::NoStore => {
                        hub.prune_unsent_notice_journal(&item.session_id, &item.command_id);
                    }
                    UndeliveredRequeue::Pending(mut preview, rx) => {
                        match hub.await_with_shutdown(rx).await {
                            Some(Ok(Ok(true))) => {
                                hub.commit_undelivered_notice_requeue(&mut preview);
                                hub.prune_unsent_notice_journal(&item.session_id, &item.command_id);
                            }
                            Some(Ok(Ok(false))) => {
                                hub.abort_undelivered_notice_requeue(&mut preview, true);
                                hub.prune_unsent_notice_journal(&item.session_id, &item.command_id);
                            }
                            Some(Ok(Err(_)) | Err(_)) => {
                                hub.abort_undelivered_notice_requeue(&mut preview, false);
                            }
                            None => {
                                hub.abandon_unknown_outcome_notice_requeue(&mut preview);
                                break;
                            }
                        }
                    }
                    UndeliveredRequeue::Rejected(_) => {}
                }
            }
        });
    }

    pub(in crate::core) fn prune_recovery_journal_for_session(&self, session_id: &str) {
        let mut claims = self.inner.notice_claims.lock();
        let a = claims.pending_recovery_parks.remove(session_id).is_some();
        let b = claims.staged_batch_parks.remove(session_id).is_some();
        let c = claims.pending_unsent_notices.remove(session_id).is_some();
        if a || b || c {
            let _ = self.persist_recovery_journal_locked(&claims);
        }
    }

    pub(in crate::core) fn record_deferred_drain_if_fenced(
        &self,
        session_id: &str,
        head_id: Option<&str>,
        manual: bool,
        allow_revive: bool,
        trigger: Option<&crate::prompt_origin::RequestCaller>,
    ) -> bool {
        let mut claims = self.inner.notice_claims.lock();
        let Some(deferred) = claims.in_flight_pending_commits.get_mut(session_id) else {
            return false;
        };
        if let Some(row_id) = head_id {
            if manual && let Some(caller) = trigger {
                deferred.drains.manual_drain = Some((row_id.to_owned(), caller.clone()));
            } else if allow_revive {
                deferred.drains.revive_head = Some(row_id.to_owned());
            }
        }
        true
    }

    pub(in crate::core) fn record_deferred_notice_wake_if_fenced(
        &self,
        session_id: &str,
        completion_id: &str,
    ) -> bool {
        let mut claims = self.inner.notice_claims.lock();
        let Some(deferred) = claims.in_flight_pending_commits.get_mut(session_id) else {
            return false;
        };
        if !deferred
            .drains
            .notice_wakes
            .iter()
            .any(|cid| cid == completion_id)
        {
            deferred.drains.notice_wakes.push(completion_id.to_owned());
        }
        true
    }

    pub(super) fn replay_deferred_drains(&self, session_id: &str, actions: DeferredDrainActions) {
        if let Some((row_id, caller)) = actions.manual_drain {
            let still_head = self
                .inner
                .sessions
                .lock()
                .get(session_id)
                .and_then(|s| s.queue.first())
                .is_some_and(|head| head.id == row_id);
            if still_head {
                self.drain_now(session_id, &caller);
            }
        }
        for cid in actions.notice_wakes {
            let _ = self.wake_queued_call_notice(session_id, &cid);
        }
        if let Some(row_id) = actions.revive_head {
            let still_head = self
                .inner
                .sessions
                .lock()
                .get(session_id)
                .and_then(|s| s.queue.first())
                .is_some_and(|head| head.id == row_id);
            if still_head {
                self.retry_resource_wait(session_id);
            }
        }
        self.try_drain(session_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_roundtrips_with_private_permissions_and_preserves_corrupt_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join("recovery-journal")
            .join("recovery-parks.journal.json");
        assert!(load_recovery_journal(&path).unwrap().is_empty());

        let mut data = RecoveryJournalData::default();
        data.parks.insert(
            "sess-j1".to_owned(),
            vec![JournalRecoveryItem {
                text: "keep me".to_owned(),
                content: vec![serde_json::json!({"type": "text", "text": "keep me"})],
                cmid: Some("cmid-j1".to_owned()),
                reason: "store unavailable".to_owned(),
            }],
        );
        save_recovery_journal(&path, &data).unwrap();
        assert!(path.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                std::fs::metadata(path.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        assert_eq!(load_recovery_journal(&path).unwrap(), data);

        std::fs::write(&path, "{not valid json").unwrap();
        assert!(load_recovery_journal(&path).is_err());
        let hub = Hub::new();
        hub.attach_and_reconcile_recovery_journal(path.clone());
        assert!(path.exists(), "corrupt journal must be preserved on disk");

        save_recovery_journal(&path, &RecoveryJournalData::default()).unwrap();
        assert!(!path.exists());
    }
}
