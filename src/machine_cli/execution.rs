//! Machine-owned target environment lifecycle. Each admitted session owns a
//! detached keeper; reconnect only reattaches and never recreates an incarnation.

use std::collections::{BTreeMap, HashMap};
use std::os::unix::fs::{
    DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _,
};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{Mutex, Semaphore};

use crate::execution_environment::{
    BindingV1, EnvironmentLocation, ExecutionAccess, RuntimeLocation, WorkspaceLocation,
};
use crate::execution_protocol::{self as keeper, LaunchContract, Scope};
use crate::machine_protocol::MachineWorkspace;
use crate::machine_protocol::execution::{Action, ExecutorInventory, Refusal, Request, Response};

mod abandonment;
mod recovery;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub schema: u16,
    pub host_command: PathBuf,
    pub executor: keeper::Executor,
    #[serde(default)]
    pub retention: Option<Retention>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Retention {
    pub command: PathBuf,
    pub closure: PathBuf,
}

pub struct Manager {
    service_id: Option<String>,
    machine_id: String,
    root: PathBuf,
    worktrees: crate::session_workspace::WorktreeRoots,
    /// Operator-declared names added to the base target environment.
    extra_environment: Vec<String>,
    logs: PathBuf,
    configuration: Option<Configuration>,
    systemd: bool,
    prepare: Mutex<()>,
    gates: parking_lot::Mutex<HashMap<String, Arc<Semaphore>>>,
    calls: Option<Arc<crate::managed_calls::host::CallHost>>,
}

impl Manager {
    /// A bound runtime may use only its exact Machine-prepared private entry.
    /// Neither an advertised source root nor a caller-supplied path substitutes
    /// for this marker, and checking it never creates or repairs an entry.
    pub(super) fn owns_runtime_entry(&self, session: &crate::runtime_wire::StartSession) -> bool {
        if let Some(child) = session
            .execution_binding
            .as_ref()
            .and_then(|binding| binding.managed_child())
        {
            return self.owns_managed_child(&child, session);
        }
        let Some(binding) = session
            .execution_binding
            .as_ref()
            .and_then(|record| record.for_runtime(&self.machine_id, &session.cwd).ok())
        else {
            return false;
        };
        if !keeper::valid_operation_id(&session.session_id) || self.service_id.is_none() {
            return false;
        }
        let directory = self.root.join("runtime").join(&session.session_id);
        if Path::new(&session.cwd) != directory
            || directory.canonicalize().ok().as_ref() != Some(&directory)
        {
            return false;
        }
        let marker = directory.join("entry.json");
        for path in [&directory, &marker] {
            let Ok(metadata) = std::fs::symlink_metadata(path) else {
                return false;
            };
            if metadata.mode() & 0o077 != 0
                || metadata.uid() != rustix::process::geteuid().as_raw()
                || (path == &directory && !metadata.is_dir())
                || (path == &marker && (!metadata.is_file() || metadata.len() > 8192))
            {
                return false;
            }
        }
        let expected = serde_json::json!({"schema": 1, "service_id": self.service_id,
            "session_id": session.session_id, "runtime": binding.runtime});
        std::fs::read(marker)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            == Some(expected)
    }

    fn owns_managed_child(
        &self,
        child: &crate::execution_environment::ManagedChildV1,
        session: &crate::runtime_wire::StartSession,
    ) -> bool {
        if self.service_id.is_none()
            || !child.accepts(&session.session_id, &self.machine_id, &session.cwd)
        {
            return false;
        }
        let directory = self.root.join("managed").join(&child.session_id);
        let workspace = directory.join("workspace");
        let marker = directory.join("snapshot.json");
        if Path::new(&child.cwd) != workspace
            || directory.canonicalize().ok().as_ref() != Some(&directory)
            || workspace.canonicalize().ok().as_ref() != Some(&workspace)
        {
            return false;
        }
        for path in [&directory, &marker] {
            let Ok(metadata) = std::fs::symlink_metadata(path) else {
                return false;
            };
            if metadata.uid() != rustix::process::geteuid().as_raw()
                || metadata.mode() & 0o077 != 0
                || (path == &directory && !metadata.is_dir())
                || (path == &marker && (!metadata.is_file() || metadata.len() > 16384))
            {
                return false;
            }
        }
        std::fs::read(marker)
            .ok()
            .and_then(|bytes| {
                serde_json::from_slice::<crate::execution_environment::ManagedChildV1>(&bytes).ok()
            })
            .as_ref()
            == Some(child)
    }

