//! The instance manager: one browser per Provider account, admitted through
//! the Device launch ledger, run in its own unit, verified from outside,
//! attached through leased display streams, persisted through published
//! snapshots and fenced by durable control records. Every mutation names the
//! incarnation it expects; a stale command is a recorded no-op.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use tokio::time::Instant;

use super::host::{self, BOOTSTRAP_PROFILE, SEALED_PROFILE};
use super::launcher::{Control, FailureKind, LauncherEvent, LauncherRefusal, ReadyReport};
use super::settings::Settings;
use super::snapshot::{self, StagingLedger, StagingReservation};
use super::state_file::{
    self, CompletedOperation, ControlRecord, InstanceRecord, OperationKind, PendingOperation,
};
use super::stream::{Stream, StreamEnd};
use super::unit::{self, Launch, Limits};
use super::verify::{self, Expectations, Verified};
use crate::launch_ledger::{Admission, LEDGER};
use crate::machine_cli::LoginSessions;
use crate::machine_plugins::{MachineBrowserRuntimePlan, MachinePluginStore};
use crate::machine_protocol::MachineEvent;
use crate::machine_protocol::machine_browser::{
    Capability, Command, Event, Geometry, InstanceRef, LEASE_EXPIRY_MS, Purpose, Reason, State,
    StateEvent, Viewport,
};
use crate::provider_account::ProviderAccountId;

/// Longest an open waits for memory headroom; an owner is waiting.
const MEMORY_WAIT: Duration = Duration::from_secs(15);
const MEMORY_POLL: Duration = Duration::from_secs(1);
/// Unit activation and control-socket appearance.
const UNIT_START_TIMEOUT: Duration = Duration::from_secs(20);
/// Ordinary launcher commands.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(15);
/// A frozen or stop-time profile copy.
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(90);
/// How long the launcher waits for the browser to exit on `SIGTERM` before a
/// stop-time copy; it must fit the unit's stop timeout with the copy.
const GRACEFUL_WAIT: Duration = Duration::from_secs(8);
const FREEZE_TIMEOUT: Duration = Duration::from_secs(3);
const SUPERVISOR_TICK: Duration = Duration::from_secs(1);
const SOCKET_SCAN_INTERVAL: Duration = Duration::from_secs(10);
/// Fixed X display number inside each instance's private namespaces.
const DISPLAY: &str = ":7";
const TMP_BYTES: u64 = 256 << 20;
const MAX_RECEIPTS: usize = 256;

/// The Controller connection events travel on, with the endpoint streams
/// are opened against and the native logins it dispatched.
struct Uplink {
    id: u64,
    events: tokio::sync::mpsc::UnboundedSender<MachineEvent>,
    endpoint: reqwest::Url,
    logins: Weak<parking_lot::Mutex<HashMap<String, crate::machine_cli::LoginSession>>>,
}

/// Dropping it detaches the connection it was created for (and no later one).
pub struct UplinkGuard {
    manager: Weak<BrowserManager>,
    id: u64,
}

impl Drop for UplinkGuard {
    fn drop(&mut self) {
        if let Some(manager) = self.manager.upgrade() {
            let mut uplink = manager.uplink.lock();
            if uplink.as_ref().is_some_and(|current| current.id == self.id) {
                *uplink = None;
            }
        }
    }
}

struct AccountSlot {
    /// Serializes lifecycle mutations and snapshots of one account.
    lifecycle: tokio::sync::Mutex<()>,
    instance: parking_lot::Mutex<Option<Arc<Instance>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Starting,
    Detached,
    Attached,
    Stopping,
}

struct Attachment {
    id: String,
    lease_expires: Instant,
    stream: Option<Stream>,
}

struct Live {
    phase: Phase,
    geometry: Geometry,
    verified: Verified,
    attachment: Option<Attachment>,
    detached_since: Option<Instant>,
    last_snapshot: Option<(u64, i64)>,
    last_periodic: Instant,
    last_scan: Instant,
    pending_navigation: Option<String>,
    /// Profile writes may have happened since the last published snapshot.
    unsaved: bool,
}

/// One running (or starting) browser instance.
pub struct Instance {
    account_id: ProviderAccountId,
    instance_id: String,
    incarnation: u64,
    purpose: Purpose,
    unit: String,
    runtime_dir: PathBuf,
    cgroup_dir: PathBuf,
    deadline: Instant,
    deadline_ms: i64,
    scale: u8,
    profile_revision: u64,
    plan: Arc<MachineBrowserRuntimePlan>,
    settings: Settings,
    report: ReadyReport,
    control: tokio::sync::Mutex<Control>,
    live: parking_lot::Mutex<Live>,
}

impl Instance {
    fn reference(&self) -> InstanceRef {
        InstanceRef {
            account_id: self.account_id.clone(),
            instance_id: self.instance_id.clone(),
            incarnation: self.incarnation,
        }
    }

    fn login_request(&self) -> Option<&str> {
        match &self.purpose {
            Purpose::SignIn { login_request_id } => Some(login_request_id),
            Purpose::Tab => None,
        }
    }

    fn expectations<'a>(
        &'a self,
        uid: u32,
        gid: u32,
        main_pid: u32,
        bootstrap: &'a str,
        sealed: &'a str,
    ) -> Expectations<'a> {
        Expectations {
            uid,
            gid,
            runtime_root: &self.plan.runtime_root,
            runtime_dir: &self.runtime_dir,
            cgroup_dir: &self.cgroup_dir,
            main_pid,
            profile_bytes: self.settings.profile_max_bytes,
            instance_bytes: self.settings.instance_max_bytes,
            policy_directory: &self.plan.contract.runtime.policy.directory,
            bootstrap_label: Some(bootstrap),
            sealed_label: Some(sealed),
        }
    }

    fn state(&self, request_id: Option<String>) -> StateEvent {
        let live = self.live.lock();
        StateEvent {
            request_id,
            instance: self.reference(),
            state: match live.phase {
                Phase::Starting => State::Starting,
                Phase::Detached => State::Detached,
                Phase::Attached => State::Attached,
                Phase::Stopping => State::Stopping,
            },
            reason: None,
            geometry: Some(live.geometry),
            scale: Some(self.scale),
            deadline_ms: Some(self.deadline_ms),
            attachment_id: live
                .attachment
                .as_ref()
                .map(|attachment| attachment.id.clone()),
            snapshot_generation: live.last_snapshot.map(|(generation, _)| generation),
            snapshot_at_ms: live.last_snapshot.map(|(_, at)| at),
            profile_revision: Some(self.profile_revision),
            unsaved_state_lost: false,
            observed_at_ms: unix_ms(),
        }
    }
}

pub struct BrowserManager {
    state_dir: PathBuf,
    providers: Arc<MachinePluginStore>,
    device_config: Option<crate::config::Handle>,
    uid: u32,
    gid: u32,
    boot_id: String,
    uplink: parking_lot::Mutex<Option<Uplink>>,
    next_uplink: std::sync::atomic::AtomicU64,
    accounts: parking_lot::Mutex<HashMap<String, Arc<AccountSlot>>>,
    staging: Arc<StagingLedger>,
    receipts: parking_lot::Mutex<VecDeque<(String, StateEvent)>>,
    headroom: fn() -> Option<u64>,
}

fn production_headroom() -> Option<u64> {
    crate::machine_cli::host_resources::memory_headroom_bytes()
}

impl BrowserManager {
    /// # Errors
    /// Returns when the kernel boot identity is unreadable.
    pub fn new(
        state_dir: &Path,
        providers: Arc<MachinePluginStore>,
        device_config: Option<crate::config::Handle>,
    ) -> Result<Arc<Self>> {
        let boot_id = crate::boot_identity::current()?
            .context("kernel boot identity unavailable")?
            .to_string();
        Ok(Arc::new(Self {
            state_dir: state_dir.to_path_buf(),
            providers,
            device_config,
            uid: rustix::process::getuid().as_raw(),
            gid: rustix::process::getgid().as_raw(),
            boot_id,
            uplink: parking_lot::Mutex::new(None),
            next_uplink: std::sync::atomic::AtomicU64::new(1),
            accounts: parking_lot::Mutex::new(HashMap::new()),
            staging: Arc::new(StagingLedger::default()),
            receipts: parking_lot::Mutex::new(VecDeque::new()),
            headroom: production_headroom,
        }))
    }

    /// Bind the Controller connection whose events, stream endpoint and
    /// native logins this manager uses from now on; running instances report
    /// their state to it at once.
    pub(crate) fn attach(
        self: &Arc<Self>,
        events: tokio::sync::mpsc::UnboundedSender<MachineEvent>,
        endpoint: reqwest::Url,
        logins: &LoginSessions,
    ) -> UplinkGuard {
        let id = self
            .next_uplink
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        *self.uplink.lock() = Some(Uplink {
            id,
            events: events.clone(),
            endpoint,
            logins: Arc::downgrade(logins),
        });
        for instance in self.running() {
            let _ = events.send(MachineEvent::Browser {
                browser: Event::State(instance.state(None)),
            });
        }
        UplinkGuard {
            manager: Arc::downgrade(self),
            id,
        }
    }

