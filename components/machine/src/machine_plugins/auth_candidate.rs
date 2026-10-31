//! Cleanup ownership follows a native login through connection handoff.
use super::MachinePluginStore;
use anyhow::{Context as _, Result, ensure};
use std::fs;
use std::io::Read as _;
use std::path::Path;
use std::sync::Arc;

const COMPLETED_FILE: &str = "native-completed";
const COMPLETED_V1: &[u8] = b"cowboy-native-login-completed-v1\n";
const TREE_EXIT_FILE: &str = "process-tree-exited";
const TREE_EXIT_V1: &[u8] = b"cowboy-login-tree-exited-v1\n";
const BOOT_FILE: &str = "native-boot";
const BOOT_V1: &str = "cowboy-native-login-boot-v1\n";
mod stops;

fn current_boot_id() -> Option<uuid::Uuid> {
    #[cfg(target_os = "linux")]
    {
        let value = fs::read_to_string("/proc/sys/kernel/random/boot_id").ok()?;
        canonical_boot_id(value.trim())
    }
    #[cfg(not(target_os = "linux"))]
    None
}

fn canonical_boot_id(value: &str) -> Option<uuid::Uuid> {
    let id = uuid::Uuid::parse_str(value).ok()?;
    (!id.is_nil() && id.to_string() == value).then_some(id)
}

impl MachinePluginStore {
    /// Called only after pre-exec containment has observed an empty subtree.
    pub fn mark_auth_candidate_tree_exited(&self, provider: &str, request: &str) -> Result<()> {
        let _guard = self.auth_candidates.lock();
        super::validate_plugin_id(provider)?;
        super::validate_auth_candidate_id(request)?;
        ensure!(
            self.auth_candidate_stop(provider, request)?.is_none(),
            "login already stopped"
        );
        let root = self.auth_candidate_root(provider);
        let candidate = root.join(request);
        ensure!(
            real_directory(&root) && real_directory(&candidate),
            "login candidate missing or linked"
        );
        super::atomic_write(&candidate.join(TREE_EXIT_FILE), TREE_EXIT_V1, 0o600)?;
        fs::File::open(&candidate)?.sync_all()?;
        fs::File::open(&root)?.sync_all()?;
        Ok(())
    }

    fn auth_candidate_tree_exited(&self, provider: &str, request: &str) -> Result<bool> {
        Ok(read_marker(
            &self
                .auth_candidate_root(provider)
                .join(request)
                .join(TREE_EXIT_FILE),
            TREE_EXIT_V1.len(),
        )?
        .as_deref()
            == Some(TREE_EXIT_V1))
    }

    /// Own named secret-input lifetime without storing the supplied secret.
    /// This path never spawns a command, including if its future is aborted.
    pub async fn with_secret_input<T, F>(
        self: &Arc<Self>,
        provider: &str,
        request: &str,
        action: impl FnOnce() -> F,
    ) -> Result<T>
    where
        F: std::future::Future<Output = Result<T>>,
    {
        if self.account_id.is_default() {
            return action().await;
        }
        crate::provider_account::portable_credentials_allowed(&self.account_id, provider)
            .map_err(anyhow::Error::msg)?;
        self.create_auth_candidate_home(provider, request)?;
        let mut cleanup = AuthCandidateCleanup::new(self.clone(), provider, request);
        let result = action().await;
        cleanup.discard()?;
        result
    }