    pub fn new(
        service_id: Option<String>,
        machine_id: String,
        state_dir: &Path,
        configuration: Option<Configuration>,
        systemd: bool,
    ) -> Result<Self> {
        if let Some(config) = &configuration {
            ensure!(
                config.schema == 1
                    && config.host_command.is_absolute()
                    && Path::new(&config.executor.command).is_absolute()
                    && config.executor.sha256.len() == 64
                    && config
                        .executor
                        .sha256
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
                    && semver::Version::parse(&config.executor.version).is_ok()
                    && service_id
                        .as_deref()
                        .is_some_and(crate::service_identity::valid_service_id),
                "invalid execution component configuration"
            );
            if let Some(retention) = &config.retention {
                ensure!(
                    retention.command.is_absolute()
                        && retention.closure.parent() == Some(Path::new("/nix/store"))
                        && Path::new(&config.executor.command).starts_with(&retention.closure),
                    "invalid executor retention contract"
                );
            }
        }
        let namespace = format!(
            "{:x}",
            Sha256::digest(service_id.as_deref().unwrap_or("unassigned").as_bytes())
        );
        Ok(Self {
            service_id,
            machine_id,
            root: state_dir.join("execution").join(namespace),
            worktrees: crate::session_workspace::WorktreeRoots::single(state_dir.join("worktrees")),
            extra_environment: Vec::new(),
            logs: crate::logs::directory(state_dir),
            configuration,
            systemd,
            prepare: Mutex::new(()),
            gates: parking_lot::Mutex::new(HashMap::new()),
            calls: None,
        })
    }

    /// Host managed-call gateways and managed child rounds on this Machine.
    pub fn with_calls(mut self, calls: Arc<crate::managed_calls::host::CallHost>) -> Self {
        self.calls = Some(calls);
        self
    }

    pub fn calls(&self) -> Option<&Arc<crate::managed_calls::host::CallHost>> {
        self.calls.as_ref()
    }

    fn managed_root(&self) -> PathBuf {
        self.root.join("managed")
    }

    /// Use the Machine's session worktree roots and copy the operator-declared
    /// variables, already validated by the Machine CLI, into target environments.
    pub fn with_placement(
        mut self,
        worktrees: crate::session_workspace::WorktreeRoots,
        extra_environment: Vec<String>,
    ) -> Self {
        self.worktrees = worktrees;
        self.extra_environment = extra_environment;
        self
    }