    /// Whether this Machine can run browsers, and with which Plugin
    /// generation. `None` when no Machine browser Plugin is installed.
    #[must_use]
    pub fn capability(&self) -> Option<Capability> {
        let plan = match self.providers.machine_browser_runtime() {
            Ok(plan) => plan?,
            Err(error) => {
                tracing::warn!(%error, "Machine browser runtime is not usable");
                return None;
            }
        };
        let settings = self.settings();
        let (available, reason) = match host::check(&plan.contract.host_requirements) {
            Ok(()) => (true, None),
            Err(unmet) => {
                tracing::info!(detail = %unmet.0, "Machine browser host requirements unmet");
                (false, Some(Reason::HostRequirements))
            }
        };
        Some(Capability {
            plugin_id: plan.plugin_id,
            plugin_version: plan.plugin_version,
            generation_digest: plan.generation_digest,
            available,
            reason,
            max_instances: settings.max_instances,
        })
    }

    /// Dispatch one Controller command. Replays answer with their original
    /// receipt; everything else answers with a state observation or a
    /// command result.
    pub async fn handle(self: &Arc<Self>, request_id: String, command: Command) {
        if let Err(error) = command.validate() {
            self.send_result(
                request_id,
                false,
                Some(format!("malformed browser command: {error}")),
            );
            return;
        }
        if let Some(receipt) = self.receipt(&request_id) {
            self.send_state(receipt);
            return;
        }
        let outcome = match command {
            Command::Open {
                account_id,
                instance_id,
                purpose,
                viewport,
            } => Some(
                self.open(&request_id, account_id, instance_id, purpose, viewport)
                    .await,
            ),
            Command::Attach {
                instance,
                attachment_id,
                stream_token,
                viewport,
            } => Some(
                self.attach_viewer(&request_id, instance, attachment_id, stream_token, viewport)
                    .await,
            ),
            Command::Lease {
                instance,
                attachment_id,
                expires_at_ms,
            } => self.lease(&request_id, &instance, &attachment_id, expires_at_ms),
            Command::Detach {
                instance,
                attachment_id,
            } => Some(self.detach(&request_id, &instance, &attachment_id)),
            Command::Resize {
                instance,
                attachment_id,
                width,
                height,
            } => Some(
                self.resize(&request_id, &instance, &attachment_id, width, height)
                    .await,
            ),
            Command::Navigate { instance, url } => {
                match self.navigate(&request_id, &instance, &url).await {
                    Ok(event) => Some(event),
                    Err(detail) => {
                        self.send_result(request_id, false, Some(detail));
                        return;
                    }
                }
            }
            Command::Snapshot { instance } => Some(self.snapshot_now(&request_id, &instance).await),
            Command::Hibernate { instance } => Some(self.hibernate(&request_id, &instance).await),
            Command::Reset {
                account_id,
                expected_profile_revision,
            } => Some(
                self.reset_or_revoke(
                    &request_id,
                    account_id,
                    OperationKind::Reset,
                    &request_id.clone(),
                    Some(expected_profile_revision),
                )
                .await,
            ),
            Command::Revoke {
                account_id,
                operation_id,
            } => Some(
                self.reset_or_revoke(
                    &request_id,
                    account_id,
                    OperationKind::Revoke,
                    &operation_id,
                    None,
                )
                .await,
            ),
            Command::Query { account_id } => Some(self.query(&request_id, &account_id)),
        };
        if let Some(event) = outcome {
            self.record_receipt(&request_id, &event);
            self.send_state(event);
        }
    }

    /// A native login emitted its verification URL: the bound sign-in
    /// browser navigates there once it is ready and verified.
    pub fn login_challenge(self: &Arc<Self>, request_id: &str, url: &str) {
        let Some(instance) = self.bound_instance(request_id) else {
            return;
        };
        let ready = {
            let mut live = instance.live.lock();
            if matches!(live.phase, Phase::Detached | Phase::Attached) {
                true
            } else {
                live.pending_navigation = Some(url.to_owned());
                false
            }
        };
        if ready {
            let manager = Arc::clone(self);
            let url = url.to_owned();
            tokio::spawn(async move {
                manager.navigate_verified(&instance, &url).await;
            });
        }
    }

    /// A native login ended (completed, cancelled, failed or expired): its
    /// browser follows it into hibernation.
    pub fn login_ended(self: &Arc<Self>, request_id: &str) {
        let Some(instance) = self.bound_instance(request_id) else {
            return;
        };
        let manager = Arc::clone(self);
        tokio::spawn(async move {
            let event = manager
                .stop_instance(&instance, Reason::LoginEnded, true, None)
                .await;
            manager.send_state(event);
        });
    }

    /// Reconcile persisted instance records and live units at Machine start:
    /// clear records of another boot, stop sign-in instances and unrecorded
    /// units, adopt `Tab` instances whose unit survived and re-verifies,
    /// resume fenced operations and remove abandoned staging.
    pub async fn reconcile_at_start(self: &Arc<Self>) {
        if let Err(error) = self.reconcile_units().await {
            tracing::warn!(%error, "browser unit reconciliation incomplete");
        }
        if let Err(error) = self.reconcile_accounts().await {
            tracing::warn!(%error, "browser account reconciliation incomplete");
        }
    }

    // ---- lifecycle -------------------------------------------------------

    async fn open(
        self: &Arc<Self>,
        request_id: &str,
        account_id: ProviderAccountId,
        instance_id: String,
        purpose: Purpose,
        viewport: Viewport,
    ) -> StateEvent {
        let slot = self.slot(&account_id);
        let hibernated =
            |reason: Reason| hibernated_event(request_id, &account_id, &instance_id, reason, None);
        let existing = slot.instance.lock().clone();
        if let Some(existing) = existing {
            return if existing.instance_id == instance_id {
                existing.state(Some(request_id.to_owned()))
            } else {
                hibernated(Reason::Busy)
            };
        }
        let _lifecycle = slot.lifecycle.lock().await;
        if slot.instance.lock().is_some() {
            return hibernated(Reason::Busy);
        }
        let plan = match self.providers.machine_browser_runtime() {
            Ok(Some(plan)) => Arc::new(plan),
            Ok(None) => return hibernated(Reason::NotAvailable),
            Err(error) => {
                tracing::warn!(%error, "Machine browser runtime is not usable");
                return hibernated(Reason::NotAvailable);
            }
        };
        if let Err(unmet) = host::check(&plan.contract.host_requirements) {
            tracing::warn!(detail = %unmet.0, "browser open refused: host requirements");
            return hibernated(Reason::HostRequirements);
        }
        let settings = self.settings();
        if settings.max_instances == 0 || self.running().len() >= settings.max_instances as usize {
            return hibernated(Reason::Capacity);
        }
        if !settings.geometry_allowed(viewport.width, viewport.height) {
            self.send_result(
                request_id.to_owned(),
                false,
                Some("browser viewport exceeds this Device's bound".into()),
            );
            return hibernated(Reason::Failed);
        }
        let account_dir = match self.account_dir(&account_id) {
            Ok(dir) => dir,
            Err(error) => {
                tracing::warn!(%error, "browser account directory unavailable");
                return hibernated(Reason::Failed);
            }
        };
        let mut record = match state_file::read_control(&account_dir) {
            Ok(record) => record,
            Err(error) => {
                tracing::warn!(%error, "browser control record unreadable");
                return hibernated(Reason::Failed);
            }
        };
        if let Some(pending) = &record.pending {
            return hibernated(match pending.kind {
                OperationKind::Reset => Reason::Busy,
                OperationKind::Revoke => Reason::Revoked,
            });
        }
        match compare_versions(
            record.browser_version.as_deref(),
            &plan.contract.runtime.browser_version,
        ) {
            VersionOrder::RecordNewer => return hibernated(Reason::ProfileNewerThanRuntime),
            VersionOrder::Same => {}
            VersionOrder::RuntimeNewer | VersionOrder::Unrecorded => {
                record.browser_version = Some(plan.contract.runtime.browser_version.clone());
                record.generation_digest = Some(plan.generation_digest.clone());
                if let Err(error) = state_file::write_control(&account_dir, &record) {
                    tracing::warn!(%error, "browser control record not writable");
                    return hibernated(Reason::Failed);
                }
            }
        }
        if let Err(error) = snapshot::reconcile(&account_dir, &record, &BTreeSet::new()) {
            tracing::warn!(%error, "browser snapshot reconciliation failed");
            return hibernated(Reason::Failed);
        }
        let incarnation = u64::try_from(unix_ms()).unwrap_or(1).max(1);
        let mut deadline_ms = unix_ms()
            .saturating_add(i64::try_from(settings.max_runtime.as_millis()).unwrap_or(i64::MAX));
        let mut pending_navigation = None;
        if let Purpose::SignIn { login_request_id } = &purpose {
            match self.bind_login(login_request_id, &account_id, &instance_id, incarnation) {
                Ok((url, expires_at_ms)) => {
                    pending_navigation = url;
                    deadline_ms = deadline_ms.min(expires_at_ms);
                }
                Err(reason) => return hibernated(reason),
            }
        }
        let reservation = match LEDGER
            .reserve(
                settings.memory_max,
                self.headroom,
                || false,
                MEMORY_WAIT,
                MEMORY_POLL,
            )
            .await
        {
            Admission::Reserved { reservation, .. } => reservation,
            Admission::Exhausted | Admission::Cancelled => {
                self.unbind_login(&purpose, &instance_id);
                return hibernated(Reason::Memory);
            }
        };
        let started = self
            .start_instance(
                &account_dir,
                &record,
                Arc::clone(&plan),
                settings,
                &account_id,
                &instance_id,
                incarnation,
                purpose.clone(),
                viewport,
                deadline_ms,
            )
            .await;
        drop(reservation);
        let instance = match started {
            Ok(instance) => instance,
            Err(failure) => {
                self.unbind_login(&purpose, &instance_id);
                tracing::warn!(detail = %failure.detail, reason = ?failure.reason, "browser open failed");
                return hibernated(failure.reason);
            }
        };
        *slot.instance.lock() = Some(Arc::clone(&instance));
        if instance.login_request().is_some() {
            // The login may have ended while the unit started.
            if let Some(reason) = self.login_binding_broken(&instance) {
                return self
                    .stop_locked(&instance, reason, false, Some(request_id))
                    .await;
            }
            let url = pending_navigation.or_else(|| {
                instance
                    .login_request()
                    .and_then(|request| self.login_url(request))
            });
            if let Some(url) = url {
                self.navigate_verified(&instance, &url).await;
            }
        }
        self.spawn_supervisor(&instance);
        instance.state(Some(request_id.to_owned()))
    }

