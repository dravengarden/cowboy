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
use crate::login_registry::{LoginSession, LoginSessions};
use crate::machine_plugins::{MachineBrowserRuntimePlan, MachinePluginStore};
use crate::machine_protocol::MachineEvent;
use crate::machine_protocol::machine_browser::{
    Capability, Command, Event, Geometry, InstanceRef, LEASE_EXPIRY_MS, Purpose, Reason, State,
    StateEvent, Viewport,
};
use crate::provider_account::ProviderAccountId;

mod attachments;
mod capacity;
mod drain;
mod launch;
mod lifecycle;
mod login_binding;
mod reconcile;
mod snapshots;
mod supervision;
mod termination;
#[cfg(test)]
mod tests;
mod units;

use capacity::{CapacityGuard, CapacityLedger};
use drain::StartGate;
use launch::{VersionOrder, compare_versions, launch_arguments};
use lifecycle::complete_fenced;
use snapshots::CopyJob;
use termination::{Leftover, Retained, StopOutcome, Unconfirmed};
use units::{Systemd, UnitControl};

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
/// How often the advertised capability is recomputed while connected.
const CAPABILITY_WATCH_INTERVAL: Duration = Duration::from_secs(30);
const MAX_RECEIPTS: usize = 256;

/// The Controller connection events travel on, with the endpoint streams
/// are opened against and the native logins it dispatched.
struct Uplink {
    id: u64,
    events: tokio::sync::mpsc::UnboundedSender<MachineEvent>,
    endpoint: reqwest::Url,
    logins: Weak<parking_lot::Mutex<HashMap<String, LoginSession>>>,
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
    /// Completion of the account's most recently dispatched ordered command;
    /// the next one starts only after it.
    order: parking_lot::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
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
}

/// What an instance runs for and until when. A `Tab` instance reused for a
/// sign-in is rebound to the login and adopts its earlier deadline.
#[derive(Debug, Clone)]
struct Binding {
    purpose: Purpose,
    deadline: Instant,
    deadline_ms: i64,
}

impl Binding {
    fn new(purpose: Purpose, deadline_ms: i64) -> Self {
        Self {
            purpose,
            deadline: Instant::now()
                + Duration::from_millis(
                    u64::try_from(deadline_ms.saturating_sub(unix_ms())).unwrap_or(0),
                ),
            deadline_ms,
        }
    }
}

/// Everything an [`Instance`] is built from, at start or adoption.
struct InstanceParts {
    account_id: ProviderAccountId,
    instance_id: String,
    incarnation: u64,
    purpose: Purpose,
    deadline_ms: i64,
    unit: String,
    runtime_dir: PathBuf,
    cgroup_dir: PathBuf,
    invocation_id: String,
    scale: u8,
    profile_revision: u64,
    plan: Arc<MachineBrowserRuntimePlan>,
    settings: Settings,
    report: ReadyReport,
    control: Control,
    phase: Phase,
    geometry: Geometry,
    last_snapshot: Option<(u64, i64)>,
    capacity: Option<CapacityGuard>,
}

/// One running (or starting) browser instance.
pub struct Instance {
    account_id: ProviderAccountId,
    instance_id: String,
    incarnation: u64,
    binding: parking_lot::Mutex<Binding>,
    unit: String,
    runtime_dir: PathBuf,
    cgroup_dir: PathBuf,
    invocation_id: String,
    scale: u8,
    profile_revision: u64,
    plan: Arc<MachineBrowserRuntimePlan>,
    settings: Settings,
    report: ReadyReport,
    control: tokio::sync::Mutex<Control>,
    live: parking_lot::Mutex<Live>,
    /// The running snapshot copy, if any.
    copy: parking_lot::Mutex<Option<CopyJob>>,
    /// Navigations refusing new copies; changed under the `copy` lock.
    copy_holds: std::sync::atomic::AtomicUsize,
    /// A staging generation a copier of this unit may still be writing; it is
    /// removed and its budget released only once the unit's exit is
    /// confirmed.
    abandoned: parking_lot::Mutex<Option<(u64, StagingReservation)>>,
    /// The Machine-wide instance slot, released on confirmed exit.
    capacity: parking_lot::Mutex<Option<CapacityGuard>>,
}