    pub async fn request(&self, request: Request, workspaces: &[MachineWorkspace]) -> Response {
        if Some(request.service_id.as_str()) != self.service_id.as_deref()
            || request.machine_id != self.machine_id
        {
            return Response::Refused {
                reason: Refusal::IdentityMismatch,
            };
        }
        match request.action {
            Action::Recover { session_id, intent } => {
                match self.recover(&session_id, &intent).await {
                    Ok(binding) => Response::Prepared { binding },
                    Err(reason) => Response::Refused { reason },
                }
            }
            Action::PrepareRuntime { session_id } => {
                match self.prepare_runtime(&session_id).await {
                    Ok(runtime) => Response::RuntimePrepared { runtime },
                    Err(reason) => Response::Refused { reason },
                }
            }
            Action::Inventory => Response::Inventory {
                executor: self.configuration.as_ref().map(|config| ExecutorInventory {
                    protocol: 1,
                    digest: format!("sha256:{}", config.executor.sha256),
                    version: config.executor.version.clone(),
                }),
            },
            Action::Close {
                session_id,
                binding,
            } => match self.close(&session_id, &binding).await {
                Ok(()) => Response::Closed,
                Err(reason) => Response::Refused { reason },
            },
            Action::AbandonPreparation {
                session_id,
                preparation,
            } => match self.abandon(&session_id, &preparation).await {
                Ok(()) => Response::Closed,
                Err(reason) => Response::Refused { reason },
            },
            Action::Prepare {
                session_id,
                workspace_id,
                runtime,
            } => {
                match self
                    .prepare(&session_id, &workspace_id, runtime, workspaces)
                    .await
                {
                    Ok(binding) => Response::Prepared { binding },
                    Err(reason) => Response::Refused { reason },
                }
            }
            Action::Call {
                session_id,
                binding,
                command,
            } => match self.call(&session_id, &binding, command).await {
                Ok(response) => Response::Call { response },
                Err(reason) => Response::Refused { reason },
            },
            Action::InstallCallGateway { grant } => match &self.calls {
                Some(calls) if self.service_id.is_some() => match calls.install(&grant) {
                    Ok(()) => Response::CallGateway,
                    Err(error) => {
                        tracing::warn!(%error, "managed call gateway refused");
                        Response::Refused {
                            reason: Refusal::Unavailable,
                        }
                    }
                },
                _ => Response::Refused {
                    reason: Refusal::Unavailable,
                },
            },
            Action::RevokeCallGateway { grant } => match &self.calls {
                Some(calls) => {
                    calls.revoke(&grant);
                    Response::CallGateway
                }
                None => Response::CallGateway,
            },
            Action::PrepareManagedRound { round } => {
                if self.service_id.is_none() || self.machine_id == "local" || !round.validate() {
                    return Response::Refused {
                        reason: Refusal::InvalidRequest,
                    };
                }
                let roots = self.worktrees.clone();
                let sources: Vec<PathBuf> = workspaces
                    .iter()
                    .map(|workspace| PathBuf::from(&workspace.canonical_path))
                    .collect();
                // A Machine that never hosted an execution environment has
                // no private root yet.
                if std::fs::create_dir_all(&self.root)
                    .and_then(|()| {
                        std::fs::set_permissions(&self.root, std::fs::Permissions::from_mode(0o700))
                    })
                    .is_err()
                {
                    return Response::ManagedRoundRefused {
                        code: "preparation_failed".into(),
                    };
                }
                let managed = self.managed_root();
                match crate::managed_calls::snapshot::prepare(
                    &managed,
                    &self.machine_id,
                    &round,
                    &roots,
                    &sources,
                )
                .await
                {
                    Ok(prepared) => Response::ManagedRound { prepared },
                    Err(code) => Response::ManagedRoundRefused { code: code.into() },
                }
            }
            Action::PrepareManagedEnvironment {
                child_session_id,
                runtime,
            } => match self
                .prepare_managed(&child_session_id, runtime, workspaces)
                .await
            {
                Ok(binding) => Response::Prepared { binding },
                Err(reason) => Response::Refused { reason },
            },
            Action::CloseManagedChild { child_session_id } => {
                if !crate::managed_calls::valid_id(&child_session_id) {
                    return Response::Refused {
                        reason: Refusal::InvalidRequest,
                    };
                }
                // Stop a remote child's environment before removing the
                // snapshot it executes in.
                if let Err(reason) = self.close_managed_environment(&child_session_id).await {
                    return Response::Refused { reason };
                }
                match crate::managed_calls::snapshot::close(&self.managed_root(), &child_session_id)
                {
                    Ok(()) => Response::Closed,
                    Err(_) => Response::Refused {
                        reason: Refusal::EnvironmentLost,
                    },
                }
            }
        }
    }

    /// The Machine-written identity of a managed child's snapshot directory.
    fn managed_identity(
        &self,
        child: &str,
    ) -> Result<crate::execution_environment::ManagedChildV1, Refusal> {
        let directory = self.managed_root().join(child);
        let marker = directory.join("snapshot.json");
        for path in [&directory, &marker] {
            let metadata = std::fs::symlink_metadata(path).map_err(|_| Refusal::EnvironmentLost)?;
            if metadata.uid() != rustix::process::geteuid().as_raw()
                || metadata.mode() & 0o077 != 0
                || (path == &directory && !metadata.is_dir())
                || (path == &marker && (!metadata.is_file() || metadata.len() > 16384))
            {
                return Err(Refusal::IdentityMismatch);
            }
        }
        let identity: crate::execution_environment::ManagedChildV1 =
            serde_json::from_slice(&std::fs::read(&marker).map_err(|_| Refusal::EnvironmentLost)?)
                .map_err(|_| Refusal::IdentityMismatch)?;
        let workspace = directory.join("workspace");
        if identity.validate().is_err()
            || identity.session_id != child
            || identity.machine_id != self.machine_id
            || Path::new(&identity.cwd) != workspace
            || workspace.canonicalize().ok().as_ref() != Some(&workspace)
        {
            return Err(Refusal::IdentityMismatch);
        }
        Ok(identity)
    }