    #[allow(clippy::too_many_arguments)]
    async fn start_instance(
        &self,
        account_dir: &Path,
        record: &ControlRecord,
        plan: Arc<MachineBrowserRuntimePlan>,
        settings: Settings,
        account_id: &ProviderAccountId,
        instance_id: &str,
        incarnation: u64,
        purpose: Purpose,
        viewport: Viewport,
        deadline_ms: i64,
    ) -> Result<Arc<Instance>, Failure> {
        let unit_name = unit::unit_name(instance_id);
        let runtime_dir = unit::runtime_directory(instance_id);
        let staging_root = state_file::staging_root(account_dir);
        std::fs::create_dir_all(&staging_root).map_err(|error| Failure::internal(error.into()))?;
        for directory in [state_file::browser_root(account_dir), staging_root.clone()] {
            crate::provider_account::prepare_private_directory(&directory)
                .map_err(Failure::internal)?;
        }
        let snapshot_dir = snapshot::current_snapshot(account_dir, record);
        let arguments = launch_arguments(
            &plan,
            &settings,
            &runtime_dir,
            &staging_root,
            snapshot_dir.as_deref(),
            viewport,
            &self.state_dir,
        )
        .map_err(Failure::internal)?;
        let launch = Launch {
            unit: &unit_name,
            instance_id,
            limits: Limits {
                memory_max: settings.memory_max,
                memory_high: settings.memory_high,
                cpu_quota_percent: settings.cpu_quota_percent,
                tasks_max: settings.tasks_max,
                max_runtime: settings.max_runtime,
            },
            launcher: &plan.launcher,
            arguments,
        };
        unit::start(&launch).await.map_err(Failure::internal)?;
        let outcome = async {
            let status = unit::wait_running(&unit_name, UNIT_START_TIMEOUT)
                .await
                .map_err(Failure::internal)?;
            let cgroup_dir = status
                .cgroup_dir()
                .ok_or_else(|| Failure::internal(anyhow!("unit reports no cgroup")))?;
            let readiness = Duration::from_millis(plan.contract.launch.readiness_timeout_ms);
            let mut control = Control::connect(&runtime_dir, readiness)
                .await
                .map_err(Failure::internal)?;
            let report = control
                .wait_ready(readiness)
                .await
                .map_err(Failure::launcher)?;
            let bootstrap = host::enforce_label(BOOTSTRAP_PROFILE);
            let sealed = host::enforce_label(SEALED_PROFILE);
            let instance = Instance {
                account_id: account_id.clone(),
                instance_id: instance_id.to_owned(),
                incarnation,
                purpose,
                unit: unit_name.clone(),
                runtime_dir: runtime_dir.clone(),
                cgroup_dir,
                deadline: Instant::now()
                    + Duration::from_millis(
                        u64::try_from(deadline_ms.saturating_sub(unix_ms())).unwrap_or(0),
                    ),
                deadline_ms,
                scale: viewport.scale,
                profile_revision: record.profile_revision,
                plan: Arc::clone(&plan),
                settings,
                report: report.clone(),
                control: tokio::sync::Mutex::new(control),
                live: parking_lot::Mutex::new(Live {
                    phase: Phase::Starting,
                    geometry: Geometry {
                        width: viewport.width,
                        height: viewport.height,
                    },
                    verified: Verified {
                        supervisor: verify::Pin { pid: 0, start: 0 },
                        init: verify::Pin { pid: 0, start: 0 },
                        display_server: verify::Pin { pid: 0, start: 0 },
                        window_manager: verify::Pin { pid: 0, start: 0 },
                        browser: verify::Pin { pid: 0, start: 0 },
                    },
                    attachment: None,
                    detached_since: Some(Instant::now()),
                    last_snapshot: record
                        .current_generation
                        .zip(record.current_generation_at_ms),
                    last_periodic: Instant::now(),
                    last_scan: Instant::now(),
                    pending_navigation: None,
                    unsaved: false,
                }),
            };
            let verified = verify::verify(
                &instance.expectations(self.uid, self.gid, status.main_pid, &bootstrap, &sealed),
                &report,
            )
            .map_err(|violation| Failure {
                reason: Reason::ConfinementUnverified,
                detail: violation.0,
            })?;
            {
                let mut live = instance.live.lock();
                live.verified = verified;
                live.phase = Phase::Detached;
            }
            let instance = Arc::new(instance);
            state_file::write_instance(
                &self.state_dir,
                &InstanceRecord {
                    schema_version: 1,
                    instance_id: instance_id.to_owned(),
                    account_id: account_id.as_str().to_owned(),
                    purpose: instance.purpose.clone(),
                    plugin_id: plan.plugin_id.clone(),
                    plugin_version: plan.plugin_version.clone(),
                    generation_digest: plan.generation_digest.clone(),
                    unit: unit_name.clone(),
                    invocation_id: status.invocation_id.clone(),
                    boot_id: self.boot_id.clone(),
                    incarnation,
                    deadline_ms,
                    profile_revision: record.profile_revision,
                    started_at_ms: unix_ms(),
                    runtime_dir: runtime_dir.clone(),
                    cgroup_dir: instance.cgroup_dir.clone(),
                    geometry: Geometry {
                        width: viewport.width,
                        height: viewport.height,
                    },
                    scale: viewport.scale,
                    ready: report,
                },
            )
            .map_err(Failure::internal)?;
            Ok::<_, Failure>(instance)
        }
        .await;
        if outcome.is_err() {
            // Never retry with weaker isolation: the unit ends and the
            // reservation is refunded by the caller.
            if let Err(error) = unit::stop_and_wait(&unit_name).await {
                tracing::warn!(%error, unit = %unit_name, "failed browser unit did not stop cleanly");
            }
        }
        outcome
    }

    async fn attach_viewer(
        self: &Arc<Self>,
        request_id: &str,
        reference: InstanceRef,
        attachment_id: String,
        stream_token: String,
        viewport: Viewport,
    ) -> StateEvent {
        let instance = match self.current(&reference) {
            Ok(instance) => instance,
            Err(event) => return (*event).with_request(request_id),
        };
        if !matches!(
            instance.live.lock().phase,
            Phase::Detached | Phase::Attached
        ) {
            return instance
                .state(Some(request_id.to_owned()))
                .with_reason(Reason::Busy);
        }
        if let Err(violation) = self.recheck(&instance) {
            return self
                .stop_instance(&instance, violation, false, Some(request_id))
                .await;
        }
        let endpoint = match self
            .uplink
            .lock()
            .as_ref()
            .map(|uplink| uplink.endpoint.clone())
        {
            Some(endpoint) => endpoint,
            None => {
                return instance
                    .state(Some(request_id.to_owned()))
                    .with_reason(Reason::Failed);
            }
        };
        let current = instance.live.lock().geometry;
        if (current.width, current.height) != (viewport.width, viewport.height) {
            if !instance
                .settings
                .geometry_allowed(viewport.width, viewport.height)
            {
                self.send_result(
                    request_id.to_owned(),
                    false,
                    Some("browser viewport exceeds this Device's bound".into()),
                );
                return instance
                    .state(Some(request_id.to_owned()))
                    .with_reason(Reason::Failed);
            }
            let resized = instance
                .control
                .lock()
                .await
                .resize(viewport.width, viewport.height, COMMAND_TIMEOUT)
                .await;
            match resized {
                Ok(()) => {
                    instance.live.lock().geometry = Geometry {
                        width: viewport.width,
                        height: viewport.height,
                    };
                }
                Err(error) => {
                    tracing::warn!(%error, "browser resize at attach failed");
                }
            }
        }
        let weak = Arc::downgrade(self);
        let end_instance = Arc::downgrade(&instance);
        let end_attachment = attachment_id.clone();
        let stream = Stream::start(
            endpoint,
            stream_token,
            instance.runtime_dir.join(super::launcher::RFB_SOCKET),
            Box::new(move |end, _counters| {
                if let (Some(manager), Some(instance)) = (weak.upgrade(), end_instance.upgrade()) {
                    manager.stream_ended(&instance, &end_attachment, &end);
                }
            }),
        );
        {
            let mut live = instance.live.lock();
            if let Some(previous) = live.attachment.take()
                && let Some(previous_stream) = previous.stream
            {
                previous_stream.cancel();
            }
            live.attachment = Some(Attachment {
                id: attachment_id.clone(),
                lease_expires: Instant::now()
                    + Duration::from_millis(u64::try_from(LEASE_EXPIRY_MS).unwrap_or(30_000)),
                stream: Some(stream),
            });
            live.phase = Phase::Attached;
            live.detached_since = None;
        }
        if let Err(error) = instance
            .control
            .lock()
            .await
            .attach(&attachment_id, COMMAND_TIMEOUT)
            .await
        {
            tracing::debug!(%error, "launcher attach bookkeeping failed");
        }
        instance.state(Some(request_id.to_owned()))
    }