impl Instance {
    fn new(parts: InstanceParts) -> Self {
        Self {
            account_id: parts.account_id,
            instance_id: parts.instance_id,
            incarnation: parts.incarnation,
            binding: parking_lot::Mutex::new(Binding::new(parts.purpose, parts.deadline_ms)),
            unit: parts.unit,
            runtime_dir: parts.runtime_dir,
            cgroup_dir: parts.cgroup_dir,
            invocation_id: parts.invocation_id,
            scale: parts.scale,
            profile_revision: parts.profile_revision,
            plan: parts.plan,
            settings: parts.settings,
            report: parts.report,
            control: tokio::sync::Mutex::new(parts.control),
            live: parking_lot::Mutex::new(Live {
                phase: parts.phase,
                geometry: parts.geometry,
                verified: Verified::unpinned(),
                attachment: None,
                detached_since: Some(Instant::now()),
                last_snapshot: parts.last_snapshot,
                last_periodic: Instant::now(),
                last_scan: Instant::now(),
                pending_navigation: None,
            }),
            copy: parking_lot::Mutex::new(None),
            copy_holds: std::sync::atomic::AtomicUsize::new(0),
            abandoned: parking_lot::Mutex::new(None),
            capacity: parking_lot::Mutex::new(parts.capacity),
        }
    }

    fn reference(&self) -> InstanceRef {
        InstanceRef {
            account_id: self.account_id.clone(),
            instance_id: self.instance_id.clone(),
            incarnation: self.incarnation,
        }
    }

    fn purpose(&self) -> Purpose {
        self.binding.lock().purpose.clone()
    }

    fn deadline(&self) -> Instant {
        self.binding.lock().deadline
    }

    fn deadline_ms(&self) -> i64 {
        self.binding.lock().deadline_ms
    }

    /// The durable record written once an instance is verified (and again
    /// when it is rebound to a login).
    fn record(&self, boot_id: &str) -> InstanceRecord {
        let binding = self.binding.lock().clone();
        InstanceRecord {
            schema_version: 1,
            instance_id: self.instance_id.clone(),
            account_id: self.account_id.as_str().to_owned(),
            purpose: binding.purpose,
            plugin_id: self.plan.plugin_id.clone(),
            plugin_version: self.plan.plugin_version.clone(),
            generation_digest: self.plan.generation_digest.clone(),
            unit: self.unit.clone(),
            invocation_id: self.invocation_id.clone(),
            boot_id: boot_id.to_owned(),
            incarnation: self.incarnation,
            deadline_ms: binding.deadline_ms,
            profile_revision: self.profile_revision,
            started_at_ms: unix_ms(),
            runtime_dir: self.runtime_dir.clone(),
            cgroup_dir: self.cgroup_dir.clone(),
            geometry: self.live.lock().geometry,
            scale: self.scale,
            ready: self.report.clone(),
        }
    }

    fn login_request(&self) -> Option<String> {
        match &self.binding.lock().purpose {
            Purpose::SignIn { login_request_id } => Some(login_request_id.clone()),
            Purpose::Tab => None,
        }
    }