    /// Start, or observe, the read-only environment of a managed child whose
    /// Agent runtime is on another Machine. The keeper enforces the profile.
    async fn prepare_managed(
        &self,
        child: &str,
        runtime: RuntimeLocation,
        workspaces: &[MachineWorkspace],
    ) -> Result<BindingV1, Refusal> {
        let config = self.configuration.as_ref().ok_or(Refusal::Unavailable)?;
        if !keeper::valid_operation_id(child)
            || !crate::managed_calls::valid_id(child)
            || self.service_id.is_none()
            || self.machine_id == "local"
            || runtime.machine_id == self.machine_id
            || !crate::managed_calls::valid_id(&runtime.machine_id)
            || !Path::new(&runtime.cwd).is_absolute()
        {
            return Err(Refusal::InvalidRequest);
        }
        let identity = self.managed_identity(child)?;
        let source = workspaces
            .iter()
            .find(|workspace| workspace.id == identity.workspace_id)
            .ok_or(Refusal::WorkspaceUnavailable)?
            .canonical_path
            .clone();
        let managed = keeper::ManagedExecution {
            profile: identity.profile,
            round_path: self
                .managed_root()
                .join(child)
                .join("round.json")
                .display()
                .to_string(),
        };
        let _gate = self.prepare.lock().await;
        let directory = self.root.join(child);
        let path = directory.join("contract.json");
        if directory.join("closed.json").exists() {
            return Err(Refusal::EnvironmentLost);
        }
        if path.exists() {
            let contract = read_contract(&path).map_err(|_| Refusal::EnvironmentLost)?;
            if contract.session_id != child
                || contract.binding.runtime != runtime
                || contract.binding.workspace.cwd != identity.cwd
                || contract.binding.workspace.id != identity.workspace_id
                || contract.binding.environment.machine_id != self.machine_id
                || contract.managed.as_ref() != Some(&managed)
            {
                return Err(Refusal::IdentityMismatch);
            }
            return match exchange(&directory, &contract, keeper::Command::Describe).await {
                Ok(keeper::Response::Ready { scope, .. })
                    if scope == Scope::from_binding(&contract.binding) =>
                {
                    Ok(contract.binding)
                }
                _ => Err(Refusal::EnvironmentLost),
            };
        }
        if directory.exists() {
            return Err(Refusal::EnvironmentLost);
        }
        std::fs::create_dir_all(&self.root).map_err(|_| Refusal::PreparationFailed)?;
        std::fs::set_permissions(&self.root, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| Refusal::PreparationFailed)?;
        let binding = BindingV1 {
            schema: 1,
            id: random_id(),
            revision: 1,
            runtime,
            environment: EnvironmentLocation {
                machine_id: self.machine_id.clone(),
                id: random_id(),
                incarnation: random_id(),
                executor_digest: format!("sha256:{}", config.executor.sha256),
                protocol: 1,
            },
            workspace: WorkspaceLocation {
                id: identity.workspace_id.clone(),
                worktree_id: child.to_owned(),
                source_path: source,
                cwd: identity.cwd.clone(),
            },
            access: ExecutionAccess::Project,
            managed: Some(crate::execution_environment::ManagedBindingV1 {
                parent_session_id: identity.parent_session_id.clone(),
                profile: identity.profile,
            }),
        };
        binding.validate().map_err(|_| Refusal::InvalidRequest)?;
        self.start_keeper(config, child, &directory, binding, None, Some(managed))
            .await
    }

    /// Stop a managed child's environment, if it has one. Absent is closed.
    async fn close_managed_environment(&self, child: &str) -> Result<(), Refusal> {
        let directory = self.root.join(child);
        let path = directory.join("contract.json");
        if !path.exists() {
            return Ok(());
        }
        let contract = read_contract(&path).map_err(|_| Refusal::EnvironmentLost)?;
        if contract.session_id != child || contract.managed.is_none() {
            return Err(Refusal::IdentityMismatch);
        }
        let _gate = self.prepare.lock().await;
        self.close_locked(child, &contract.binding).await
    }