    fn stream_ended(
        self: &Arc<Self>,
        instance: &Arc<Instance>,
        attachment_id: &str,
        end: &StreamEnd,
    ) {
        let detached = {
            let mut live = instance.live.lock();
            if live
                .attachment
                .as_ref()
                .is_some_and(|attachment| attachment.id == attachment_id)
            {
                live.attachment = None;
                if live.phase == Phase::Attached {
                    live.phase = Phase::Detached;
                    live.detached_since = Some(Instant::now());
                }
                true
            } else {
                false
            }
        };
        if detached {
            tracing::info!(?end, instance = %instance.instance_id, "browser viewer detached");
            self.send_state(instance.state(None));
        }
    }

    fn lease(
        &self,
        request_id: &str,
        reference: &InstanceRef,
        attachment_id: &str,
        expires_at_ms: i64,
    ) -> Option<StateEvent> {
        let instance = match self.current(reference) {
            Ok(instance) => instance,
            Err(event) => return Some((*event).with_request(request_id)),
        };
        let now = Instant::now();
        let mut live = instance.live.lock();
        let Some(attachment) = live.attachment.as_mut() else {
            return Some(
                instance
                    .state(Some(request_id.to_owned()))
                    .with_reason(Reason::Superseded),
            );
        };
        if attachment.id != attachment_id || attachment.lease_expires <= now {
            // Expiry is monotonic: a late or stale renewal cannot resurrect.
            return Some(
                instance
                    .state(Some(request_id.to_owned()))
                    .with_reason(Reason::Superseded),
            );
        }
        let bound = now + Duration::from_millis(u64::try_from(LEASE_EXPIRY_MS).unwrap_or(30_000));
        let requested = now
            + Duration::from_millis(
                u64::try_from(expires_at_ms.saturating_sub(unix_ms())).unwrap_or(0),
            );
        attachment.lease_expires = bound.min(requested).max(now);
        None
    }

    fn detach(&self, request_id: &str, reference: &InstanceRef, attachment_id: &str) -> StateEvent {
        let instance = match self.current(reference) {
            Ok(instance) => instance,
            Err(event) => return (*event).with_request(request_id),
        };
        let mut live = instance.live.lock();
        if live
            .attachment
            .as_ref()
            .is_some_and(|attachment| attachment.id == attachment_id)
        {
            if let Some(attachment) = live.attachment.take()
                && let Some(stream) = attachment.stream
            {
                stream.cancel();
            }
            live.phase = Phase::Detached;
            live.detached_since = Some(Instant::now());
            drop(live);
            return instance.state(Some(request_id.to_owned()));
        }
        drop(live);
        instance
            .state(Some(request_id.to_owned()))
            .with_reason(Reason::Superseded)
    }

    async fn resize(
        self: &Arc<Self>,
        request_id: &str,
        reference: &InstanceRef,
        attachment_id: &str,
        width: u32,
        height: u32,
    ) -> StateEvent {
        let instance = match self.current(reference) {
            Ok(instance) => instance,
            Err(event) => return (*event).with_request(request_id),
        };
        let attached = instance
            .live
            .lock()
            .attachment
            .as_ref()
            .is_some_and(|attachment| attachment.id == attachment_id);
        if !attached {
            return instance
                .state(Some(request_id.to_owned()))
                .with_reason(Reason::Superseded);
        }
        if !instance.settings.geometry_allowed(width, height) {
            self.send_result(
                request_id.to_owned(),
                false,
                Some("browser geometry exceeds this Device's bound".into()),
            );
            return instance
                .state(Some(request_id.to_owned()))
                .with_reason(Reason::Failed);
        }
        if let Err(violation) = self.recheck(&instance) {
            return self
                .stop_instance(&instance, violation, false, Some(request_id))
                .await;
        }
        let resized = instance
            .control
            .lock()
            .await
            .resize(width, height, COMMAND_TIMEOUT)
            .await;
        match resized {
            Ok(()) => {
                instance.live.lock().geometry = Geometry { width, height };
                instance.state(Some(request_id.to_owned()))
            }
            Err(error) => {
                tracing::warn!(%error, "browser resize failed");
                instance
                    .state(Some(request_id.to_owned()))
                    .with_reason(Reason::Failed)
            }
        }
    }

    async fn navigate(
        self: &Arc<Self>,
        request_id: &str,
        reference: &InstanceRef,
        url: &str,
    ) -> Result<StateEvent, String> {
        let instance = match self.current(reference) {
            Ok(instance) => instance,
            Err(event) => return Ok((*event).with_request(request_id)),
        };
        if instance.purpose != Purpose::Tab {
            return Err("only tab browsers accept owner navigation".into());
        }
        self.navigate_verified(&instance, url).await;
        Ok(instance.state(Some(request_id.to_owned())))
    }

    /// Full confinement verification, then navigation. A violation stops
    /// the instance instead of navigating.
    async fn navigate_verified(self: &Arc<Self>, instance: &Arc<Instance>, url: &str) {
        let bootstrap = host::enforce_label(BOOTSTRAP_PROFILE);
        let sealed = host::enforce_label(SEALED_PROFILE);
        let supervisor = instance.live.lock().verified.supervisor.pid;
        let verified = verify::verify(
            &instance.expectations(self.uid, self.gid, supervisor, &bootstrap, &sealed),
            &instance.report,
        );
        match verified {
            Ok(verified) => instance.live.lock().verified = verified,
            Err(violation) => {
                tracing::warn!(detail = %violation, "browser confinement failed before navigation");
                let event = self
                    .stop_instance(instance, Reason::ConfinementUnverified, false, None)
                    .await;
                self.send_state(event);
                return;
            }
        }
        let navigated = instance
            .control
            .lock()
            .await
            .navigate(url, COMMAND_TIMEOUT)
            .await;
        match navigated {
            Ok(()) => instance.live.lock().unsaved = true,
            Err(error) => tracing::warn!(%error, "browser navigation failed"),
        }
    }

    async fn snapshot_now(
        self: &Arc<Self>,
        request_id: &str,
        reference: &InstanceRef,
    ) -> StateEvent {
        let instance = match self.current(reference) {
            Ok(instance) => instance,
            Err(event) => return (*event).with_request(request_id),
        };
        let slot = self.slot(&instance.account_id);
        let _lifecycle = slot.lifecycle.lock().await;
        match self.frozen_snapshot(&instance).await {
            Ok(()) => instance.state(Some(request_id.to_owned())),
            Err(error) => {
                tracing::warn!(%error, "browser snapshot failed");
                instance
                    .state(Some(request_id.to_owned()))
                    .with_reason(Reason::StorageExceeded)
            }
        }
    }

    /// Freeze the browser sub-cgroup, let the launcher copy the profile,
    /// thaw on every path, then validate and publish. Caller holds the
    /// account lifecycle lock.
    async fn frozen_snapshot(&self, instance: &Arc<Instance>) -> Result<()> {
        ensure!(
            matches!(
                instance.live.lock().phase,
                Phase::Detached | Phase::Attached
            ),
            "browser is not running"
        );
        let account_dir = self.account_dir(&instance.account_id)?;
        let mut record = state_file::read_control(&account_dir)?;
        ensure!(
            record.pending.is_none() && record.profile_revision == instance.profile_revision,
            "browser profile was reset since this instance started"
        );
        let generation = record.allocate_generation();
        state_file::write_control(&account_dir, &record)?;
        let reservation =
            self.admit_staging(&instance.account_id, generation, &instance.settings)?;
        snapshot::create_staging(&account_dir, generation)?;
        let browser_cgroup = instance.cgroup_dir.join("browser");
        unit::set_frozen(&browser_cgroup, true)?;
        let copied = async {
            ensure!(
                unit::wait_frozen(&browser_cgroup, FREEZE_TIMEOUT).await?,
                "browser did not freeze in time"
            );
            instance
                .control
                .lock()
                .await
                .snapshot(generation, SNAPSHOT_TIMEOUT)
                .await
        }
        .await;
        unit::set_frozen(&browser_cgroup, false)?;
        copied?;
        self.publish_snapshot(instance, &account_dir, generation, reservation, true)
    }

    fn publish_snapshot(
        &self,
        instance: &Arc<Instance>,
        account_dir: &Path,
        generation: u64,
        reservation: StagingReservation,
        flushed: bool,
    ) -> Result<()> {
        let mut record = state_file::read_control(account_dir)?;
        ensure!(
            record.pending.is_none() && record.profile_revision == instance.profile_revision,
            "browser profile was reset during the snapshot; staging discarded"
        );
        let stats = snapshot::publish(
            account_dir,
            generation,
            &mut record,
            snapshot::Bounds::for_profile(instance.settings.profile_max_bytes),
            unix_ms(),
        )?;
        drop(reservation);
        let mut live = instance.live.lock();
        live.last_snapshot = Some((generation, unix_ms()));
        live.last_periodic = Instant::now();
        if flushed {
            live.unsaved = false;
        }
        tracing::info!(
            instance = %instance.instance_id,
            generation,
            bytes = stats.bytes,
            entries = stats.entries,
            "browser profile snapshot published"
        );
        Ok(())
    }