    fn state(&self, request_id: Option<String>) -> StateEvent {
        let deadline_ms = self.deadline_ms();
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
            deadline_ms: Some(deadline_ms),
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
    /// Abandoned staging generations whose scoped removal failed; their
    /// budget stays reserved until a retry removes them.
    leftover_staging: parking_lot::Mutex<Vec<Leftover>>,
    /// Machine-wide instance slots, reserved before any asynchronous start.
    capacity: Arc<CapacityLedger>,
    /// Unit invocations whose exit could not be confirmed, by retention
    /// ticket (one unit name may stand for more than one invocation). Their
    /// accounts' reset and revocation fences stay raised until a retry
    /// confirms exit.
    unconfirmed: parking_lot::Mutex<HashMap<u64, Unconfirmed>>,
    next_retention: std::sync::atomic::AtomicU64,
    /// Raised while the restart inventory has not accounted for every live
    /// unit invocation: any unit may still run for any account, so every
    /// account's fences and opens are held.
    inventory_pending: std::sync::atomic::AtomicBool,
    /// systemd, or a synthetic unit inventory in tests.
    units: Arc<dyn UnitControl>,
    /// Unit starts against Plugin-uninstall drains: opens are refused while
    /// a drain holds it, and a drain waits for admitted starts to settle.
    starts: Arc<StartGate>,
    /// Host paths every instance must hide behind a read-only mask.
    masks: Vec<PathBuf>,
    receipts: parking_lot::Mutex<VecDeque<(String, StateEvent)>>,
    headroom: fn() -> Option<u64>,
    this: Weak<BrowserManager>,
}

impl BrowserManager {
    /// `headroom` reports the Device's current memory headroom for launch
    /// admission (the Machine passes its host-resource observer).
    ///
    /// # Errors
    /// Returns when the kernel boot identity is unreadable.
    pub fn new(
        state_dir: &Path,
        providers: Arc<MachinePluginStore>,
        device_config: Option<crate::config::Handle>,
        headroom: fn() -> Option<u64>,
    ) -> Result<Arc<Self>> {
        Self::build(
            state_dir,
            providers,
            device_config,
            headroom,
            Arc::new(Systemd),
        )
    }

    fn build(
        state_dir: &Path,
        providers: Arc<MachinePluginStore>,
        device_config: Option<crate::config::Handle>,
        headroom: fn() -> Option<u64>,
        units: Arc<dyn UnitControl>,
    ) -> Result<Arc<Self>> {
        let boot_id = crate::boot_identity::current()?
            .context("kernel boot identity unavailable")?
            .to_string();
        let uid = rustix::process::getuid().as_raw();
        let manager = Arc::new_cyclic(|this| Self {
            state_dir: state_dir.to_path_buf(),
            providers,
            device_config,
            uid,
            gid: rustix::process::getgid().as_raw(),
            boot_id,
            uplink: parking_lot::Mutex::new(None),
            next_uplink: std::sync::atomic::AtomicU64::new(1),
            accounts: parking_lot::Mutex::new(HashMap::new()),
            staging: Arc::new(StagingLedger::default()),
            leftover_staging: parking_lot::Mutex::new(Vec::new()),
            capacity: Arc::new(CapacityLedger::default()),
            unconfirmed: parking_lot::Mutex::new(HashMap::new()),
            next_retention: std::sync::atomic::AtomicU64::new(1),
            inventory_pending: std::sync::atomic::AtomicBool::new(false),
            units,
            starts: Arc::new(StartGate::default()),
            masks: sensitive_paths(state_dir, uid),
            receipts: parking_lot::Mutex::new(VecDeque::new()),
            headroom,
            this: this.clone(),
        });
        manager.register_drain();
        Ok(manager)
    }