    async fn prepare_runtime(&self, session_id: &str) -> Result<RuntimeLocation, Refusal> {
        if !keeper::valid_operation_id(session_id)
            || self.service_id.is_none()
            || self.machine_id == "local"
        {
            return Err(Refusal::InvalidRequest);
        }
        let _gate = self.prepare.lock().await;
        let root = self.root.join("runtime");
        std::fs::create_dir_all(&root).map_err(|_| Refusal::PreparationFailed)?;
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| Refusal::PreparationFailed)?;
        let directory = root.join(session_id);
        let runtime = RuntimeLocation {
            machine_id: self.machine_id.clone(),
            cwd: directory.display().to_string(),
        };
        let marker = directory.join("entry.json");
        let expected = serde_json::json!({"schema": 1, "service_id": self.service_id, "session_id": session_id, "runtime": runtime});
        if let Ok(metadata) = std::fs::symlink_metadata(&directory) {
            if !metadata.is_dir()
                || metadata.mode() & 0o077 != 0
                || metadata.uid() != rustix::process::geteuid().as_raw()
            {
                return Err(Refusal::IdentityMismatch);
            }
            let metadata =
                std::fs::symlink_metadata(&marker).map_err(|_| Refusal::EnvironmentLost)?;
            if !metadata.is_file() || metadata.len() > 8192 || metadata.mode() & 0o077 != 0 {
                return Err(Refusal::IdentityMismatch);
            }
            let existing: serde_json::Value = serde_json::from_slice(
                &std::fs::read(&marker).map_err(|_| Refusal::EnvironmentLost)?,
            )
            .map_err(|_| Refusal::EnvironmentLost)?;
            return if existing == expected {
                Ok(runtime)
            } else {
                Err(Refusal::IdentityMismatch)
            };
        }
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .map_err(|_| Refusal::PreparationFailed)?;
        let file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&marker)
            .map_err(|_| Refusal::PreparationFailed)?;
        serde_json::to_writer(&file, &expected).map_err(|_| Refusal::PreparationFailed)?;
        file.sync_all().map_err(|_| Refusal::PreparationFailed)?;
        std::fs::File::open(&directory)
            .and_then(|file| file.sync_all())
            .map_err(|_| Refusal::PreparationFailed)?;
        Ok(runtime)
    }

    async fn prepare(
        &self,
        session_id: &str,
        workspace_id: &str,
        runtime: RuntimeLocation,
        workspaces: &[MachineWorkspace],
    ) -> Result<BindingV1, Refusal> {
        let config = self.configuration.as_ref().ok_or(Refusal::Unavailable)?;
        if !keeper::valid_operation_id(session_id) || self.machine_id == "local" {
            return Err(Refusal::InvalidRequest);
        }
        let workspace = workspaces
            .iter()
            .find(|workspace| workspace.id == workspace_id)
            .ok_or(Refusal::WorkspaceUnavailable)?;
        if runtime.machine_id.is_empty()
            || runtime.machine_id.len() > 128
            || !Path::new(&runtime.cwd).is_absolute()
        {
            return Err(Refusal::InvalidRequest);
        }
        let _gate = self.prepare.lock().await;
        let directory = self.root.join(session_id);
        let path = directory.join("contract.json");
        if self.root.join("abandoned").join(session_id).exists()
            || directory.join("closed.json").exists()
        {
            return Err(Refusal::EnvironmentLost);
        }
        if path.exists() {
            let contract = read_contract(&path).map_err(|_| Refusal::EnvironmentLost)?;
            if contract.session_id != session_id
                || contract.binding.workspace.id != workspace_id
                || contract.binding.workspace.source_path != workspace.canonical_path
                || contract.binding.environment.machine_id != self.machine_id
                || contract.binding.runtime != runtime
                // A managed environment never becomes an ordinary one.
                || contract.managed.is_some()
            {
                return Err(Refusal::IdentityMismatch);
            }
            let ready = exchange(&directory, &contract, keeper::Command::Describe)
                .await
                .map_err(|_| Refusal::EnvironmentLost)?;
            return match ready {
                keeper::Response::Ready { scope, .. }
                    if scope == Scope::from_binding(&contract.binding) =>
                {
                    Ok(contract.binding)
                }
                _ => Err(Refusal::EnvironmentLost),
            };
        }
        // Interrupted directories are retained. Never guess whether a previous
        // worktree/contract/keeper attempt happened and start it a second time.
        if directory.exists() {
            return Err(Refusal::EnvironmentLost);
        }
        std::fs::create_dir_all(&self.root).map_err(|_| Refusal::PreparationFailed)?;
        std::fs::set_permissions(&self.root, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| Refusal::PreparationFailed)?;
        let count = std::fs::read_dir(&self.root)
            .map_err(|_| Refusal::PreparationFailed)?
            .filter(|entry| {
                entry
                    .as_ref()
                    .map_or(true, |entry| !entry.path().join("stopped.json").exists())
            })
            .take(513)
            .count();
        if count >= 512 {
            return Err(Refusal::Capacity);
        }
        let prepared = crate::session_workspace::prepare(
            crate::session_workspace::PrepareWorkspaceRequest {
                root: workspace.canonical_path.clone(),
                session_id: session_id.to_owned(),
            },
            self.worktrees.for_session(session_id),
        )
        .await
        .map_err(|_| Refusal::WorkspaceUnavailable)?;
        let binding = BindingV1 {
            schema: 1,
            id: random_id(),
            revision: 1,
            runtime,
            environment: EnvironmentLocation {
                machine_id: self.machine_id.clone(),
                id: random_id(),
                incarnation: random_id(),
                executor_digest: format!("sha256:{}", config.executor.sha256),
                protocol: 1,
            },
            workspace: WorkspaceLocation {
                id: workspace_id.to_owned(),
                worktree_id: session_id.to_owned(),
                source_path: prepared.source_path,
                cwd: prepared.path,
            },
            access: ExecutionAccess::Project,
            managed: None,
        };
        binding.validate().map_err(|_| Refusal::InvalidRequest)?;
        let call_context = self
            .calls
            .as_ref()
            .and_then(|calls| calls.context_path(session_id))
            .map(|path| path.display().to_string());
        self.start_keeper(config, session_id, &directory, binding, call_context, None)
            .await
    }

    /// Create an admitted environment's private directory, contract and keeper.
    /// The caller holds the preparation gate and has checked that neither the
    /// directory nor a tombstone exists.
    async fn start_keeper(
        &self,
        config: &Configuration,
        session_id: &str,
        directory: &Path,
        binding: BindingV1,
        call_context: Option<String>,
        managed: Option<keeper::ManagedExecution>,
    ) -> Result<BindingV1, Refusal> {
        let path = directory.join("contract.json");
        std::fs::create_dir(directory).map_err(|_| Refusal::EnvironmentLost)?;
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| Refusal::PreparationFailed)?;
        if let Some(retention) = &config.retention {
            let status = tokio::time::timeout(
                Duration::from_secs(20),
                tokio::process::Command::new(&retention.command)
                    .arg("--add-root")
                    .arg(directory.join("executor-root"))
                    .arg("--indirect")
                    .arg("--realise")
                    .arg(&retention.closure)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .kill_on_drop(true)
                    .status(),
            )
            .await
            .map_err(|_| Refusal::PreparationFailed)?
            .map_err(|_| Refusal::PreparationFailed)?;
            if !status.success() {
                return Err(Refusal::PreparationFailed);
            }
        }
        let environment: BTreeMap<String, String> = crate::execution_target_environment::BASE
            .into_iter()
            .chain(self.extra_environment.iter().map(String::as_str))
            .filter_map(|key| std::env::var(key).ok().map(|value| (key.to_owned(), value)))
            .collect();
        let contract = LaunchContract {
            schema: 1,
            session_id: session_id.to_owned(),
            binding: binding.clone(),
            executor: config.executor.clone(),
            capability: format!("{}{}", random_id(), random_id()),
            environment,
            call_context,
            managed,
        };
        let file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&path)
            .map_err(|_| Refusal::PreparationFailed)?;
        serde_json::to_writer(&file, &contract).map_err(|_| Refusal::PreparationFailed)?;
        file.sync_all().map_err(|_| Refusal::PreparationFailed)?;
        std::fs::File::open(directory)
            .and_then(|file| file.sync_all())
            .map_err(|_| Refusal::PreparationFailed)?;
        self.spawn(config, &path, directory, &binding)
            .await
            .map_err(|_| Refusal::PreparationFailed)?;
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                if let Ok(keeper::Response::Ready { scope, .. }) =
                    exchange(directory, &contract, keeper::Command::Describe).await
                    && scope == Scope::from_binding(&binding)
                {
                    return binding;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .map_err(|_| Refusal::EnvironmentLost)
    }

    async fn spawn(
        &self,
        config: &Configuration,
        contract: &Path,
        directory: &Path,
        binding: &BindingV1,
    ) -> Result<()> {
        let mut process = if self.systemd {
            let mut process = tokio::process::Command::new("systemd-run");
            process.args([
                "--user",
                "--quiet",
                "--collect",
                "--service-type=exec",
                "--property=Restart=no",
                "--property=KillMode=control-group",
                "--property=TimeoutStopSec=15s",
                "--property=Slice=cowboy-agents.slice",
                &format!(
                    "--unit=cowboy-execution-{}",
                    binding.environment.incarnation
                ),
            ]);
            process.arg(&config.host_command);
            process
        } else {
            let mut process = tokio::process::Command::new(&config.host_command);
            process.process_group(0);
            process
        };
        process
            .args(["--contract"])
            .arg(contract)
            .args(["--state-dir"])
            .arg(directory)
            .arg("--logs-dir")
            .arg(&self.logs)
            .arg("--socket")
            .arg(control_socket(directory, binding)?)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(false);
        let mut child = process.spawn().context("starting execution keeper")?;
        if self.systemd {
            ensure!(
                tokio::time::timeout(Duration::from_secs(10), child.wait())
                    .await??
                    .success(),
                "execution unit refused"
            );
        } else {
            tokio::spawn(async move {
                let _ = child.wait().await;
            });
        }
        Ok(())
    }

    async fn call(
        &self,
        session_id: &str,
        binding: &BindingV1,
        command: keeper::Command,
    ) -> Result<keeper::Response, Refusal> {
        if !keeper::valid_operation_id(session_id)
            || binding.validate().is_err()
            || binding.environment.machine_id != self.machine_id
        {
            return Err(Refusal::InvalidRequest);
        }
        let directory = self.root.join(session_id);
        if directory.join("closed.json").exists() {
            return Err(Refusal::EnvironmentLost);
        }
        let contract = read_contract(&directory.join("contract.json"))
            .map_err(|_| Refusal::EnvironmentLost)?;
        if contract.session_id != session_id || &contract.binding != binding {
            return Err(Refusal::IdentityMismatch);
        }
        let gate = {
            let mut gates = self.gates.lock();
            if !gates.contains_key(session_id) && gates.len() >= 512 {
                return Err(Refusal::Capacity);
            }
            Arc::clone(
                gates
                    .entry(session_id.to_owned())
                    .or_insert_with(|| Arc::new(Semaphore::new(24))),
            )
        };
        let _permit = gate.try_acquire().map_err(|_| Refusal::Capacity)?;
        exchange(&directory, &contract, command)
            .await
            .map_err(|_| Refusal::Unavailable)
    }

    async fn close(&self, session_id: &str, binding: &BindingV1) -> Result<(), Refusal> {
        if !keeper::valid_operation_id(session_id)
            || binding.validate().is_err()
            || binding.environment.machine_id != self.machine_id
        {
            return Err(Refusal::InvalidRequest);
        }
        let _gate = self.prepare.lock().await;
        self.close_locked(session_id, binding).await
    }

    async fn close_locked(&self, session_id: &str, binding: &BindingV1) -> Result<(), Refusal> {
        let directory = self.root.join(session_id);
        let contract = read_contract(&directory.join("contract.json"))
            .map_err(|_| Refusal::EnvironmentLost)?;
        if contract.session_id != session_id || &contract.binding != binding {
            return Err(Refusal::IdentityMismatch);
        }
        if directory.join("stopped.json").exists() {
            return validate_close_marker(&directory.join("stopped.json"), binding);
        }
        // Record the permanent fence before stopping. A lost stop reply can be
        // retried against this exact unit, but preparation can never restart it.
        write_close_marker(&directory, "closed.json", binding)?;
        if self.systemd {
            let unit = format!(
                "cowboy-execution-{}.service",
                binding.environment.incarnation
            );
            let status = tokio::time::timeout(
                Duration::from_secs(20),
                tokio::process::Command::new("systemctl")
                    .args(["--user", "stop", &unit])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .kill_on_drop(true)
                    .status(),
            )
            .await;
            if !matches!(status, Ok(Ok(status)) if status.success()) {
                // --collect removes a fully stopped unit. A missing unit is a
                // valid repeat observation, not permission to stop another one.
                let observation = tokio::time::timeout(
                    Duration::from_secs(3),
                    tokio::process::Command::new("systemctl")
                        .args([
                            "--user",
                            "show",
                            &unit,
                            "--property=LoadState",
                            "--property=ActiveState",
                        ])
                        .stdin(Stdio::null())
                        .stderr(Stdio::null())
                        .kill_on_drop(true)
                        .output(),
                )
                .await;
                if !matches!(observation, Ok(Ok(output)) if output.status.success() && String::from_utf8_lossy(&output.stdout).lines().any(|line| line == "LoadState=not-found") && String::from_utf8_lossy(&output.stdout).lines().any(|line| line == "ActiveState=inactive"))
                {
                    return Err(Refusal::Unavailable);
                }
            }
        } else {
            let response = exchange(&directory, &contract, keeper::Command::Shutdown)
                .await
                .map_err(|_| Refusal::Unavailable)?;
            if response != keeper::Response::Closed {
                return Err(Refusal::Unavailable);
            }
        }
        write_close_marker(&directory, "stopped.json", binding)?;
        self.gates.lock().remove(session_id);
        // Worktree and tombstone remain; stopped native bytes need no GC root.
        let _ = std::fs::remove_file(directory.join("executor-root"));
        Ok(())
    }
}

