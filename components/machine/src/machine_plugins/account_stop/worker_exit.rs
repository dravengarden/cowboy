//! Per-launch containment journal. Intent and process exit are distinct facts.
use super::{
    MachinePluginStore, ProviderAccountId, atomic_write, marker_name, native_identity,
    prepare_private_directory,
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerExit {
    schema: u32,
    pub session: String,
    pub account: ProviderAccountId,
    pub launch: uuid::Uuid,
    pub tree_exited: bool,
    #[serde(default)]
    pub binding: Option<WorkerBinding>,
    #[serde(default)]
    pub readmission: Option<String>,
    #[serde(default)]
    pub boot: Option<uuid::Uuid>,
    #[serde(default)]
    pub rebooted_into: Option<uuid::Uuid>,
    #[serde(default)]
    pub systemd: Option<SystemdWorker>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub containment: Option<crate::cgroup::ContainmentIdentity>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerBinding {
    pub(super) provider: String,
    pub(super) version: String,
    pub(super) digest: String,
    pub(super) generation: u64,
}

impl WorkerBinding {
    fn from_launch(launch: &crate::runtime_wire::StartSession) -> Result<Self> {
        let binding = Self {
            provider: launch.provider.clone(),
            version: launch.provider_version.clone(),
            digest: launch.provider_generation_digest.clone(),
            generation: launch.provider_auth_generation.unwrap_or_default(),
        };
        binding.validate()?;
        Ok(binding)
    }

    pub(super) fn validate(&self) -> Result<()> {
        ensure!(
            self.generation > 0
                && [&self.provider, &self.version, &self.digest]
                    .iter()
                    .all(|value| !value.is_empty() && value.len() <= 512 && !value.contains('\0')),
            "invalid worker native binding"
        );
        Ok(())
    }
}

#[test]
fn bound_launch_survives_restart_and_rejects_generation_changes() {
    let temp = tempfile::tempdir().unwrap();
    let open = || {
        MachinePluginStore::new(
            temp.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap()
    };
    let store = open();
    let launch: crate::runtime_wire::StartSession = serde_json::from_value(serde_json::json!({
        "session_id":"bound", "provider":"claude-code", "provider_account_id":uuid::Uuid::new_v4().to_string(),
        "provider_version":"1", "provider_generation_digest":"digest", "provider_auth_generation":7,
        "cwd":"/fixture", "generation":"worker"
    })).unwrap();
    let first = store.begin_bound_account_worker(&launch).unwrap();
    assert!(store.begin_bound_account_worker(&launch).is_err());
    store
        .record_account_worker_exit(&launch.session_id, &launch.provider_account_id, first)
        .unwrap();
    drop(store);
    let store = open();
    assert!(store.account_worker_binding_matches(&launch).unwrap());
    for field in 0..6 {
        let mut changed = launch.clone();
        match field {
            0 => changed.provider = "other".into(),
            1 => changed.provider_version = "2".into(),
            2 => changed.provider_generation_digest = "other".into(),
            3 => changed.provider_auth_generation = Some(8),
            4 => changed.provider_account_id = ProviderAccountId::default(),
            _ => changed.provider_auth_generation = None,
        }
        assert!(
            !store
                .account_worker_binding_matches(&changed)
                .unwrap_or(false)
        );
        assert!(store.begin_bound_account_worker(&changed).is_err());
    }
    assert!(
        store
            .begin_account_worker(&launch.session_id, &launch.provider_account_id)
            .is_err()
    );
    let next = store.begin_bound_account_worker(&launch).unwrap();
    assert_ne!(first, next);
    store
        .begin_account_worker("legacy", &launch.provider_account_id)
        .unwrap();
    let mut legacy = launch.clone();
    legacy.session_id = "legacy".into();
    assert!(!store.account_worker_binding_matches(&legacy).unwrap());
}

#[test]
fn a_named_binding_carries_only_to_a_release_its_login_admits() {
    use super::super::native_profile::tests::{package, ready};
    let temp = tempfile::tempdir().unwrap();
    let store = MachinePluginStore::new(
        temp.path(),
        crate::machine_protocol::Platform::Linux,
        "x86_64".into(),
    )
    .unwrap();
    let account: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
    let scoped = store.for_account(&account).unwrap();
    let signed_in = package();
    ready(&scoped, &signed_in);
    let mut later = package();
    later.manifest.version = "999.0.0".into();
    let mut changed = package();
    changed.manifest.version = "999.1.0".into();
    changed.manifest.authentication.required = !changed.manifest.authentication.required;
    let retained = |_: &str, digest: &str| -> Result<super::super::ProviderPackage> {
        match digest {
            "fixture" => Ok(signed_in.clone()),
            "later" => Ok(later.clone()),
            "changed" => Ok(changed.clone()),
            _ => anyhow::bail!("not installed"),
        }
    };
    let binding = |version: &str, digest: &str, generation: u64| WorkerBinding {
        provider: "claude-code".into(),
        version: version.into(),
        digest: digest.into(),
        generation,
    };
    let old = binding(&signed_in.manifest.version, "fixture", 1);
    let carries = |account: &ProviderAccountId, incoming: &WorkerBinding| {
        store.release_carries_with(account, &old, incoming, &retained)
    };
    assert!(carries(&account, &old));
    assert!(carries(&account, &binding("999.0.0", "later", 1)));
    for refused in [
        binding("999.0.1", "later", 1), // declared version is not the retained package's
        binding("999.1.0", "changed", 1), // signed authentication changed
        binding("999.0.0", "later", 2), // another login generation
        binding("999.0.0", "missing", 1), // release not installed
    ] {
        assert!(!carries(&account, &refused), "{refused:?}");
    }
    let mut other_provider = binding("999.0.0", "later", 1);
    other_provider.provider = "codex".into();
    assert!(!carries(&account, &other_provider));
    assert!(!carries(
        &ProviderAccountId::default(),
        &binding("999.0.0", "later", 1)
    ));
    // A revoked login carries nowhere, while the exact binding is unaffected.
    scoped.revoke_native_profiles("claude-code").unwrap();
    assert!(!carries(&account, &binding("999.0.0", "later", 1)));
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemdWorker {
    pub unit: String,
    pub invocation: uuid::Uuid,
    pub boot: uuid::Uuid,
    pub cgroup: String,
}

impl MachinePluginStore {
    pub fn observe_account_worker_exit(
        &self,
        request: &crate::provider_account::WorkerExitRequest,
    ) -> Result<Option<crate::provider_account::WorkerExitObservation>> {
        let _guard = self.auth_candidates.lock();
        ensure!(!request.account_id.is_default(), "named account required");
        if !self.account_stop_matches(&request.session_id, &request.account_id)? {
            return Ok(None);
        }
        if let Some(command) = &request.stop_command_id
            && self
                .account_stop_command(&request.session_id, &request.account_id)?
                .as_ref()
                != Some(command)
        {
            return Ok(None);
        }
        Ok(self
            .account_worker_exit(&request.session_id)?
            .filter(|receipt| receipt.account == request.account_id && receipt.tree_exited)
            .map(|receipt| crate::provider_account::WorkerExitObservation {
                request: request.clone(),
                launch_id: receipt.launch,
            }))
    }

    fn worker_exit_root(&self) -> PathBuf {
        self.auth_base.join("session-account-worker-exits")
    }

    /// Absence is unknown; reading never creates an exit observation.
    pub fn account_worker_exit(&self, session: &str) -> Result<Option<WorkerExit>> {
        let root = self.worker_exit_root();
        match fs::symlink_metadata(&root) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            result => ensure!(result?.is_dir(), "invalid worker exit namespace"),
        }
        let name = marker_name(session);
        match fs::symlink_metadata(root.join(&name)) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            result => ensure!(result?.is_file(), "invalid worker exit receipt"),
        }
        let receipt: WorkerExit =
            serde_json::from_slice(&native_identity::read_login_file(&root, &name, 4096)?)?;
        ensure!(
            receipt.schema == 1
                && receipt.session == session
                && !receipt.account.is_default()
                && !receipt.launch.is_nil()
                && receipt.boot.is_none_or(|boot| !boot.is_nil()),
            "worker exit receipt scope mismatch"
        );
        ensure!(
            receipt.rebooted_into.is_none_or(|current| !current.is_nil()
                && receipt.tree_exited
                && receipt.boot.is_some_and(|previous| previous != current)),
            "invalid worker reboot evidence"
        );
        if let Some(binding) = &receipt.binding {
            binding.validate()?;
        }
        if let Some(identity) = &receipt.containment {
            ensure!(
                receipt.systemd.is_none() && receipt.boot == Some(identity.boot()),
                "worker containment scope mismatch"
            );
        }
        ensure!(
            receipt
                .readmission
                .as_ref()
                .is_none_or(|operation| receipt.tree_exited
                    && receipt.binding.is_some()
                    && crate::managed_calls::valid_id(operation)),
            "invalid worker readmission record"
        );
        Ok(Some(receipt))
    }

    /// Durable binding comparison for restart/adoption. Missing old binding
    /// metadata is unknown and cannot authorize a named worker replacement.
    pub fn account_worker_binding_matches(
        &self,
        launch: &crate::runtime_wire::StartSession,
    ) -> Result<bool> {
        self.account_worker_binding_admits(launch, false)
    }

    /// [`Self::account_worker_binding_matches`], also admitting a release the
    /// same login carries over to ([`Self::account_release_carries`]). The
    /// previous tree must have exited unless `replacing`: a reset confirms the
    /// stop before its launch, whose journal entry rechecks the exit.
    pub fn account_worker_binding_admits(
        &self,
        launch: &crate::runtime_wire::StartSession,
        replacing: bool,
    ) -> Result<bool> {
        let previous = self.account_worker_exit(&launch.session_id)?;
        if launch.provider_account_id.is_default() {
            return Ok(previous.is_none());
        }
        let incoming = WorkerBinding::from_launch(launch)?;
        Ok(previous.is_none_or(|old| {
            old.account == launch.provider_account_id
                && old.binding.as_ref().is_some_and(|binding| {
                    *binding == incoming
                        || ((replacing || old.tree_exited)
                            && self.account_release_carries(&old.account, binding, &incoming))
                })
        }))
    }

    /// A named login carries one session's worker binding to another release
    /// in place: same account, Provider and login generation, and the login's
    /// native profile admits the release (identical signed authentication to
    /// its retained signed-in release; see `read_native_profile`). The native
    /// home and session are unchanged; this is never a new login.
    pub fn account_release_carries(
        &self,
        account: &ProviderAccountId,
        previous: &WorkerBinding,
        incoming: &WorkerBinding,
    ) -> bool {
        self.release_carries_with(account, previous, incoming, &|provider, digest| {
            self.package_for_generation(provider, digest)
                .map(|(package, _)| package)
        })
    }

    /// [`Self::account_release_carries`] with an explicit retained-package
    /// lookup (the verified installed generations in production).
    pub(super) fn release_carries_with(
        &self,
        account: &ProviderAccountId,
        previous: &WorkerBinding,
        incoming: &WorkerBinding,
        retained: &dyn Fn(&str, &str) -> Result<super::super::ProviderPackage>,
    ) -> bool {
        if *previous == *incoming {
            return true;
        }
        !account.is_default()
            && previous.provider == incoming.provider
            && previous.generation == incoming.generation
            && retained(&incoming.provider, &incoming.digest)
                .ok()
                .filter(|package| {
                    package.manifest.id == incoming.provider
                        && package.manifest.version == incoming.version
                })
                .is_some_and(|package| {
                    self.for_account(account).is_ok_and(|scoped| {
                        scoped
                            .native_profile_home_with(&package, incoming.generation, retained)
                            .is_ok()
                    })
                })
    }

    /// [`Self::account_release_carries`] for two launch declarations.
    pub fn account_launch_carries(
        &self,
        previous: &crate::runtime_wire::StartSession,
        incoming: &crate::runtime_wire::StartSession,
    ) -> bool {
        let account = &incoming.provider_account_id;
        previous.provider_account_id == *account
            && matches!(
                (WorkerBinding::from_launch(previous), WorkerBinding::from_launch(incoming)),
                (Ok(old), Ok(new)) if self.account_release_carries(account, &old, &new)
            )
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn begin_bound_account_worker(
        &self,
        launch: &crate::runtime_wire::StartSession,
    ) -> Result<uuid::Uuid> {
        self.begin_contained_account_worker(launch, None)
    }

    pub fn begin_contained_account_worker(
        &self,
        launch: &crate::runtime_wire::StartSession,
        containment: Option<&crate::cgroup::ContainmentIdentity>,
    ) -> Result<uuid::Uuid> {
        self.begin_account_worker_binding(
            &launch.session_id,
            &launch.provider_account_id,
            Some(WorkerBinding::from_launch(launch)?),
            containment.cloned(),
        )
    }

    /// Persist before spawn. Unknown prior launches cannot be overwritten even
    /// after a broker restart. A new launch invalidates the previous exit proof.
    #[cfg(any(test, feature = "test-support"))]
    pub fn begin_account_worker(
        &self,
        session: &str,
        account: &ProviderAccountId,
    ) -> Result<uuid::Uuid> {
        self.begin_account_worker_binding(session, account, None, None)
    }

    fn begin_account_worker_binding(
        &self,
        session: &str,
        account: &ProviderAccountId,
        binding: Option<WorkerBinding>,
        containment: Option<crate::cgroup::ContainmentIdentity>,
    ) -> Result<uuid::Uuid> {
        ensure!(
            !account.is_default()
                && !session.is_empty()
                && session.len() <= 512
                && !session.contains('\0'),
            "invalid worker exit scope"
        );
        let boot = current_boot()?;
        if let Some(identity) = &containment {
            ensure!(
                Some(identity.boot()) == boot
                    && *identity.owner() == crate::cgroup::ProcessOwner::current()?,
                "worker containment owner mismatch"
            );
        }
        self.reconcile_worker_boot(session, boot)?;
        let _guard = self.auth_candidates.lock();
        ensure!(
            !self.account_session_stopped(session)?,
            "account session is stopped"
        );
        if let Some(old) = self.account_worker_exit(session)? {
            ensure!(
                old.account == *account && old.tree_exited,
                "previous worker tree exit unconfirmed"
            );
            ensure!(
                old.binding == binding
                    || matches!((&old.binding, &binding), (Some(previous), Some(incoming))
                        if self.account_release_carries(account, previous, incoming)),
                "worker native binding changed without readmission"
            );
        }
        let launch = uuid::Uuid::new_v4();
        self.persist_worker_exit(&WorkerExit {
            schema: 1,
            session: session.into(),
            account: account.clone(),
            launch,
            tree_exited: false,
            binding,
            readmission: None,
            boot,
            rebooted_into: None,
            systemd: None,
            containment,
        })?;
        Ok(launch)
    }

    /// A new kernel boot ends every process from the recorded old boot. The
    /// journal remains scoped to this Machine's local state; moving journals
    /// between hosts is not a supported recovery mechanism.
    pub fn reconcile_account_worker_boot(&self, session: &str) -> Result<()> {
        self.reconcile_worker_boot(session, current_boot()?)
    }

    /// Explicit stop/recycle only; passive reads never kill an active worker.
    /// Reload the exact launch after the await so a stale recovery cannot mark
    /// a newer worker exited. Missing legacy containment remains unknown.
    pub async fn terminate_account_worker_tree(&self, session: &str) -> Result<()> {
        self.reconcile_account_worker_boot(session)?;
        let Some(receipt) = self.account_worker_exit(session)? else {
            return Ok(());
        };
        if !receipt.tree_exited
            && let Some(identity) = &receipt.containment
        {
            identity.terminate().await?;
            self.record_account_worker_exit(session, &receipt.account, receipt.launch)?;
        }
        Ok(())
    }

    fn reconcile_worker_boot(&self, session: &str, current: Option<uuid::Uuid>) -> Result<()> {
        ensure!(
            current.is_none_or(|boot| !boot.is_nil()),
            "invalid current boot"
        );
        let _guard = self.auth_candidates.lock();
        let Some(mut receipt) = self.account_worker_exit(session)? else {
            return Ok(());
        };
        if !receipt.tree_exited
            && let (Some(previous), Some(current)) = (receipt.boot, current)
            && previous != current
        {
            receipt.tree_exited = true;
            receipt.rebooted_into = Some(current);
            self.persist_worker_exit(&receipt)?;
        }
        Ok(())
    }

    /// The owning reaper needs child wait and cgroup-empty confirmation. The
    /// spawn owner may also record a synchronous spawn failure (no execution).
    /// Stale owners cannot complete a replacement launch.
    pub fn record_account_worker_exit(
        &self,
        session: &str,
        account: &ProviderAccountId,
        launch: uuid::Uuid,
    ) -> Result<()> {
        let _guard = self.auth_candidates.lock();
        let mut receipt = self
            .account_worker_exit(session)?
            .ok_or_else(|| anyhow::anyhow!("worker launch missing"))?;
        ensure!(
            receipt.account == *account && receipt.launch == launch,
            "worker exit scope mismatch"
        );
        receipt.tree_exited = true;
        self.persist_worker_exit(&receipt)
    }

    pub fn bind_account_worker_systemd(
        &self,
        session: &str,
        account: &ProviderAccountId,
        launch: uuid::Uuid,
        binding: SystemdWorker,
    ) -> Result<()> {
        let _guard = self.auth_candidates.lock();
        let mut receipt = self
            .account_worker_exit(session)?
            .ok_or_else(|| anyhow::anyhow!("worker launch missing"))?;
        ensure!(
            receipt.account == *account && receipt.launch == launch,
            "worker binding scope mismatch"
        );
        ensure!(
            !receipt.tree_exited && receipt.containment.is_none(),
            "worker already exited or bound to Direct containment"
        );
        ensure!(
            !binding.boot.is_nil() && receipt.boot.is_none_or(|boot| boot == binding.boot),
            "worker binding boot mismatch"
        );
        ensure!(
            receipt.systemd.as_ref().is_none_or(|old| old == &binding),
            "worker systemd binding conflict"
        );
        receipt.systemd = Some(binding);
        self.persist_worker_exit(&receipt)
    }

    pub(super) fn persist_worker_exit(&self, receipt: &WorkerExit) -> Result<()> {
        let bytes = serde_json::to_vec(receipt)?;
        ensure!(
            bytes.len() <= 4096,
            "worker exit journal exceeds read limit"
        );
        let root = self.worker_exit_root();
        prepare_private_directory(&root)?;
        atomic_write(&root.join(marker_name(&receipt.session)), &bytes, 0o600)?;
        fs::File::open(&root)?.sync_all()?;
        fs::File::open(&self.auth_base)?.sync_all()?;
        Ok(())
    }
}

fn current_boot() -> Result<Option<uuid::Uuid>> {
    #[cfg(target_os = "linux")]
    {
        let boot: uuid::Uuid = fs::read_to_string("/proc/sys/kernel/random/boot_id")?
            .trim()
            .parse()?;
        ensure!(!boot.is_nil(), "invalid kernel boot identity");
        Ok(Some(boot))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine_protocol::Platform;

    #[test]
    fn worker_boot_recovery_requires_known_distinct_boots() {
        let temp = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(temp.path(), Platform::Linux, "x86_64".into()).unwrap();
        let account = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        store.begin_account_worker("s", &account).unwrap();
        let mut receipt = store.account_worker_exit("s").unwrap().unwrap();
        let old_boot = uuid::Uuid::new_v4();
        receipt.boot = Some(old_boot);
        store.persist_worker_exit(&receipt).unwrap();
        store.reconcile_worker_boot("s", Some(old_boot)).unwrap();
        store.reconcile_worker_boot("s", None).unwrap();
        assert!(!store.account_worker_exit("s").unwrap().unwrap().tree_exited);
        assert!(
            store
                .reconcile_worker_boot("s", Some(uuid::Uuid::nil()))
                .is_err()
        );
        store
            .reconcile_worker_boot("s", Some(uuid::Uuid::new_v4()))
            .unwrap();
        drop(store);
        let store = MachinePluginStore::new(temp.path(), Platform::Linux, "x86_64".into()).unwrap();
        let recovered = store.account_worker_exit("s").unwrap().unwrap();
        assert!(recovered.tree_exited);
        assert!(recovered.rebooted_into.is_some());
        assert_eq!(recovered.boot, Some(old_boot));
        assert_eq!(recovered.launch, receipt.launch);
        let replacement = store.begin_account_worker("s", &account).unwrap();
        assert_ne!(replacement, receipt.launch);
        assert!(
            store
                .record_account_worker_exit("s", &account, receipt.launch)
                .is_err()
        );
        // Legacy journals without a boot stamp cannot infer a reboot.
        let mut unknown = store.account_worker_exit("s").unwrap().unwrap();
        unknown.boot = None;
        store.persist_worker_exit(&unknown).unwrap();
        store
            .reconcile_worker_boot("s", Some(uuid::Uuid::new_v4()))
            .unwrap();
        assert!(!store.account_worker_exit("s").unwrap().unwrap().tree_exited);
        unknown.boot = Some(uuid::Uuid::nil());
        store.persist_worker_exit(&unknown).unwrap();
        assert!(store.reconcile_account_worker_boot("s").is_err());
    }

    #[test]
    fn worker_exit_journal_survives_restart_and_rejects_stale_reapers() {
        let temp = tempfile::tempdir().unwrap();
        let store = MachinePluginStore::new(temp.path(), Platform::Linux, "x86_64".into()).unwrap();
        let account = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        assert!(store.account_worker_exit("s").unwrap().is_none());
        assert!(!store.worker_exit_root().exists());
        let first = store.begin_account_worker("s", &account).unwrap();
        drop(store);
        let store = MachinePluginStore::new(temp.path(), Platform::Linux, "x86_64".into()).unwrap();
        assert!(!store.account_worker_exit("s").unwrap().unwrap().tree_exited);
        assert!(store.begin_account_worker("s", &account).is_err());
        let other = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        assert!(
            store
                .record_account_worker_exit("s", &other, first)
                .is_err()
        );
        store
            .record_account_worker_exit("s", &account, first)
            .unwrap();
        store
            .record_account_worker_exit("s", &account, first)
            .unwrap();
        let second = store.begin_account_worker("s", &account).unwrap();
        let request = crate::provider_account::WorkerExitRequest {
            stop_command_id: None,
            account_id: account.clone(),
            session_id: "s".into(),
        };
        assert!(
            store
                .observe_account_worker_exit(&request)
                .unwrap()
                .is_none()
        );
        assert_ne!(first, second);
        assert!(
            store
                .record_account_worker_exit("s", &account, first)
                .is_err()
        );
        assert!(!store.account_worker_exit("s").unwrap().unwrap().tree_exited);
        store.stop_account_session("s", &account).unwrap();
        assert!(
            store
                .observe_account_worker_exit(&request)
                .unwrap()
                .is_none()
        );
        store
            .record_account_worker_exit("s", &account, second)
            .unwrap();
        assert!(store.begin_account_worker("s", &account).is_err());
        drop(store);
        let store = MachinePluginStore::new(temp.path(), Platform::Linux, "x86_64".into()).unwrap();
        let receipt = store.account_worker_exit("s").unwrap().unwrap();
        assert!(receipt.tree_exited);
        assert_eq!(receipt.launch, second);
        assert_eq!(
            store
                .observe_account_worker_exit(&request)
                .unwrap()
                .unwrap()
                .launch_id,
            second
        );
        let wrong = crate::provider_account::WorkerExitRequest {
            account_id: other,
            ..request
        };
        assert!(store.observe_account_worker_exit(&wrong).unwrap().is_none());
    }
}
#[test]
fn tree_observation_is_bound_to_the_current_stop_command() {
    let temp = tempfile::tempdir().unwrap();
    let open = || {
        MachinePluginStore::new(
            temp.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap()
    };
    let store = open();
    let account: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
    let launch = store.begin_account_worker("s", &account).unwrap();
    store
        .record_account_worker_exit("s", &account, launch)
        .unwrap();
    store.stop_account_session("s", &account).unwrap();
    let mut request = crate::provider_account::WorkerExitRequest {
        account_id: account.clone(),
        session_id: "s".into(),
        stop_command_id: Some("stop-a".into()),
    };
    assert!(
        store
            .observe_account_worker_exit(&request)
            .unwrap()
            .is_none()
    );
    store
        .stop_account_session_scoped("s", &account, "stop-a")
        .unwrap();
    assert_eq!(
        store
            .observe_account_worker_exit(&request)
            .unwrap()
            .unwrap()
            .launch_id,
        launch
    );
    store
        .stop_account_session_scoped("s", &account, "stop-b")
        .unwrap();
    assert!(
        store
            .observe_account_worker_exit(&request)
            .unwrap()
            .is_none()
    );
    drop(store);
    let store = open();
    assert_eq!(
        store.account_stops().unwrap(),
        vec![("s".into(), account.clone())]
    );
    assert!(store.begin_account_worker("s", &account).is_err());
    request.stop_command_id = Some("stop-b".into());
    assert_eq!(
        store
            .observe_account_worker_exit(&request)
            .unwrap()
            .unwrap()
            .launch_id,
        launch
    );
}
