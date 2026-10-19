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
pub(crate) struct WorkerExit {
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
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerBinding {
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SystemdWorker {
    pub unit: String,
    pub invocation: uuid::Uuid,
    pub boot: uuid::Uuid,
    pub cgroup: String,
}

impl MachinePluginStore {
    pub(crate) fn observe_account_worker_exit(
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
    pub(crate) fn account_worker_exit(&self, session: &str) -> Result<Option<WorkerExit>> {
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
    pub(crate) fn account_worker_binding_matches(
        &self,
        launch: &crate::runtime_wire::StartSession,
    ) -> Result<bool> {
        let previous = self.account_worker_exit(&launch.session_id)?;
        if launch.provider_account_id.is_default() {
            return Ok(previous.is_none());
        }
        let incoming = WorkerBinding::from_launch(launch)?;
        Ok(previous.is_none_or(|old| {
            old.account == launch.provider_account_id && old.binding.as_ref() == Some(&incoming)
        }))
    }

    pub(crate) fn begin_bound_account_worker(
        &self,
        launch: &crate::runtime_wire::StartSession,
    ) -> Result<uuid::Uuid> {
        self.begin_account_worker_binding(
            &launch.session_id,
            &launch.provider_account_id,
            Some(WorkerBinding::from_launch(launch)?),
        )
    }

    /// Persist before spawn. Unknown prior launches cannot be overwritten even
    /// after a broker restart. A new launch invalidates the previous exit proof.
    #[cfg(test)]
    pub(crate) fn begin_account_worker(
        &self,
        session: &str,
        account: &ProviderAccountId,
    ) -> Result<uuid::Uuid> {
        self.begin_account_worker_binding(session, account, None)
    }

    fn begin_account_worker_binding(
        &self,
        session: &str,
        account: &ProviderAccountId,
        binding: Option<WorkerBinding>,
    ) -> Result<uuid::Uuid> {
        ensure!(
            !account.is_default()
                && !session.is_empty()
                && session.len() <= 512
                && !session.contains('\0'),
            "invalid worker exit scope"
        );
        let boot = current_boot()?;
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
                old.binding == binding,
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
        })?;
        Ok(launch)
    }

    /// A new kernel boot ends every process from the recorded old boot. The
    /// journal remains scoped to this Machine's local state; moving journals
    /// between hosts is not a supported recovery mechanism.
    pub(crate) fn reconcile_account_worker_boot(&self, session: &str) -> Result<()> {
        self.reconcile_worker_boot(session, current_boot()?)
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
    pub(crate) fn record_account_worker_exit(
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

    pub(crate) fn bind_account_worker_systemd(
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
        ensure!(!receipt.tree_exited, "worker already exited");
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
        let root = self.worker_exit_root();
        prepare_private_directory(&root)?;
        atomic_write(
            &root.join(marker_name(&receipt.session)),
            &serde_json::to_vec(receipt)?,
            0o600,
        )?;
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