    /// A disconnected request is not evidence of process exit. Finalization
    /// after reconnect must use the durable completion marker, not absence
    /// from the in-memory request registry. Missing homes remain idempotent.
    pub(super) fn ensure_auth_candidate_completed(
        &self,
        provider: &str,
        request: &str,
    ) -> Result<bool> {
        super::validate_plugin_id(provider)?;
        super::validate_auth_candidate_id(request)?;
        let root = self.auth_candidate_root(provider);
        let candidate = root.join(request);
        match fs::symlink_metadata(&candidate) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            result => ensure!(result?.is_dir(), "invalid native login candidate"),
        }
        ensure!(real_directory(&root), "invalid native login candidate root");
        ensure!(
            completed(&candidate.join(COMPLETED_FILE))?,
            "native login process completion is unconfirmed"
        );
        Ok(true)
    }

    /// Preserve completion evidence before deleting its last source marker.
    /// A legacy missing home remains idempotent but cannot invent a receipt.
    pub(super) fn finalize_completed_auth_candidate(
        &self,
        provider: &str,
        request: &str,
    ) -> Result<()> {
        if let Some(receipt) = self.auth_candidate_stop(provider, request)? {
            return self.discard_stopped_auth_candidate(provider, request, receipt.process_started);
        }
        if self.ensure_auth_candidate_completed(provider, request)? {
            self.discard_stopped_auth_candidate(provider, request, true)?;
        }
        Ok(())
    }

    /// Serialize scope inspection and creation across store clones. A retained
    /// home is an unresolved native login, not permission to start another one.
    pub(super) fn create_auth_candidate_home(
        &self,
        provider: &str,
        request: &str,
    ) -> Result<std::path::PathBuf> {
        let _guard = self.auth_candidates.lock();
        super::validate_plugin_id(provider)?;
        super::validate_auth_candidate_id(request)?;
        let root = self.auth_candidate_root(provider);
        ensure!(
            self.auth_candidate_stop(provider, request)?.is_none(),
            "native login request has already stopped"
        );
        for directory in [self.auth_provider_root(provider), root.clone()] {
            crate::provider_account::prepare_private_directory(&directory)?;
        }
        ensure!(
            fs::read_dir(&root)?.next().transpose()?.is_none(),
            "Provider account has an active or unresolved native login on this Machine; reconcile its temporary home before starting another authorization"
        );
        let candidate = root.join(request);
        crate::provider_account::prepare_private_directory(&candidate)?;
        let home = candidate.join("home");
        crate::provider_account::prepare_private_directory(&home)?;
        self.record_auth_candidate_boot(provider, request)?;
        Ok(home)
    }

    /// Record before admitting any native process. Unsupported hosts retain
    /// unknown crash remnants rather than infer process death from timestamps.
    pub(super) fn record_auth_candidate_boot(&self, provider: &str, request: &str) -> Result<()> {
        if let Some(boot) = current_boot_id() {
            let candidate = self.auth_candidate_root(provider).join(request);
            super::atomic_write(
                &candidate.join(BOOT_FILE),
                format!("{BOOT_V1}{boot}\n").as_bytes(),
                0o600,
            )?;
            fs::File::open(&candidate)?.sync_all()?;
            fs::File::open(self.auth_candidate_root(provider))?.sync_all()?;
        }
        Ok(())
    }

    /// Only call after the native process has exited and its process-group
    /// cleanup guard has run. This contains no credential or account identity.
    pub fn mark_auth_candidate_completed(&self, provider: &str, request: &str) -> Result<()> {
        super::validate_plugin_id(provider)?;
        super::validate_auth_candidate_id(request)?;
        let root = self.auth_candidate_root(provider);
        let candidate = root.join(request);
        ensure!(
            real_directory(&root) && real_directory(&candidate),
            "login candidate directory is missing or linked"
        );
        super::atomic_write(&candidate.join(COMPLETED_FILE), COMPLETED_V1, 0o600)?;
        fs::File::open(&candidate)?.sync_all()?;
        fs::File::open(&root)?.sync_all()?;
        Ok(())
    }

    /// Constructor-only recovery, under the exclusive Machine journal owner.
    /// An old timestamp does not prove a process exited. Unknown/legacy homes
    /// remain untouched, even if they are older than a native login deadline.
    pub(super) fn recover_completed_auth_candidates(&self) -> Result<()> {
        let boot = current_boot_id();
        for scope in self.existing_auth_scopes() {
            let providers = scope.auth_watch_root();
            if !real_directory(&providers) {
                continue;
            }
            for provider in fs::read_dir(providers)? {
                let provider = provider?;
                let name = provider.file_name().to_string_lossy().into_owned();
                if !provider.file_type()?.is_dir() || super::validate_plugin_id(&name).is_err() {
                    continue;
                }
                scope.recover_auth_candidate_stops(&name)?;
                let candidates = provider.path().join("candidates");
                if !real_directory(&candidates) {
                    continue;
                }
                for candidate in fs::read_dir(candidates)? {
                    let candidate = candidate?;
                    let request = candidate.file_name().to_string_lossy().into_owned();
                    if !candidate.file_type()?.is_dir()
                        || super::validate_auth_candidate_id(&request).is_err()
                    {
                        continue;
                    }
                    if let Some(receipt) = scope.auth_candidate_stop(&name, &request)? {
                        scope.discard_stopped_auth_candidate(
                            &name,
                            &request,
                            receipt.process_started,
                        )?;
                        continue;
                    }
                    if scope.auth_candidate_tree_exited(&name, &request)? {
                        scope.discard_stopped_auth_candidate(&name, &request, true)?;
                        continue;
                    }
                    let finished = completed(&candidate.path().join(COMPLETED_FILE))?;
                    let previous_boot = from_previous_boot(&candidate.path(), boot)?;
                    if finished || previous_boot {
                        if finished {
                            scope.finalize_completed_auth_candidate(&name, &request)?;
                        } else {
                            // A previous boot proves no surviving process, but
                            // not whether this request ever spawned a command.
                            scope.discard_auth_candidate(&name, &request)?;
                        }
                        tracing::info!(provider = %name, request_id = %request, account_id = %scope.account_id.as_str(), previous_boot, "recovered login temporary home");
                    } else {
                        tracing::warn!(provider = %name, request_id = %request, account_id = %scope.account_id.as_str(), "retaining login temporary home without native completion evidence");
                    }
                }
            }
        }
        Ok(())
    }
}