    /// Verification expectations for `instance` against `main_pid`.
    fn expectations<'a>(
        &'a self,
        instance: &'a Instance,
        main_pid: u32,
        bootstrap: &'a str,
        sealed: &'a str,
    ) -> Expectations<'a> {
        Expectations {
            uid: self.uid,
            gid: self.gid,
            runtime_root: &instance.plan.runtime_root,
            runtime_dir: &instance.runtime_dir,
            cgroup_dir: &instance.cgroup_dir,
            main_pid,
            profile_bytes: instance.settings.profile_max_bytes,
            instance_bytes: instance.settings.instance_max_bytes,
            policy_directory: &instance.plan.contract.runtime.policy.directory,
            policy_file: &instance.plan.contract.runtime.policy.file,
            masks: &self.masks,
            bootstrap_label: Some(bootstrap),
            sealed_label: Some(sealed),
        }
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
        self.watch_capability(id, events);
        UplinkGuard {
            manager: Arc::downgrade(self),
            id,
        }
    }

    /// Re-announce the browser capability whenever it changes while this
    /// uplink is current: a Plugin installed, upgraded or removed, or the host
    /// policy appearing or disappearing, must not wait for a reconnect.
    fn watch_capability(
        self: &Arc<Self>,
        uplink: u64,
        events: tokio::sync::mpsc::UnboundedSender<MachineEvent>,
    ) {
        let manager = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut announced: Option<Option<Capability>> = None;
            let mut tick = tokio::time::interval(CAPABILITY_WATCH_INTERVAL);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                let Some(manager) = manager.upgrade() else {
                    return;
                };
                if !manager
                    .uplink
                    .lock()
                    .as_ref()
                    .is_some_and(|current| current.id == uplink)
                {
                    return;
                }
                let current = manager.capability();
                // The hello already carried the first observation.
                let changed = announced
                    .as_ref()
                    .is_some_and(|previous| *previous != current);
                if announced.is_none() {
                    announced = Some(current);
                    continue;
                }
                if changed {
                    let event = match &current {
                        Some(capability) => Event::Capability(capability.clone()),
                        None => Event::CapabilityLost,
                    };
                    if events
                        .send(MachineEvent::Browser { browser: event })
                        .is_err()
                    {
                        return;
                    }
                    announced = Some(current);
                }
            }
        });
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

    /// Run one Controller command in the background. Lifecycle commands
    /// (open, hibernate, reset, revoke, query) of one account run strictly
    /// in arrival order, so an open and a revocation the Controller sent in
    /// that order are applied in that order; leases, attachments and other
    /// per-instance commands never wait behind a slow lifecycle step.
    pub fn dispatch(self: &Arc<Self>, request_id: String, command: Command) {
        let ordered = match &command {
            Command::Open { account_id, .. }
            | Command::Reset { account_id, .. }
            | Command::Revoke { account_id, .. }
            | Command::Query { account_id } => Some(account_id.clone()),
            Command::Hibernate { instance } => Some(instance.account_id.clone()),
            _ => None,
        };
        let turn = ordered.map(|account| {
            let (done, next) = tokio::sync::oneshot::channel::<()>();
            let previous = self.slot(&account).order.lock().replace(next);
            (previous, done)
        });
        let manager = Arc::clone(self);
        tokio::spawn(async move {
            let _done = match turn {
                Some((previous, done)) => {
                    if let Some(previous) = previous {
                        // Resolves when the previous command's sender drops.
                        let _ = previous.await;
                    }
                    Some(done)
                }
                None => None,
            };
            manager.handle(request_id, command).await;
        });
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
                if let Err(reason) = manager.navigate_checked(&instance, &url).await {
                    let event = manager.stop_instance(&instance, reason, false, None).await;
                    manager.send_state(event);
                }
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
                        order: parking_lot::Mutex::new(None),
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
    /// superseded observation to answer with. An incarnation retained until
    /// its exit is confirmed answers as `stopping`, never as absent.
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
            None => Err(Box::new(
                self.unresolved_state(&reference.account_id)
                    .filter(|unresolved| unresolved.instance == *reference)
                    .unwrap_or_else(|| {
                        hibernated_event(
                            "",
                            &reference.account_id,
                            &reference.instance_id,
                            Reason::Superseded,
                            None,
                        )
                    }),
            )),
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

/// The host paths an instance must not see: `/home`, the Machine user's
/// runtime directory and the Machine state root as the kernel resolves it
/// (the launcher's mask mount lands on the resolved path; a state root below
/// `/home` is covered by that mask).
fn sensitive_paths(state_dir: &Path, uid: u32) -> Vec<PathBuf> {
    vec![
        PathBuf::from("/home"),
        PathBuf::from(format!("/run/user/{uid}")),
        std::fs::canonicalize(state_dir).unwrap_or_else(|_| state_dir.to_path_buf()),
    ]
}

fn unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}