fn write_close_marker(directory: &Path, name: &str, binding: &BindingV1) -> Result<(), Refusal> {
    match std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(directory.join(name))
    {
        Ok(file) => {
            serde_json::to_writer(&file, &keeper::Scope::from_binding(binding))
                .map_err(|_| Refusal::Unavailable)?;
            file.sync_all().map_err(|_| Refusal::Unavailable)?;
            std::fs::File::open(directory)
                .and_then(|file| file.sync_all())
                .map_err(|_| Refusal::Unavailable)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            validate_close_marker(&directory.join(name), binding)?;
        }
        Err(_) => return Err(Refusal::Unavailable),
    }
    Ok(())
}

fn validate_close_marker(path: &Path, binding: &BindingV1) -> Result<(), Refusal> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| Refusal::Unavailable)?;
    if !metadata.is_file()
        || metadata.len() > 8192
        || metadata.mode() & 0o077 != 0
        || metadata.uid() != rustix::process::geteuid().as_raw()
    {
        return Err(Refusal::IdentityMismatch);
    }
    let scope: keeper::Scope =
        serde_json::from_slice(&std::fs::read(path).map_err(|_| Refusal::Unavailable)?)
            .map_err(|_| Refusal::IdentityMismatch)?;
    if scope != keeper::Scope::from_binding(binding) {
        return Err(Refusal::IdentityMismatch);
    }
    Ok(())
}