    fn admit_staging(
        &self,
        account_id: &ProviderAccountId,
        generation: u64,
        settings: &Settings,
    ) -> Result<StagingReservation> {
        self.staging
            .admit(
                account_id.as_str(),
                generation,
                settings.profile_max_bytes,
                settings,
                snapshot::free_bytes(&self.providers.accounts_root()),
            )
            .map_err(|refusal| anyhow!("snapshot staging refused: {refusal:?}"))
    }

    async fn hibernate(self: &Arc<Self>, request_id: &str, reference: &InstanceRef) -> StateEvent {
        match self.current(reference) {
            Ok(instance) => {
                self.stop_instance(&instance, Reason::OwnerRequest, true, Some(request_id))
                    .await
            }
            Err(event) => (*event).with_request(request_id),
        }
    }

    /// Stop with the account lifecycle lock taken here.
    async fn stop_instance(
        &self,
        instance: &Arc<Instance>,
        reason: Reason,
        graceful: bool,
        request_id: Option<&str>,
    ) -> StateEvent {
        let slot = self.slot(&instance.account_id);
        let _lifecycle = slot.lifecycle.lock().await;
        self.stop_locked(instance, reason, graceful, request_id)
            .await
    }

    /// Stop while the caller holds the account lifecycle lock. A graceful
    /// stop lets the browser flush and copies the profile at the barrier; a
    /// forced stop keeps the last published generation and reports that
    /// unsaved state was lost.
    async fn stop_locked(
        &self,
        instance: &Arc<Instance>,
        reason: Reason,
        graceful: bool,
        request_id: Option<&str>,
    ) -> StateEvent {
        let already_stopping = {
            let mut live = instance.live.lock();
            let was = live.phase == Phase::Stopping;
            live.phase = Phase::Stopping;
            if let Some(attachment) = live.attachment.take()
                && let Some(stream) = attachment.stream
            {
                stream.cancel();
            }
            was
        };
        let slot = self.slot(&instance.account_id);
        if already_stopping {
            // Another path is stopping it; report the hibernated outcome
            // once the unit is gone rather than racing the stop.
            let _ =
                unit::wait_gone(&instance.unit, unit::STOP_TIMEOUT + Duration::from_secs(10)).await;
            return self.hibernated_after(instance, reason, request_id, false);
        }
        self.send_state(instance.state(None).with_reason(reason));
        let mut unsaved = instance.live.lock().unsaved;
        let mut published = None;
        if graceful {
            match self.stop_with_snapshot(instance).await {
                Ok(Some((generation, flushed))) => {
                    published = Some(generation);
                    unsaved = !flushed;
                }
                Ok(None) => unsaved = true,
                Err(error) => {
                    tracing::warn!(%error, "browser graceful stop did not publish a snapshot");
                    unsaved = true;
                }
            }
        }
        match unit::stop_and_wait(&instance.unit).await {
            Ok(true) => {}
            Ok(false) => {
                tracing::error!(unit = %instance.unit, "browser unit is stuck after SIGKILL")
            }
            Err(error) => tracing::warn!(%error, unit = %instance.unit, "browser unit stop failed"),
        }
        if let Err(error) = state_file::remove_instance(&self.state_dir, &instance.instance_id) {
            tracing::warn!(%error, "browser instance record not removed");
        }
        {
            let mut current = slot.instance.lock();
            if current
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, instance))
            {
                *current = None;
            }
        }
        if let Some(generation) = published {
            instance.live.lock().last_snapshot = Some((generation, unix_ms()));
        }
        self.hibernated_after(instance, reason, request_id, unsaved)
    }

    fn hibernated_after(
        &self,
        instance: &Arc<Instance>,
        reason: Reason,
        request_id: Option<&str>,
        unsaved: bool,
    ) -> StateEvent {
        let last_snapshot = instance.live.lock().last_snapshot;
        StateEvent {
            request_id: request_id.map(ToOwned::to_owned),
            instance: instance.reference(),
            state: State::Hibernated,
            reason: Some(reason),
            geometry: None,
            scale: None,
            deadline_ms: None,
            attachment_id: None,
            snapshot_generation: last_snapshot.map(|(generation, _)| generation),
            snapshot_at_ms: last_snapshot.map(|(_, at)| at),
            profile_revision: Some(instance.profile_revision),
            unsaved_state_lost: unsaved,
            observed_at_ms: unix_ms(),
        }
    }

    /// Graceful stop through the launcher with a stop-time copy. Returns the
    /// published generation and whether the browser exited gracefully (the
    /// copy is flushed) or was killed (crash-consistent).
    async fn stop_with_snapshot(&self, instance: &Arc<Instance>) -> Result<Option<(u64, bool)>> {
        let account_dir = self.account_dir(&instance.account_id)?;
        let mut record = state_file::read_control(&account_dir)?;
        if record.pending.is_some() || record.profile_revision != instance.profile_revision {
            let _ = instance
                .control
                .lock()
                .await
                .stop(GRACEFUL_WAIT, None, unit::STOP_TIMEOUT)
                .await;
            return Ok(None);
        }
        let generation = record.allocate_generation();
        state_file::write_control(&account_dir, &record)?;
        let reservation =
            match self.admit_staging(&instance.account_id, generation, &instance.settings) {
                Ok(reservation) => reservation,
                Err(error) => {
                    tracing::warn!(%error, "stop-time snapshot refused");
                    let _ = instance
                        .control
                        .lock()
                        .await
                        .stop(GRACEFUL_WAIT, None, unit::STOP_TIMEOUT)
                        .await;
                    return Ok(None);
                }
            };
        snapshot::create_staging(&account_dir, generation)?;
        let outcome = instance
            .control
            .lock()
            .await
            .stop(GRACEFUL_WAIT, Some(generation), unit::STOP_TIMEOUT)
            .await?;
        if !outcome.copied {
            bail!("launcher made no stop-time copy");
        }
        self.publish_snapshot(
            instance,
            &account_dir,
            generation,
            reservation,
            outcome.graceful,
        )?;
        Ok(Some((generation, outcome.graceful)))
    }

    async fn reset_or_revoke(
        &self,
        request_id: &str,
        account_id: ProviderAccountId,
        kind: OperationKind,
        operation_id: &str,
        expected_revision: Option<u64>,
    ) -> StateEvent {
        let slot = self.slot(&account_id);
        let _lifecycle = slot.lifecycle.lock().await;
        let hibernated = |reason: Option<Reason>, revision: Option<u64>| StateEvent {
            request_id: Some(request_id.to_owned()),
            instance: InstanceRef {
                account_id: account_id.clone(),
                instance_id: String::new(),
                incarnation: 0,
            },
            state: State::Hibernated,
            reason,
            geometry: None,
            scale: None,
            deadline_ms: None,
            attachment_id: None,
            snapshot_generation: None,
            snapshot_at_ms: None,
            profile_revision: revision,
            unsaved_state_lost: false,
            observed_at_ms: unix_ms(),
        };
        let account_dir = match self.account_dir(&account_id) {
            Ok(dir) => dir,
            Err(error) => {
                tracing::warn!(%error, "browser account directory unavailable");
                return hibernated(Some(Reason::Failed), None);
            }
        };
        let mut record = match state_file::read_control(&account_dir) {
            Ok(record) => record,
            Err(error) => {
                tracing::warn!(%error, "browser control record unreadable");
                return hibernated(Some(Reason::Failed), None);
            }
        };
        if record
            .last_operation
            .as_ref()
            .is_some_and(|done| done.operation_id == operation_id)
        {
            return hibernated(None, Some(record.profile_revision));
        }
        match &record.pending {
            Some(pending) if pending.operation_id != operation_id => {
                return hibernated(Some(Reason::Busy), Some(record.profile_revision));
            }
            Some(_) => {}
            None => {
                if let Some(expected) = expected_revision
                    && expected != record.profile_revision
                {
                    return hibernated(Some(Reason::Superseded), Some(record.profile_revision));
                }
                record.pending = Some(PendingOperation {
                    kind,
                    operation_id: operation_id.to_owned(),
                    next_profile_revision: record.profile_revision + 1,
                    started_at_ms: unix_ms(),
                });
                if let Err(error) = state_file::write_control(&account_dir, &record) {
                    tracing::warn!(%error, "browser fence not writable");
                    return hibernated(Some(Reason::Failed), Some(record.profile_revision));
                }
            }
        }
        let reason = match kind {
            OperationKind::Reset => Reason::OwnerRequest,
            OperationKind::Revoke => Reason::Revoked,
        };
        let running = slot.instance.lock().clone();
        if let Some(instance) = running {
            // The fence is raised: the copy, if any, is interrupted with the
            // unit and its staging abandoned.
            let event = self.stop_locked(&instance, reason, false, None).await;
            self.send_state(event);
        }
        match self.complete_fenced(&account_dir, &mut record) {
            Ok(()) => hibernated(Some(reason), Some(record.profile_revision)),
            Err(error) => {
                tracing::warn!(%error, "browser reset or revocation incomplete; fence retained");
                hibernated(Some(Reason::Failed), Some(record.profile_revision))
            }
        }
    }

    /// Delete the profile tree and lower the fence; the control record
    /// survives with the advanced revision.
    fn complete_fenced(&self, account_dir: &Path, record: &mut ControlRecord) -> Result<()> {
        let pending = record
            .pending
            .clone()
            .context("no fenced operation to complete")?;
        snapshot::remove_browser_dir(account_dir)?;
        record.profile_revision = pending.next_profile_revision;
        record.current_generation = None;
        record.current_generation_at_ms = None;
        record.pending = None;
        record.last_operation = Some(CompletedOperation {
            kind: pending.kind,
            operation_id: pending.operation_id,
            profile_revision: record.profile_revision,
            completed_at_ms: unix_ms(),
        });
        state_file::write_control(account_dir, record)
    }

    fn query(&self, request_id: &str, account_id: &ProviderAccountId) -> StateEvent {
        let slot = self.slot(account_id);
        let running = slot.instance.lock().clone();
        if let Some(instance) = running {
            return instance.state(Some(request_id.to_owned()));
        }
        let record = self
            .account_dir(account_id)
            .and_then(|dir| state_file::read_control(&dir))
            .unwrap_or_default();
        StateEvent {
            request_id: Some(request_id.to_owned()),
            instance: InstanceRef {
                account_id: account_id.clone(),
                instance_id: String::new(),
                incarnation: 0,
            },
            state: State::Hibernated,
            reason: record.pending.as_ref().map(|pending| match pending.kind {
                OperationKind::Reset => Reason::Busy,
                OperationKind::Revoke => Reason::Revoked,
            }),
            geometry: None,
            scale: None,
            deadline_ms: None,
            attachment_id: None,
            snapshot_generation: record.current_generation,
            snapshot_at_ms: record.current_generation_at_ms,
            profile_revision: Some(record.profile_revision),
            unsaved_state_lost: false,
            observed_at_ms: unix_ms(),
        }
    }

    // ---- supervision -----------------------------------------------------

    fn spawn_supervisor(self: &Arc<Self>, instance: &Arc<Instance>) {
        let manager = Arc::downgrade(self);
        let instance = Arc::clone(instance);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(SUPERVISOR_TICK);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                let Some(manager) = manager.upgrade() else {
                    return;
                };
                if manager.supervise(&instance).await {
                    return;
                }
            }
        });
    }

    /// One supervision tick; `true` once the instance is gone.
    async fn supervise(self: &Arc<Self>, instance: &Arc<Instance>) -> bool {
        let now = Instant::now();
        let (phase, lease_expired, grace_expired, scan_due, snapshot_due) = {
            let live = instance.live.lock();
            (
                live.phase,
                live.attachment
                    .as_ref()
                    .is_some_and(|attachment| attachment.lease_expires <= now),
                live.detached_since
                    .is_some_and(|since| since + instance.settings.hibernate_after <= now),
                live.last_scan + SOCKET_SCAN_INTERVAL <= now,
                live.phase == Phase::Attached
                    && live.last_periodic + instance.settings.snapshot_interval <= now,
            )
        };
        if phase == Phase::Stopping {
            return true;
        }
        if now >= instance.deadline {
            let event = self
                .stop_instance(instance, Reason::Deadline, false, None)
                .await;
            self.send_state(event);
            return true;
        }
        if let Some(reason) = self.login_binding_broken(instance) {
            let event = self.stop_instance(instance, reason, true, None).await;
            self.send_state(event);
            return true;
        }
        if lease_expired {
            let mut live = instance.live.lock();
            if let Some(attachment) = live.attachment.take()
                && let Some(stream) = attachment.stream
            {
                stream.cancel();
            }
            live.phase = Phase::Detached;
            live.detached_since = Some(now);
            drop(live);
            self.send_state(instance.state(None));
        }
        if grace_expired && instance.live.lock().phase == Phase::Detached {
            let event = self
                .stop_instance(instance, Reason::Grace, true, None)
                .await;
            self.send_state(event);
            return true;
        }
        if scan_due {
            instance.live.lock().last_scan = now;
            if let Err(reason) = self.recheck(instance) {
                let event = self.stop_instance(instance, reason, false, None).await;
                self.send_state(event);
                return true;
            }
        }
        if let Ok(mut control) = instance.control.try_lock() {
            match control.next_event(Duration::from_millis(20)).await {
                Ok(Some(LauncherEvent::Exited { process, code })) => {
                    drop(control);
                    tracing::warn!(instance = %instance.instance_id, %process, ?code, "browser process exited");
                    let event = self
                        .stop_instance(instance, Reason::Failed, false, None)
                        .await;
                    self.send_state(event);
                    return true;
                }
                Ok(Some(LauncherEvent::Error { kind, message })) => {
                    tracing::warn!(instance = %instance.instance_id, ?kind, %message, "launcher reported an error");
                }
                Ok(_) => {}
                Err(error) => {
                    drop(control);
                    tracing::warn!(%error, instance = %instance.instance_id, "launcher control socket failed");
                    let event = self
                        .stop_instance(instance, Reason::Failed, false, None)
                        .await;
                    self.send_state(event);
                    return true;
                }
            }
        }
        if snapshot_due {
            let slot = self.slot(&instance.account_id);
            let _lifecycle = slot.lifecycle.lock().await;
            if let Err(error) = self.frozen_snapshot(instance).await {
                tracing::warn!(%error, instance = %instance.instance_id, "periodic browser snapshot failed");
                instance.live.lock().last_periodic = now;
            }
        }
        let pending = instance.live.lock().pending_navigation.take();
        if let Some(url) = pending {
            self.navigate_verified(instance, &url).await;
        }
        false
    }

    /// Cheap periodic evidence: pins, socket scan and runtime directory.
    fn recheck(&self, instance: &Arc<Instance>) -> Result<(), Reason> {
        if !instance.live.lock().verified.pins_hold() {
            return Err(Reason::ConfinementUnverified);
        }
        if let Err(violation) = verify::scan_sockets(&instance.cgroup_dir) {
            tracing::warn!(detail = %violation, "browser socket scan violation");
            return Err(Reason::ListenerDetected);
        }
        if let Err(violation) = verify::runtime_directory_clean(&instance.runtime_dir) {
            tracing::warn!(detail = %violation, "browser runtime directory violation");
            return Err(Reason::ConfinementUnverified);
        }
        Ok(())
    }

    // ---- sign-in binding -------------------------------------------------

    fn bind_login(
        &self,
        request_id: &str,
        account_id: &ProviderAccountId,
        instance_id: &str,
        incarnation: u64,
    ) -> Result<(Option<String>, i64), Reason> {
        let logins = self
            .uplink
            .lock()
            .as_ref()
            .and_then(|uplink| uplink.logins.upgrade())
            .ok_or(Reason::LoginMismatch)?;
        let mut logins = logins.lock();
        let login = logins.get_mut(request_id).ok_or(Reason::LoginMismatch)?;
        if login.account.as_ref() != Some(account_id)
            || login.provider != "claude-code"
            || login
                .browser
                .as_ref()
                .is_some_and(|(bound, _)| bound != instance_id)
        {
            return Err(Reason::LoginMismatch);
        }
        login.browser = Some((instance_id.to_owned(), incarnation));
        Ok((login.verification_url.clone(), login.expires_at_ms))
    }

    fn unbind_login(&self, purpose: &Purpose, instance_id: &str) {
        let Purpose::SignIn { login_request_id } = purpose else {
            return;
        };
        let Some(logins) = self
            .uplink
            .lock()
            .as_ref()
            .and_then(|uplink| uplink.logins.upgrade())
        else {
            return;
        };
        if let Some(login) = logins.lock().get_mut(login_request_id)
            && login
                .browser
                .as_ref()
                .is_some_and(|(bound, _)| bound == instance_id)
        {
            login.browser = None;
        }
    }

    /// `Some(reason)` when a sign-in instance's login is no longer the active
    /// login it was bound to.
    fn login_binding_broken(&self, instance: &Arc<Instance>) -> Option<Reason> {
        let request = instance.login_request()?;
        let logins = self
            .uplink
            .lock()
            .as_ref()
            .and_then(|uplink| uplink.logins.upgrade());
        let Some(logins) = logins else {
            return Some(Reason::LoginEnded);
        };
        let logins = logins.lock();
        let Some(login) = logins.get(request) else {
            return Some(Reason::LoginEnded);
        };
        let bound = login.browser.as_ref().is_some_and(|(bound, incarnation)| {
            bound == &instance.instance_id && *incarnation == instance.incarnation
        });
        (!bound || login.account.as_ref() != Some(&instance.account_id))
            .then_some(Reason::LoginMismatch)
    }

    fn login_url(&self, request_id: &str) -> Option<String> {
        let logins = self
            .uplink
            .lock()
            .as_ref()
            .and_then(|uplink| uplink.logins.upgrade())?;
        let logins = logins.lock();
        logins.get(request_id)?.verification_url.clone()
    }

    fn bound_instance(&self, request_id: &str) -> Option<Arc<Instance>> {
        self.running()
            .into_iter()
            .find(|instance| instance.login_request() == Some(request_id))
    }

    // ---- reconciliation --------------------------------------------------

    async fn reconcile_units(self: &Arc<Self>) -> Result<()> {
        let records = state_file::read_instances(&self.state_dir)?;
        let mut recorded = BTreeSet::new();
        for record in records {
            recorded.insert(record.unit.clone());
            if let Err(error) = self.reconcile_record(record).await {
                tracing::warn!(%error, "browser instance record reconciliation failed");
            }
        }
        for unit_name in unit::list_units().await? {
            if recorded.contains(&unit_name) {
                continue;
            }
            tracing::warn!(unit = %unit_name, "stopping unrecorded browser unit");
            thaw_browser_cgroup(&unit_name).await;
            unit::stop_and_wait(&unit_name).await?;
        }
        Ok(())
    }

    async fn reconcile_record(self: &Arc<Self>, record: InstanceRecord) -> Result<()> {
        let discard = |why: &str| {
            tracing::info!(instance = %record.instance_id, why, "clearing browser instance record");
            state_file::remove_instance(&self.state_dir, &record.instance_id)
        };
        if record.boot_id != self.boot_id {
            return discard("other boot");
        }
        let status = unit::show(&record.unit).await?;
        if status.gone() || !status.live() || status.invocation_id != record.invocation_id {
            if !status.gone() {
                unit::stop_and_wait(&record.unit).await?;
            }
            return discard("unit ended or replaced");
        }
        // A sign-in instance's login did not survive the restart.
        if record.purpose != Purpose::Tab {
            thaw_browser_cgroup(&record.unit).await;
            unit::stop_and_wait(&record.unit).await?;
            return discard("sign-in login ended with the Machine restart");
        }
        match self.adopt(&record, &status).await {
            Ok(instance) => {
                *self.slot(&instance.account_id).instance.lock() = Some(Arc::clone(&instance));
                self.spawn_supervisor(&instance);
                tracing::info!(instance = %record.instance_id, "adopted surviving browser instance");
                Ok(())
            }
            Err(error) => {
                tracing::warn!(%error, instance = %record.instance_id, "browser instance not adopted; stopping");
                thaw_browser_cgroup(&record.unit).await;
                unit::stop_and_wait(&record.unit).await?;
                discard("adoption failed")
            }
        }
    }

    async fn adopt(
        &self,
        record: &InstanceRecord,
        status: &unit::UnitStatus,
    ) -> Result<Arc<Instance>> {
        let plan = self
            .providers
            .machine_browser_runtime()?
            .context("no Machine browser runtime installed")?;
        ensure!(
            plan.generation_digest == record.generation_digest,
            "installed generation changed"
        );
        host::check(&plan.contract.host_requirements).map_err(|unmet| anyhow!(unmet.0))?;
        let account_id = ProviderAccountId::try_from(record.account_id.clone())
            .map_err(|error| anyhow!("{error}"))?;
        let account_dir = self.account_dir(&account_id)?;
        let control_record = state_file::read_control(&account_dir)?;
        ensure!(control_record.pending.is_none(), "account is fenced");
        ensure!(
            control_record.profile_revision == record.profile_revision,
            "profile revision advanced"
        );
        ensure!(record.deadline_ms > unix_ms(), "deadline passed");
        let settings = self.settings();
        let reservation = match LEDGER
            .reserve(
                settings.memory_max,
                self.headroom,
                || false,
                Duration::ZERO,
                MEMORY_POLL,
            )
            .await
        {
            Admission::Reserved { reservation, .. } => reservation,
            _ => bail!("no memory headroom to adopt"),
        };
        let readiness = Duration::from_millis(plan.contract.launch.readiness_timeout_ms);
        let mut control = Control::connect(&record.runtime_dir, Duration::from_secs(5)).await?;
        let mut report = control.status(COMMAND_TIMEOUT).await?;
        let waited = Instant::now();
        while report.snapshot_in_progress && waited.elapsed() < readiness {
            tokio::time::sleep(Duration::from_millis(500)).await;
            report = control.status(COMMAND_TIMEOUT).await?;
        }
        ensure!(
            !report.snapshot_in_progress,
            "interrupted snapshot did not finish"
        );
        let browser_cgroup = record.cgroup_dir.join("browser");
        if unit::is_frozen(&browser_cgroup).unwrap_or(false) {
            unit::set_frozen(&browser_cgroup, false)?;
        }
        let plan = Arc::new(plan);
        let bootstrap = host::enforce_label(BOOTSTRAP_PROFILE);
        let sealed = host::enforce_label(SEALED_PROFILE);
        let instance = Instance {
            account_id,
            instance_id: record.instance_id.clone(),
            incarnation: record.incarnation,
            purpose: record.purpose.clone(),
            unit: record.unit.clone(),
            runtime_dir: record.runtime_dir.clone(),
            cgroup_dir: record.cgroup_dir.clone(),
            deadline: Instant::now()
                + Duration::from_millis(
                    u64::try_from(record.deadline_ms.saturating_sub(unix_ms())).unwrap_or(0),
                ),
            deadline_ms: record.deadline_ms,
            scale: record.scale,
            profile_revision: record.profile_revision,
            plan,
            settings,
            report: report.clone(),
            control: tokio::sync::Mutex::new(control),
            live: parking_lot::Mutex::new(Live {
                phase: Phase::Detached,
                geometry: record.geometry,
                verified: Verified {
                    supervisor: verify::Pin { pid: 0, start: 0 },
                    init: verify::Pin { pid: 0, start: 0 },
                    display_server: verify::Pin { pid: 0, start: 0 },
                    window_manager: verify::Pin { pid: 0, start: 0 },
                    browser: verify::Pin { pid: 0, start: 0 },
                },
                attachment: None,
                detached_since: Some(Instant::now()),
                last_snapshot: control_record
                    .current_generation
                    .zip(control_record.current_generation_at_ms),
                last_periodic: Instant::now(),
                last_scan: Instant::now(),
                pending_navigation: None,
                unsaved: true,
            }),
        };
        let verified = verify::verify(
            &instance.expectations(self.uid, self.gid, status.main_pid, &bootstrap, &sealed),
            &report,
        )
        .map_err(|violation| anyhow!("confinement: {violation}"))?;
        instance.live.lock().verified = verified;
        drop(reservation);
        Ok(Arc::new(instance))
    }

    /// Resume fenced operations and remove orphan generations and abandoned
    /// staging of every account, with the control records as authority.
    async fn reconcile_accounts(&self) -> Result<()> {
        let accounts = self.providers.accounts_root();
        let entries = match std::fs::read_dir(&accounts) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        for entry in entries {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let account_dir = entry.path();
            if !account_dir.join(state_file::CONTROL_FILE).is_file() {
                continue;
            }
            let Ok(account_id) =
                ProviderAccountId::try_from(entry.file_name().to_string_lossy().into_owned())
            else {
                continue;
            };
            let slot = self.slot(&account_id);
            let _lifecycle = slot.lifecycle.lock().await;
            let mut record = state_file::read_control(&account_dir)?;
            if record.pending.is_some() {
                let running = slot.instance.lock().clone();
                if let Some(instance) = running {
                    let event = self
                        .stop_locked(&instance, Reason::Revoked, false, None)
                        .await;
                    self.send_state(event);
                }
                self.complete_fenced(&account_dir, &mut record)?;
                tracing::info!(account = %account_id.as_str(), "resumed fenced browser operation");
            }
            let live: BTreeSet<u64> = self
                .staging
                .live()
                .get(account_id.as_str())
                .copied()
                .into_iter()
                .collect();
            let removed = snapshot::reconcile(&account_dir, &record, &live)?;
            if !removed.is_empty() {
                tracing::info!(account = %account_id.as_str(), ?removed, "reconciled browser snapshot storage");
            }
        }
        Ok(())
    }

    // ---- plumbing --------------------------------------------------------

    fn settings(&self) -> Settings {
        Settings::from_config(self.device_config.as_ref())
    }

    fn slot(&self, account_id: &ProviderAccountId) -> Arc<AccountSlot> {
        Arc::clone(
            self.accounts
                .lock()
                .entry(account_id.as_str().to_owned())
                .or_insert_with(|| {
                    Arc::new(AccountSlot {
                        lifecycle: tokio::sync::Mutex::new(()),
                        instance: parking_lot::Mutex::new(None),
                    })
                }),
        )
    }

    fn running(&self) -> Vec<Arc<Instance>> {
        self.accounts
            .lock()
            .values()
            .filter_map(|slot| slot.instance.lock().clone())
            .collect()
    }

    /// The running instance `reference` names, or the hibernated or
    /// superseded observation to answer with.
    fn current(&self, reference: &InstanceRef) -> Result<Arc<Instance>, Box<StateEvent>> {
        let slot = self.slot(&reference.account_id);
        let instance = slot.instance.lock().clone();
        match instance {
            Some(instance)
                if instance.instance_id == reference.instance_id
                    && instance.incarnation == reference.incarnation =>
            {
                Ok(instance)
            }
            Some(instance) => Err(Box::new(
                instance.state(None).with_reason(Reason::Superseded),
            )),
            None => Err(Box::new(hibernated_event(
                "",
                &reference.account_id,
                &reference.instance_id,
                Reason::Superseded,
                None,
            ))),
        }
    }

    fn account_dir(&self, account_id: &ProviderAccountId) -> Result<PathBuf> {
        ensure!(!account_id.is_default(), "browser requires a named account");
        self.providers.for_account(account_id)?;
        Ok(self.providers.accounts_root().join(account_id.as_str()))
    }

    fn send_state(&self, event: StateEvent) {
        if let Some(uplink) = self.uplink.lock().as_ref() {
            let _ = uplink.events.send(MachineEvent::Browser {
                browser: Event::State(event),
            });
        }
    }

    fn send_result(&self, request_id: String, accepted: bool, detail: Option<String>) {
        if let Some(uplink) = self.uplink.lock().as_ref() {
            let _ = uplink.events.send(MachineEvent::CommandResult {
                request_id,
                accepted,
                detail,
            });
        }
    }

    fn receipt(&self, request_id: &str) -> Option<StateEvent> {
        self.receipts
            .lock()
            .iter()
            .find(|(id, _)| id == request_id)
            .map(|(_, event)| event.clone())
    }

    fn record_receipt(&self, request_id: &str, event: &StateEvent) {
        let mut receipts = self.receipts.lock();
        receipts.push_back((request_id.to_owned(), event.clone()));
        while receipts.len() > MAX_RECEIPTS {
            receipts.pop_front();
        }
    }
}