fn real_directory(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.is_dir())
}

fn completed(path: &Path) -> Result<bool> {
    Ok(read_marker(path, COMPLETED_V1.len())?.as_deref() == Some(COMPLETED_V1))
}

fn from_previous_boot(candidate: &Path, current: Option<uuid::Uuid>) -> Result<bool> {
    let Some(current) = current else {
        return Ok(false);
    };
    let Some(bytes) = read_marker(&candidate.join(BOOT_FILE), BOOT_V1.len() + 37)? else {
        return Ok(false);
    };
    let recorded = std::str::from_utf8(&bytes)
        .ok()
        .and_then(|text| text.strip_prefix(BOOT_V1))
        .and_then(|text| text.strip_suffix('\n'))
        .and_then(canonical_boot_id);
    Ok(recorded.is_some_and(|recorded| recorded != current))
}

fn read_marker(path: &Path, maximum: usize) -> Result<Option<Vec<u8>>> {
    use rustix::fs::{Mode, OFlags, open};
    let fd = match open(
        path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(rustix::io::Errno::NOENT | rustix::io::Errno::LOOP) => return Ok(None),
        Err(error) => return Err(error).context("opening native login completion evidence"),
    };
    let file = fs::File::from(fd);
    if !file.metadata()?.is_file() {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    file.take((maximum + 1) as u64).read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

pub struct AuthCandidateCleanup {
    store: Arc<MachinePluginStore>,
    provider: String,
    request: String,
    armed: bool,
    process_unconfirmed: bool,
    process_started: bool,
}

impl AuthCandidateCleanup {
    pub fn new(store: Arc<MachinePluginStore>, provider: &str, request: &str) -> Self {
        Self {
            store,
            provider: provider.into(),
            request: request.into(),
            armed: true,
            process_unconfirmed: false,
            process_started: false,
        }
    }

    pub fn discard(&mut self) -> anyhow::Result<()> {
        ensure!(
            !self.process_unconfirmed,
            "native login process exit is unconfirmed; retaining its reservation"
        );
        if self.armed {
            self.store.discard_stopped_auth_candidate(
                &self.provider,
                &self.request,
                self.process_started,
            )?;
            self.armed = false;
        }
        Ok(())
    }

    /// Set immediately after spawn, before the next await. If the task is
    /// aborted, Drop must retain the home and its exclusive login reservation.
    pub fn process_started(&mut self) {
        self.process_unconfirmed = true;
        self.process_started = true;
    }

    /// Only the process runner's confirmed exit may release the reservation.
    pub fn process_exited(&mut self) {
        self.process_unconfirmed = false;
    }

    /// Move cleanup responsibility without a moment in which it is unowned.
    #[must_use]
    pub fn transfer(&mut self) -> Self {
        let guard = Self {
            store: self.store.clone(),
            provider: self.provider.clone(),
            request: self.request.clone(),
            armed: self.armed,
            process_unconfirmed: self.process_unconfirmed,
            process_started: self.process_started,
        };
        self.armed = false;
        guard
    }

    pub fn matches(
        &self,
        provider: &str,
        account: &crate::provider_account::ProviderAccountId,
    ) -> bool {
        self.provider == provider && self.store.account_id() == account
    }
}

impl Drop for AuthCandidateCleanup {
    fn drop(&mut self) {
        if let Err(error) = self.discard() {
            tracing::warn!(provider = %self.provider, request_id = %self.request, account_id = %self.store.account_id().as_str(), %error, "temporary login credential cleanup failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tree_exit_marker_recovers_failed_cleanup_without_success_marker() {
        let dir = tempfile::tempdir().unwrap();
        let root = MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap();
        let id = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let store = Arc::new(root.for_account(&id).unwrap());
        let home = store.create_auth_candidate_home("grok", "request").unwrap();
        let mut guard = AuthCandidateCleanup::new(store.clone(), "grok", "request");
        guard.process_started();
        store
            .mark_auth_candidate_tree_exited("grok", "request")
            .unwrap();
        guard.process_exited();
        let receipts = store.auth_provider_root("grok").join("login-receipts");
        fs::write(&receipts, "obstruction").unwrap();
        assert!(guard.discard().is_err());
        assert!(home.exists());
        drop(guard);
        fs::remove_file(receipts).unwrap();
        drop(store);
        drop(root);
        let root = MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap();
        let store = root.for_account(&id).unwrap();
        let receipt = store
            .auth_candidate_stop("grok", "request")
            .unwrap()
            .unwrap();
        assert!(receipt.process_tree_exited && receipt.leader_exited && receipt.candidate_removed);
        assert!(!home.exists());
        assert!(
            store
                .mark_auth_candidate_tree_exited("grok", "request")
                .is_err()
        );
        store
            .discard_stopped_auth_candidate("grok", "request", true)
            .unwrap();
        assert_eq!(
            store
                .auth_candidate_stop("grok", "request")
                .unwrap()
                .unwrap(),
            receipt
        );
    }

    #[test]
    fn malformed_tree_marker_cannot_upgrade_legacy_completion() {
        let dir = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap();
        let path = candidate(&store);
        fs::write(
            path.join(TREE_EXIT_FILE),
            b"cowboy-login-tree-exited-v1\nextra",
        )
        .unwrap();
        store
            .mark_auth_candidate_completed("claude-code", "request")
            .unwrap();
        store
            .finalize_completed_auth_candidate("claude-code", "request")
            .unwrap();
        assert!(
            !store
                .auth_candidate_stop("claude-code", "request")
                .unwrap()
                .unwrap()
                .process_tree_exited
        );
    }
    use crate::machine_protocol::Platform;

    fn candidate(store: &MachinePluginStore) -> std::path::PathBuf {
        let path = store.auth_candidate_root("claude-code").join("request");
        std::fs::create_dir_all(path.join("home")).unwrap();
        std::fs::write(path.join("home/credential"), b"synthetic").unwrap();
        path
    }

    #[test]
    fn named_claude_export_is_rejected_before_reading_native_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap();
        let account = store
            .for_account(&uuid::Uuid::new_v4().to_string().try_into().unwrap())
            .unwrap();
        let path = candidate(&account);
        assert_eq!(
            account
                .export_auth_candidate("claude-code", "native", "request")
                .err()
                .expect("native credentials must not be exported")
                .to_string(),
            "native_owned_credentials_required"
        );
        assert_eq!(
            account
                .auth_candidate_from_secret("claude-code", "native", "synthetic-secret")
                .err()
                .expect("native credentials must not be imported")
                .to_string(),
            "native_owned_credentials_required"
        );
        assert_eq!(
            account
                .prepare_auth_candidate_home("claude-code", "other")
                .unwrap_err()
                .to_string(),
            "native_owned_credentials_required"
        );
        assert_eq!(
            fs::read(path.join("home/credential")).unwrap(),
            b"synthetic"
        );
        assert!(!path.parent().unwrap().join("other").exists());
        let mut failed = std::collections::BTreeSet::new();
        assert!(
            account
                .auth_refresh_candidates_for_provider("claude-code", &mut failed)
                .unwrap()
                .is_empty()
        );
        account
            .discard_auth_candidate("claude-code", "request")
            .unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn concurrent_native_logins_have_one_winner_per_scope() {
        let dir = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let attempts: Vec<_> = (0..2)
            .map(|index| {
                let store = store.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    store.create_auth_candidate_home("claude-code", &format!("request-{index}"))
                })
            })
            .collect();
        let results: Vec<_> = attempts
            .into_iter()
            .map(|task| task.join().unwrap())
            .collect();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        let winner = results.into_iter().find_map(Result::ok).unwrap();
        let request = winner
            .parent()
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap();
        let named = store
            .for_account(&uuid::Uuid::new_v4().to_string().try_into().unwrap())
            .unwrap();
        assert!(
            named
                .create_auth_candidate_home("claude-code", "independent-account")
                .is_ok()
        );
        assert!(
            store
                .create_auth_candidate_home("codex", "independent-provider")
                .is_ok()
        );
        store
            .discard_auth_candidate("claude-code", request)
            .unwrap();
        assert!(
            store
                .create_auth_candidate_home("claude-code", "after-cleanup")
                .is_ok()
        );
    }

    #[test]
    fn restart_retains_and_fences_unknown_login_without_touching_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap();
        let path = candidate(&store);
        drop(store);
        let reopened =
            MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap();
        let error = reopened
            .create_auth_candidate_home("claude-code", "replacement")
            .unwrap_err();
        assert!(error.to_string().contains("unresolved native login"));
        assert_eq!(
            fs::read(path.join("home/credential")).unwrap(),
            b"synthetic"
        );
        assert!(!path.parent().unwrap().join("replacement").exists());
    }

    #[test]
    fn native_login_creation_rejects_linked_roots_and_unknown_entries() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap();
        let provider = store.auth_provider_root("claude-code");
        symlink(outside.path(), &provider).unwrap();
        assert!(
            store
                .create_auth_candidate_home("claude-code", "request")
                .is_err()
        );
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
        fs::remove_file(&provider).unwrap();
        fs::create_dir(&provider).unwrap();
        let root = store.auth_candidate_root("claude-code");
        symlink(outside.path(), &root).unwrap();
        assert!(
            store
                .create_auth_candidate_home("claude-code", "request")
                .is_err()
        );
        fs::remove_file(&root).unwrap();
        fs::create_dir(&root).unwrap();
        fs::write(root.join("unknown"), b"retained").unwrap();
        assert!(
            store
                .create_auth_candidate_home("claude-code", "request")
                .is_err()
        );
        assert_eq!(fs::read(root.join("unknown")).unwrap(), b"retained");
    }

    #[test]
    fn boot_recovery_requires_exact_valid_evidence_and_never_follows_links() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let current = uuid::Uuid::new_v4();
        let previous = uuid::Uuid::new_v4();
        let marker = dir.path().join(BOOT_FILE);
        assert!(!from_previous_boot(dir.path(), Some(current)).unwrap());
        for content in [
            format!("{BOOT_V1}{current}\n"),
            format!("{BOOT_V1}{}\n", uuid::Uuid::nil()),
            format!("{BOOT_V1}{previous}\nextra"),
            format!("{BOOT_V1}{previous}"),
            format!("cowboy-native-login-boot-v2\n{previous}\n"),
            "invalid".into(),
        ] {
            fs::write(&marker, content).unwrap();
            assert!(!from_previous_boot(dir.path(), Some(current)).unwrap());
        }
        fs::write(&marker, format!("{BOOT_V1}{previous}\n")).unwrap();
        assert!(from_previous_boot(dir.path(), Some(current)).unwrap());
        assert!(!from_previous_boot(dir.path(), None).unwrap());
        let target = dir.path().join("target");
        fs::rename(&marker, &target).unwrap();
        symlink(&target, &marker).unwrap();
        assert!(!from_previous_boot(dir.path(), Some(current)).unwrap());
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn startup_recovers_previous_boot_but_retains_same_boot_native_login() {
        let dir = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap();
        let named = store
            .for_account(&uuid::Uuid::new_v4().to_string().try_into().unwrap())
            .unwrap();
        let same_boot = candidate(&store);
        let old_boot = candidate(&named);
        store
            .record_auth_candidate_boot("claude-code", "request")
            .unwrap();
        assert!(current_boot_id().is_some());
        assert!(!from_previous_boot(&same_boot, current_boot_id()).unwrap());
        let previous = uuid::Uuid::new_v4();
        assert_ne!(Some(previous), current_boot_id());
        fs::write(old_boot.join(BOOT_FILE), format!("{BOOT_V1}{previous}\n")).unwrap();
        drop(named);
        drop(store);
        let _reopened =
            MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap();
        assert!(same_boot.join("home/credential").exists());
        assert!(!old_boot.exists());
    }

    #[test]
    fn startup_recovers_completed_homes_across_accounts_and_preserves_unknown_work() {
        let dir = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap();
        let account = store
            .for_account(&uuid::Uuid::new_v4().to_string().try_into().unwrap())
            .unwrap();
        let root_path = candidate(&store);
        let account_path = candidate(&account);
        for scope in [&store, &account] {
            scope
                .mark_auth_candidate_completed("claude-code", "request")
                .unwrap();
        }
        let unknown = store
            .auth_candidate_root("claude-code")
            .join("unknown/home");
        fs::create_dir_all(&unknown).unwrap();
        fs::write(unknown.join("credential"), b"synthetic").unwrap();
        // Age is never sufficient authority to remove possibly running work.
        fs::File::open(unknown.parent().unwrap())
            .unwrap()
            .set_modified(std::time::SystemTime::UNIX_EPOCH)
            .unwrap();
        let replica = account
            .auth_provider_root("claude-code")
            .join("replicas/1.sealed.json");
        fs::create_dir_all(replica.parent().unwrap()).unwrap();
        fs::write(&replica, b"retained-replica").unwrap();
        drop(account);
        drop(store);
        let reopened =
            MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap();
        assert!(!root_path.exists());
        assert!(!account_path.exists());
        let retained = reopened
            .auth_candidate_stop("claude-code", "request")
            .unwrap()
            .unwrap();
        assert!(retained.process_started && retained.leader_exited && retained.candidate_removed);
        assert!(unknown.join("credential").exists());
        assert_eq!(fs::read(replica).unwrap(), b"retained-replica");
        drop(reopened);
        assert!(MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).is_ok());
    }

    #[test]
    fn startup_never_follows_candidate_or_completion_links() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap();
        let path = candidate(&store);
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join(COMPLETED_FILE), COMPLETED_V1).unwrap();
        fs::write(outside.path().join("sentinel"), b"retained").unwrap();
        symlink(
            outside.path().join(COMPLETED_FILE),
            path.join(COMPLETED_FILE),
        )
        .unwrap();
        symlink(
            outside.path(),
            store.auth_candidate_root("claude-code").join("linked"),
        )
        .unwrap();
        symlink(
            outside.path(),
            store.auth_watch_root().join("linked-provider"),
        )
        .unwrap();
        drop(store);
        let reopened =
            MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap();
        assert!(path.join("home/credential").exists());
        assert_eq!(
            fs::read(outside.path().join("sentinel")).unwrap(),
            b"retained"
        );
        // A truncated, extended or unknown version is not completion evidence.
        fs::remove_file(path.join(COMPLETED_FILE)).unwrap();
        for bytes in [
            b"".as_slice(),
            b"cowboy-native-login-completed-v2\n",
            b"cowboy-native-login-completed-v1\nextra",
        ] {
            fs::write(path.join(COMPLETED_FILE), bytes).unwrap();
            assert!(!completed(&path.join(COMPLETED_FILE)).unwrap());
        }
        drop(reopened);
    }

    #[tokio::test]
    async fn dropping_a_login_task_cleans_only_its_account_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let root = Arc::new(
            MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap(),
        );
        let account = root
            .for_account(&uuid::Uuid::new_v4().to_string().try_into().unwrap())
            .unwrap();
        let root_path = candidate(&root);
        let account_path = candidate(&account);
        let guard = AuthCandidateCleanup::new(Arc::new(account), "claude-code", "request");
        let (started, ready) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _guard = guard;
            started.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        ready.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(!account_path.exists());
        assert!(root_path.join("home/credential").is_file());
    }

    #[test]
    fn transfer_retains_candidate_until_connection_owner_drops() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(
            MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap(),
        );
        let path = candidate(&store);
        let mut guard = AuthCandidateCleanup::new(store.clone(), "claude-code", "request");
        let connection_guard = guard.transfer();
        drop(guard);
        assert!(path.exists());
        drop(connection_guard);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn aborted_running_login_retains_reservation_across_restart() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(
            MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap(),
        );
        let home = store
            .create_auth_candidate_home("claude-code", "request")
            .unwrap();
        let mut guard = AuthCandidateCleanup::new(store.clone(), "claude-code", "request");
        guard.process_started();
        assert!(guard.discard().is_err());
        let (started, ready) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let _guard = guard;
            started.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        ready.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(home.is_dir());
        assert!(
            store
                .create_auth_candidate_home("claude-code", "retry")
                .is_err()
        );
        drop(store);
        let reopened =
            MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap();
        assert!(home.is_dir());
        assert!(
            reopened
                .create_auth_candidate_home("claude-code", "retry")
                .is_err()
        );
    }

    #[test]
    fn login_stop_receipt_recovers_cleanup_and_fences_request_reuse() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(
            MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap(),
        );
        let home = store.create_auth_candidate_home("grok", "request").unwrap();
        let mut guard = AuthCandidateCleanup::new(store.clone(), "grok", "request");
        guard.process_started();
        assert!(guard.discard().is_err());
        assert!(
            store
                .auth_candidate_stop("grok", "request")
                .unwrap()
                .is_none()
        );
        guard.process_exited();
        guard.discard().unwrap();
        let receipt = store
            .auth_candidate_stop("grok", "request")
            .unwrap()
            .unwrap();
        assert!(receipt.process_started && receipt.leader_exited && receipt.candidate_removed);
        assert!(
            store
                .auth_candidate_stop("grok", "other")
                .unwrap()
                .is_none()
        );
        assert!(store.create_auth_candidate_home("grok", "request").is_err());
        let receipt_path = store
            .auth_provider_root("grok")
            .join("login-receipts/request.json");
        let mut partial: serde_json::Value =
            serde_json::from_slice(&fs::read(&receipt_path).unwrap()).unwrap();
        partial["candidate_removed"] = false.into();
        fs::write(&receipt_path, serde_json::to_vec(&partial).unwrap()).unwrap();
        fs::write(
            receipt_path
                .parent()
                .unwrap()
                .join(".request.json.1.1.partial"),
            "incomplete",
        )
        .unwrap();
        fs::create_dir_all(&home).unwrap();
        fs::write(home.join("private"), "fixture").unwrap();
        drop(guard);
        drop(store);
        let store = MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap();
        assert!(!home.exists());
        assert!(
            store
                .auth_candidate_stop("grok", "request")
                .unwrap()
                .unwrap()
                .candidate_removed
        );
        assert!(store.create_auth_candidate_home("grok", "request").is_err());
        assert!(
            store
                .create_auth_candidate_home("grok", "new-request")
                .is_ok()
        );
        let id = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let other = store.for_account(&id).unwrap();
        assert!(
            other
                .auth_candidate_stop("grok", "request")
                .unwrap()
                .is_none()
        );
        partial["request"] = "other".into();
        fs::write(receipt_path, serde_json::to_vec(&partial).unwrap()).unwrap();
        assert!(store.auth_candidate_stop("grok", "request").is_err());
    }

    #[test]
    fn login_stop_cleanup_rejects_linked_candidate_root() {
        let dir = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let store = Arc::new(
            MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap(),
        );
        store.create_auth_candidate_home("grok", "request").unwrap();
        let root = store.auth_candidate_root("grok");
        fs::rename(&root, root.with_file_name("retained-candidates")).unwrap();
        fs::create_dir(external.path().join("request")).unwrap();
        let private = external.path().join("request/private");
        fs::write(&private, "must remain").unwrap();
        std::os::unix::fs::symlink(external.path(), &root).unwrap();
        let mut guard = AuthCandidateCleanup::new(store, "grok", "request");
        assert!(guard.discard().is_err());
        assert_eq!(fs::read_to_string(private).unwrap(), "must remain");
    }

    #[test]
    fn login_stop_observation_is_read_only_and_account_scoped() {
        let dir = tempfile::tempdir().unwrap();
        let root = MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap();
        let id = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let store = Arc::new(root.for_account(&id).unwrap());
        let request = crate::provider_account::LoginStopRequest {
            account_id: id,
            provider_id: "grok".into(),
            login_request_id: "request".into(),
        };
        assert!(store.observe_login_stop(&request).unwrap().is_none());
        assert!(
            !store
                .auth_provider_root("grok")
                .join("login-receipts")
                .exists()
        );
        let home = store.create_auth_candidate_home("grok", "request").unwrap();
        let mut guard = AuthCandidateCleanup::new(store.clone(), "grok", "request");
        guard.process_started();
        assert!(store.observe_login_stop(&request).unwrap().is_none());
        assert!(home.exists());
        guard.process_exited();
        guard.discard().unwrap();
        let observation = store.observe_login_stop(&request).unwrap().unwrap();
        assert!(
            observation.process_started
                && observation.leader_exited
                && observation.candidate_removed
        );
        assert_eq!(observation.request, request);
        assert!(root.observe_login_stop(&request).is_err());
        let mut foreign = request.clone();
        foreign.account_id = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        assert!(store.observe_login_stop(&foreign).is_err());
        foreign = request;
        foreign.login_request_id = "unknown".into();
        assert!(store.observe_login_stop(&foreign).unwrap().is_none());
    }

    #[tokio::test]
    async fn secret_input_records_never_spawned_cleanup_on_success_failure_and_abort() {
        let dir = tempfile::tempdir().unwrap();
        let root = MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap();
        let id = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let store = Arc::new(root.for_account(&id).unwrap());
        assert_eq!(
            store
                .with_secret_input("grok", "success", || async { Ok(7) })
                .await
                .unwrap(),
            7
        );
        assert!(
            store
                .with_secret_input::<(), _>("grok", "failed", || async {
                    anyhow::bail!("input cancelled")
                })
                .await
                .is_err()
        );
        let (ready, started) = tokio::sync::oneshot::channel();
        let task_store = store.clone();
        let task = tokio::spawn(async move {
            task_store
                .with_secret_input::<(), _>("grok", "aborted", || async {
                    ready.send(()).unwrap();
                    std::future::pending().await
                })
                .await
        });
        started.await.unwrap();
        assert!(
            store
                .auth_candidate_stop("grok", "aborted")
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .with_secret_input("grok", "overlap", || async { Ok(()) })
                .await
                .is_err()
        );
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        for request in ["success", "failed", "aborted"] {
            let observed = store
                .observe_login_stop(&crate::provider_account::LoginStopRequest {
                    account_id: id.clone(),
                    provider_id: "grok".into(),
                    login_request_id: request.into(),
                })
                .unwrap()
                .unwrap();
            assert!(
                !observed.process_started && !observed.leader_exited && observed.candidate_removed
            );
            assert!(
                store
                    .with_secret_input("grok", request, || async {
                        panic!("stopped request reused");
                        #[allow(unreachable_code)]
                        Ok(())
                    })
                    .await
                    .is_err()
            );
        }
        assert!(
            store
                .with_secret_input("claude-code", "forbidden", || async { Ok(()) })
                .await
                .is_err()
        );
        assert!(!store.auth_candidate_root("claude-code").exists());
    }

    #[tokio::test]
    async fn secret_input_cleanup_failure_does_not_return_a_successful_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let root = MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap();
        let store = Arc::new(
            root.for_account(&uuid::Uuid::new_v4().to_string().try_into().unwrap())
                .unwrap(),
        );
        let receipts = store.auth_provider_root("grok").join("login-receipts");
        let result = store
            .with_secret_input("grok", "failed", || async {
                // The action produced a candidate, but its cleanup journal cannot commit.
                fs::write(&receipts, "obstruction").unwrap();
                Ok("fixture candidate")
            })
            .await;
        assert!(result.is_err());
        assert!(store.auth_candidate_root("grok").join("failed").exists());
        fs::remove_file(receipts).unwrap();
        let mut cleanup = AuthCandidateCleanup::new(store.clone(), "grok", "failed");
        cleanup.discard().unwrap();
        assert!(
            store
                .auth_candidate_stop("grok", "failed")
                .unwrap()
                .unwrap()
                .candidate_removed
        );
    }

    #[test]
    fn confirmed_exit_releases_cleanup_after_unconfirmed_attempt() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(
            MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap(),
        );
        let path = candidate(&store);
        let mut guard = AuthCandidateCleanup::new(store, "claude-code", "request");
        guard.process_started();
        assert!(guard.discard().is_err());
        assert!(path.exists());
        guard.process_exited();
        guard.discard().unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn reconnect_finalization_requires_durable_completion() {
        let dir = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap();
        let path = candidate(&store);
        assert!(
            store
                .finalize_completed_auth_candidate("claude-code", "request")
                .is_err()
        );
        assert!(path.join("home/credential").is_file());
        store
            .mark_auth_candidate_completed("claude-code", "request")
            .unwrap();
        store
            .finalize_completed_auth_candidate("claude-code", "request")
            .unwrap();
        assert!(!path.exists());
        let receipt = store
            .auth_candidate_stop("claude-code", "request")
            .unwrap()
            .unwrap();
        assert!(receipt.process_started && receipt.leader_exited && receipt.candidate_removed);
        store
            .finalize_completed_auth_candidate("claude-code", "request")
            .unwrap();
        assert_eq!(
            store.auth_candidate_stop("claude-code", "request").unwrap(),
            Some(receipt)
        );
        store
            .finalize_completed_auth_candidate("claude-code", "missing")
            .unwrap();
        assert!(
            store
                .auth_candidate_stop("claude-code", "missing")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn completed_finalization_retains_marker_until_receipt_can_commit() {
        let dir = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap();
        let path = candidate(&store);
        store
            .mark_auth_candidate_completed("claude-code", "request")
            .unwrap();
        let receipts = store
            .auth_provider_root("claude-code")
            .join("login-receipts");
        fs::write(&receipts, "obstruction").unwrap();
        assert!(
            store
                .finalize_completed_auth_candidate("claude-code", "request")
                .is_err()
        );
        assert!(completed(&path.join(COMPLETED_FILE)).unwrap());
        assert_eq!(
            fs::read(path.join("home/credential")).unwrap(),
            b"synthetic"
        );
        fs::remove_file(receipts).unwrap();
        store
            .finalize_completed_auth_candidate("claude-code", "request")
            .unwrap();
        assert!(!path.exists());
        assert!(
            store
                .auth_candidate_stop("claude-code", "request")
                .unwrap()
                .unwrap()
                .candidate_removed
        );
    }

    #[test]
    fn failed_cleanup_stays_armed_and_can_retry() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(
            MachinePluginStore::new(dir.path(), Platform::Linux, "x86_64".into()).unwrap(),
        );
        let path = candidate(&store);
        std::fs::remove_dir_all(&path).unwrap();
        std::fs::write(&path, b"obstruction").unwrap();
        let mut guard = AuthCandidateCleanup::new(store.clone(), "claude-code", "request");
        assert!(guard.discard().is_err());
        std::fs::remove_file(&path).unwrap();
        candidate(&store);
        guard.discard().unwrap();
        assert!(!path.exists());
        // After successful cleanup, dropping the old guard must not delete a
        // later directory at the same path.
        candidate(&store);
        drop(guard);
        assert!(path.exists());
    }
}