fn random_id() -> String {
    format!("{:032x}", rand::random::<u128>())
}

fn read_contract(path: &Path) -> Result<LaunchContract> {
    let metadata = std::fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file()
            && metadata.permissions().mode() & 0o077 == 0
            && metadata.len() <= 64 * 1024,
        "invalid private execution contract"
    );
    let contract: LaunchContract = serde_json::from_slice(&std::fs::read(path)?)
        .map_err(|_| anyhow::anyhow!("execution contract unavailable"))?;
    contract.binding.validate().map_err(anyhow::Error::msg)?;
    ensure!(contract.schema == 1, "unsupported execution contract");
    Ok(contract)
}

async fn exchange(
    directory: &Path,
    contract: &LaunchContract,
    command: keeper::Command,
) -> Result<keeper::Response> {
    tokio::time::timeout(Duration::from_secs(24), async {
        let mut stream = UnixStream::connect(control_socket(directory, &contract.binding)?).await?;
        let request = keeper::Request {
            schema: 1,
            scope: Scope::from_binding(&contract.binding),
            capability: contract.capability.clone(),
            command,
        };
        let mut bytes = serde_json::to_vec(&request)?;
        ensure!(
            bytes.len() < keeper::MAX_FRAME_BYTES,
            "execution request exceeds limit"
        );
        bytes.push(b'\n');
        stream.write_all(&bytes).await?;
        let mut bytes = Vec::new();
        BufReader::new(stream)
            .take((keeper::MAX_FRAME_BYTES + 1) as u64)
            .read_until(b'\n', &mut bytes)
            .await?;
        ensure!(
            bytes.len() <= keeper::MAX_FRAME_BYTES && bytes.last() == Some(&b'\n'),
            "invalid execution response"
        );
        serde_json::from_slice(&bytes).map_err(|_| anyhow::anyhow!("invalid execution response"))
    })
    .await?
}

/// Unix socket address limits apply to the path passed to bind/connect, not
/// the resolved directory. A private short alias keeps durable state under its
/// full Service namespace without depending on /run/user surviving a logout.
fn control_socket(directory: &Path, binding: &BindingV1) -> Result<PathBuf> {
    let uid = rustix::process::geteuid().as_raw();
    let root = PathBuf::from(format!("/tmp/cowboy-execution-{uid}"));
    match std::fs::DirBuilder::new().mode(0o700).create(&root) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let metadata = std::fs::symlink_metadata(&root)?;
    ensure!(
        metadata.is_dir() && metadata.uid() == uid && metadata.mode() & 0o077 == 0,
        "private control namespace unavailable"
    );
    let directory = std::fs::canonicalize(directory)?;
    let alias = root.join(&binding.environment.incarnation);
    match std::os::unix::fs::symlink(&directory, &alias) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            ensure!(
                std::fs::read_link(&alias)? == directory,
                "execution control alias changed"
            );
        }
        Err(error) => return Err(error.into()),
    }
    Ok(alias.join("keeper.sock"))
}