/// Thaw a unit's `browser/` sub-cgroup before stopping it, so a stop never
/// waits on frozen processes.
async fn thaw_browser_cgroup(unit_name: &str) {
    if let Ok(status) = unit::show(unit_name).await
        && let Some(cgroup) = status.cgroup_dir()
    {
        let _ = unit::set_frozen(&cgroup.join("browser"), false);
    }
}

struct Failure {
    reason: Reason,
    detail: String,
}

impl Failure {
    fn internal(error: anyhow::Error) -> Self {
        Self {
            reason: Reason::Failed,
            detail: format!("{error:#}"),
        }
    }

    fn launcher(error: anyhow::Error) -> Self {
        match error.downcast_ref::<LauncherRefusal>() {
            Some(refusal) => Self {
                reason: match refusal.kind {
                    Some(FailureKind::Sandbox) => Reason::SandboxUnavailable,
                    Some(FailureKind::Host) => Reason::HostRequirements,
                    Some(FailureKind::Internal) | None => Reason::Failed,
                },
                detail: refusal.message.clone(),
            },
            None => Self::internal(error),
        }
    }
}

trait StateEventExt {
    fn with_reason(self, reason: Reason) -> Self;
    fn with_request(self, request_id: &str) -> Self;
}

impl StateEventExt for StateEvent {
    fn with_reason(mut self, reason: Reason) -> Self {
        self.reason = Some(reason);
        self
    }

    fn with_request(mut self, request_id: &str) -> Self {
        self.request_id = Some(request_id.to_owned());
        self
    }
}

fn hibernated_event(
    request_id: &str,
    account_id: &ProviderAccountId,
    instance_id: &str,
    reason: Reason,
    profile_revision: Option<u64>,
) -> StateEvent {
    StateEvent {
        request_id: (!request_id.is_empty()).then(|| request_id.to_owned()),
        instance: InstanceRef {
            account_id: account_id.clone(),
            instance_id: instance_id.to_owned(),
            incarnation: 0,
        },
        state: State::Hibernated,
        reason: Some(reason),
        geometry: None,
        scale: None,
        deadline_ms: None,
        attachment_id: None,
        snapshot_generation: None,
        snapshot_at_ms: None,
        profile_revision,
        unsaved_state_lost: false,
        observed_at_ms: unix_ms(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VersionOrder {
    Unrecorded,
    Same,
    RuntimeNewer,
    RecordNewer,
}

/// The profile compatibility fence compares full browser versions, not
/// majors: an older build of the same major may not read a newer profile.
fn compare_versions(recorded: Option<&str>, runtime: &str) -> VersionOrder {
    let Some(recorded) = recorded else {
        return VersionOrder::Unrecorded;
    };
    let (Some(recorded), Some(runtime)) = (
        cowboy_plugin_sdk::parse_browser_version(recorded),
        cowboy_plugin_sdk::parse_browser_version(runtime),
    ) else {
        // Unparseable versions are compared textually and never silently
        // treated as compatible.
        return if recorded == runtime {
            VersionOrder::Same
        } else {
            VersionOrder::RecordNewer
        };
    };
    match recorded.cmp(&runtime) {
        std::cmp::Ordering::Less => VersionOrder::RuntimeNewer,
        std::cmp::Ordering::Equal => VersionOrder::Same,
        std::cmp::Ordering::Greater => VersionOrder::RecordNewer,
    }
}

/// Argument vector of `cowboy-browser-launcher run` for one start.
fn launch_arguments(
    plan: &MachineBrowserRuntimePlan,
    settings: &Settings,
    runtime_dir: &Path,
    staging_root: &Path,
    snapshot_dir: Option<&Path>,
    viewport: Viewport,
    state_dir: &Path,
) -> Result<Vec<OsString>> {
    let runtime = &plan.contract.runtime;
    let helper = |name: &str| -> Result<&str> {
        runtime
            .helpers
            .iter()
            .find(|helper| {
                Path::new(helper)
                    .file_name()
                    .is_some_and(|file| file == name)
            })
            .map(String::as_str)
            .with_context(|| format!("Machine browser runtime has no {name} helper"))
    };
    let mut arguments: Vec<OsString> = vec!["run".into()];
    let mut push = |flag: &str, value: &dyn AsRef<std::ffi::OsStr>| {
        arguments.push(flag.into());
        arguments.push(value.as_ref().to_owned());
    };
    push("--control-dir", &runtime_dir);
    push("--runtime-dir", &plan.runtime_root);
    if let Some(snapshot_dir) = snapshot_dir {
        push("--snapshot-dir", &snapshot_dir);
    }
    push("--staging-dir", &staging_root);
    push("--display", &DISPLAY);
    push(
        "--geometry",
        &format!("{}x{}", viewport.width, viewport.height),
    );
    push("--scale", &viewport.scale.to_string());
    push("--profile-bytes", &settings.profile_max_bytes.to_string());
    push("--instance-bytes", &settings.instance_max_bytes.to_string());
    push("--tmp-bytes", &TMP_BYTES.to_string());
    push("--policy-file", &runtime.policy.file);
    push("--policy-directory", &runtime.policy.directory);
    push("--browser", &runtime.browser);
    push("--display-server", &runtime.display_server);
    push("--window-manager", &helper("matchbox-window-manager")?);
    push("--xrandr", &helper("xrandr")?);
    push("--xkbcomp", &helper("xkbcomp")?);
    push("--xauth", &helper("xauth")?);
    push("--sealed-profile", &SEALED_PROFILE);
    push("--mask", &state_dir);
    Ok(arguments)
}

fn unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_versions_compare_fully_and_unparseable_ones_never_pass() {
        assert_eq!(
            compare_versions(None, "143.0.7499.40"),
            VersionOrder::Unrecorded
        );
        assert_eq!(
            compare_versions(Some("143.0.7499.40"), "143.0.7499.40"),
            VersionOrder::Same
        );
        assert_eq!(
            compare_versions(Some("143.0.7499.40"), "143.0.7499.41"),
            VersionOrder::RuntimeNewer
        );
        assert_eq!(
            compare_versions(Some("143.0.7499.41"), "143.0.7499.40"),
            VersionOrder::RecordNewer
        );
        assert_eq!(
            compare_versions(Some("144.0.1.1"), "143.0.7499.40"),
            VersionOrder::RecordNewer
        );
        assert_eq!(
            compare_versions(Some("weird"), "143.0.1.1"),
            VersionOrder::RecordNewer
        );
    }

    #[test]
    fn launch_arguments_follow_the_contract_and_name_every_helper() {
        let contract: cowboy_plugin_sdk::MachineBrowserContract = serde_json::from_str(
            &std::fs::read_to_string(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../plugins/machine-browser/contract.json"),
            )
            .unwrap(),
        )
        .unwrap();
        let plan = MachineBrowserRuntimePlan {
            plugin_id: "machine-browser".into(),
            plugin_version: contract.version.clone(),
            generation_digest: "sha256:aa".into(),
            command: "cowboy-browser-launcher".into(),
            runtime_root: PathBuf::from("/state/plugins/machine-browser/x/content"),
            launcher: PathBuf::from(
                "/state/plugins/machine-browser/x/content/usr/libexec/cowboy-browser-launcher",
            ),
            contract,
        };
        let arguments = launch_arguments(
            &plan,
            &Settings::from_config(None),
            Path::new("/run/user/1000/cowboy-browser/inst-1"),
            Path::new("/state/provider-auth/accounts/a/browser/staging"),
            Some(Path::new(
                "/state/provider-auth/accounts/a/browser/snapshots/gen-3",
            )),
            Viewport {
                width: 1280,
                height: 800,
                scale: 2,
            },
            Path::new("/state"),
        )
        .unwrap();
        let text: Vec<String> = arguments
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        assert_eq!(text[0], "run");
        for flag in [
            "--control-dir",
            "--runtime-dir",
            "--snapshot-dir",
            "--staging-dir",
            "--geometry",
            "--scale",
            "--profile-bytes",
            "--policy-file",
            "--policy-directory",
            "--browser",
            "--display-server",
            "--window-manager",
            "--xrandr",
            "--xkbcomp",
            "--xauth",
            "--sealed-profile",
            "--mask",
        ] {
            let position = text
                .iter()
                .position(|argument| argument == flag)
                .unwrap_or_else(|| panic!("{flag}"));
            assert!(!text[position + 1].starts_with("--"), "{flag} has a value");
        }
        let geometry = text
            .iter()
            .position(|argument| argument == "--geometry")
            .unwrap();
        assert_eq!(text[geometry + 1], "1280x800");
        let sealed = text
            .iter()
            .position(|argument| argument == "--sealed-profile")
            .unwrap();
        assert_eq!(text[sealed + 1], SEALED_PROFILE);
    }
}
