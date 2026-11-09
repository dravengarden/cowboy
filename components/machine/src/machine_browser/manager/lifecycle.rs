//! Instance lifecycle: admission and start, graceful and forced stops, reset and
//! revocation fences, and the hibernated/running state read.

use super::*;

impl BrowserManager {
    pub(super) async fn open(
        self: &Arc<Self>,
        request_id: &str,
        account_id: ProviderAccountId,
        instance_id: String,
        purpose: Purpose,
        viewport: Viewport,
    ) -> StateEvent {
        let hibernated =
            |reason: Reason| hibernated_event(request_id, &account_id, &instance_id, reason, None);
        // Browser instances are plain per-account tabs, independent of any
        // native login. The retired sign-in purpose stays decodable on the
        // wire for protocol 45 peers but is refused.
        if purpose != Purpose::Tab {
            tracing::warn!(account = %account_id.as_str(), "browser open with a retired sign-in purpose refused");
            return hibernated(Reason::NotAvailable);
        }
        let slot = self.slot(&account_id);
        let existing = slot.instance.lock().clone();
        if let Some(existing) = existing {
            if existing.instance_id != instance_id {
                return hibernated(Reason::Busy);
            }
            return existing.state(Some(request_id.to_owned()));
        }
        let _lifecycle = slot.lifecycle.lock().await;
        if slot.instance.lock().is_some() {
            return hibernated(Reason::Busy);
        }
        // An earlier unit of this account (or of an unknown one, or with the
        // same unit name) whose exit is unconfirmed may still run: a new
        // start could be killed by its termination retry or run beside it.
        if self.fence_blocked(&account_id) || self.unit_retained(&unit::unit_name(&instance_id)) {
            return hibernated(Reason::Busy);
        }
        // Held until this open returns, so an uninstall drain waits for the
        // start to settle (and see the instance) instead of missing it.
        let Some(_start) = self.starts.enter() else {
            return hibernated(Reason::NotAvailable);
        };
        let plan = match self.providers.machine_browser_runtime() {
            Ok(Some(plan)) => Arc::new(plan),
            Ok(None) => return hibernated(Reason::NotAvailable),
            Err(error) => {
                tracing::warn!(%error, "Machine browser runtime is not usable");
                return hibernated(Reason::NotAvailable);
            }
        };
        // Held until this open returns: the generation stays pinned until
        // the started instance (or its retained unit) pins it itself.
        let Some(_generation) = self.pins.pin(&plan) else {
            tracing::warn!(generation = %plan.generation_digest, "browser runtime generation retired before its start");
            return hibernated(Reason::NotAvailable);
        };
        if let Err(unmet) = host::check(&plan.contract.host_requirements) {
            tracing::warn!(detail = %unmet.0, "browser open refused: host requirements");
            return hibernated(Reason::HostRequirements);
        }
        let settings = self.settings();
        if !settings.geometry_allowed(viewport.width, viewport.height) {
            self.send_result(
                request_id.to_owned(),
                false,
                Some("browser viewport exceeds this Device's bound".into()),
            );
            return hibernated(Reason::Failed);
        }
        // Reserved before anything asynchronous, so concurrent opens for
        // other accounts cannot pass the limit together.
        let Some(capacity) = self.capacity.try_reserve(settings.max_instances) else {
            return hibernated(Reason::Capacity);
        };
        let (account_dir, record) = match self.prepare_record(&account_id, &plan) {
            Ok(prepared) => prepared,
            Err(reason) => return hibernated(reason),
        };
        let incarnation = u64::try_from(unix_ms()).unwrap_or(1).max(1);
        let deadline_ms = unix_ms()
            .saturating_add(i64::try_from(settings.max_runtime.as_millis()).unwrap_or(i64::MAX));
        let Some(budget) = self.admit_memory(&account_id, &settings).await else {
            return hibernated(Reason::Memory);
        };
        let launch_began = Instant::now();
        let started = self
            .start_instance(
                &account_dir,
                &record,
                Arc::clone(&plan),
                settings,
                &account_id,
                &instance_id,
                incarnation,
                viewport,
                deadline_ms,
                capacity.with_memory(budget),
            )
            .await;
        let instance = match started {
            Ok(instance) => instance,
            Err(failure) => {
                tracing::warn!(event_name = "browser.open.failed",
                    browser_account = %account_id.as_str(), detail = %failure.detail,
                    reason = ?failure.reason, "browser open failed");
                return hibernated(failure.reason);
            }
        };
        *slot.instance.lock() = Some(Arc::clone(&instance));
        tracing::info!(event_name = "browser.instance.ready",
            browser_instance = %instance.instance_id,
            browser_account = %account_id.as_str(),
            browser_request = %request_id,
            duration_ms = u64::try_from(launch_began.elapsed().as_millis()).unwrap_or(u64::MAX),
            "browser instance ready");
        self.spawn_supervisor(&instance);
        instance.state(Some(request_id.to_owned()))
    }

    /// The admitted budget: at most `memory_max`, at least
    /// `memory_reserve`, never more than the headroom left beside every
    /// outstanding reservation and the floor. It becomes the unit's
    /// MemoryMax and is held with the capacity slot until the unit's exit
    /// is confirmed, so the browser can never take shared memory beyond
    /// what was admitted. `None` when no headroom appeared in time.
    async fn admit_memory(
        &self,
        account_id: &ProviderAccountId,
        settings: &Settings,
    ) -> Option<crate::launch_ledger::Reservation> {
        let budget = match self
            .ledger
            .reserve_within(
                settings.memory_reserve,
                settings.memory_max,
                self.headroom,
                || self.starts.draining(),
                MEMORY_WAIT,
                MEMORY_POLL,
            )
            .await
        {
            Admission::Reserved { reservation, .. } => reservation,
            Admission::Exhausted => {
                tracing::info!(
                    event_name = "browser.open.refused",
                    browser_account = %account_id.as_str(),
                    reason = "Memory",
                    headroom_bytes = (self.headroom)(),
                    reserved_bytes = self.ledger.reserved_bytes(),
                    requested_min_bytes = settings.memory_reserve,
                    requested_max_bytes = settings.memory_max,
                    floor_bytes = FLOOR_BYTES,
                    waited_secs = MEMORY_WAIT.as_secs(),
                    "browser open refused: no memory headroom"
                );
                return None;
            }
            Admission::Cancelled => return None,
        };
        tracing::info!(
            event_name = "browser.open.admitted",
            browser_account = %account_id.as_str(),
            budget_bytes = budget.bytes(),
            headroom_bytes = (self.headroom)(),
            "browser open admitted"
        );
        Some(budget)
    }

    /// Resolve the account directory and its control record for a start:
    /// a pending reset or revocation refuses, and the browser-version fence
    /// either admits, advances the record or refuses a newer profile.
    fn prepare_record(
        &self,
        account_id: &ProviderAccountId,
        plan: &MachineBrowserRuntimePlan,
    ) -> Result<(PathBuf, ControlRecord), Reason> {
        let account_dir = self.account_dir(account_id).map_err(|error| {
            tracing::warn!(%error, "browser account directory unavailable");
            Reason::Failed
        })?;
        let mut record = state_file::read_control(&account_dir).map_err(|error| {
            tracing::warn!(%error, "browser control record unreadable");
            Reason::Failed
        })?;
        if let Some(pending) = &record.pending {
            return Err(match pending.kind {
                OperationKind::Reset => Reason::Busy,
                OperationKind::Revoke => Reason::Revoked,
            });
        }
        match compare_versions(
            record.browser_version.as_deref(),
            &plan.contract.runtime.browser_version,
        ) {
            VersionOrder::RecordNewer => return Err(Reason::ProfileNewerThanRuntime),
            VersionOrder::Same => {}
            VersionOrder::RuntimeNewer | VersionOrder::Unrecorded => {
                record.browser_version = Some(plan.contract.runtime.browser_version.clone());
                record.generation_digest = Some(plan.generation_digest.clone());
                state_file::write_control(&account_dir, &record).map_err(|error| {
                    tracing::warn!(%error, "browser control record not writable");
                    Reason::Failed
                })?;
            }
        }
        // Staging still reserved (a copier that may write, or a failed
        // removal awaiting retry), or that an unresolved unit may write, is
        // never removed here.
        self.reconcile_storage(account_id.as_str(), &account_dir, &record)
            .map_err(|error| {
                tracing::warn!(%error, "browser snapshot reconciliation failed");
                Reason::Failed
            })?;
        Ok((account_dir, record))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn start_instance(
        &self,
        account_dir: &Path,
        record: &ControlRecord,
        plan: Arc<MachineBrowserRuntimePlan>,
        settings: Settings,
        account_id: &ProviderAccountId,
        instance_id: &str,
        incarnation: u64,
        viewport: Viewport,
        deadline_ms: i64,
        // The instance slot, holding the admitted memory budget.
        capacity: CapacityGuard,
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
            limits: admitted_limits(
                &settings,
                capacity.memory_bytes().unwrap_or(settings.memory_max),
            ),
            launcher: &plan.launcher,
            arguments,
        };
        let mut cgroup_dir = None;
        let mut invocation_id = None;
        let outcome = async {
            unit::start(&launch).await.map_err(Failure::internal)?;
            let status = unit::wait_running(&unit_name, UNIT_START_TIMEOUT)
                .await
                .map_err(Failure::internal)?;
            cgroup_dir = status.cgroup_dir();
            invocation_id = Some(status.invocation_id.clone()).filter(|id| !id.is_empty());
            let readiness = Duration::from_millis(plan.contract.launch.readiness_timeout_ms);
            let mut control = Control::connect(&runtime_dir, readiness)
                .await
                .map_err(Failure::internal)?;
            let report = control
                .wait_ready(readiness)
                .await
                .map_err(Failure::launcher)?;
            let instance = Arc::new(Instance::new(InstanceParts {
                account_id: account_id.clone(),
                instance_id: instance_id.to_owned(),
                incarnation,
                deadline_ms,
                unit: unit_name.clone(),
                runtime_dir: runtime_dir.clone(),
                cgroup_dir: cgroup_dir
                    .clone()
                    .ok_or_else(|| Failure::internal(anyhow!("unit reports no cgroup")))?,
                invocation_id: status.invocation_id.clone(),
                scale: viewport.scale,
                profile_revision: record.profile_revision,
                plan: Arc::clone(&plan),
                settings,
                report,
                control,
                phase: Phase::Starting,
                geometry: Geometry {
                    width: viewport.width,
                    height: viewport.height,
                },
                last_snapshot: record
                    .current_generation
                    .zip(record.current_generation_at_ms),
                capacity: None,
            }));
            self.verify_started(&instance, status.main_pid)?;
            state_file::write_instance(&self.state_dir, &instance.record(&self.boot_id))
                .map_err(Failure::internal)?;
            Ok::<_, Failure>(instance)
        }
        .await;
        // From here the unit's own usage counts against every headroom
        // sample; the budget holds only what it may still grow into.
        if let Some(cgroup) = &cgroup_dir {
            capacity.charge_memory_to(cgroup);
        }
        match outcome {
            Ok(instance) => {
                // The instance holds the slot (and its memory budget)
                // reserved by the open.
                *instance.capacity.lock() = Some(capacity);
                Ok(instance)
            }
            Err(failure) => {
                // Never retry with weaker isolation: the unit ends (a
                // verification failure included), and its slot and memory
                // budget are released only once that is confirmed.
                self.terminate_or_retain(Retained {
                    unit: unit_name,
                    cgroup_dir,
                    invocation_id,
                    account: Some(account_id.clone()),
                    instance: None,
                    record: None,
                    capacity: Some(capacity),
                    generation: Some(plan.generation_digest.clone()),
                    // The open answered the failure; the confirmed exit
                    // reports the incarnation's terminal state, which
                    // `Query` reports as stopping until then.
                    outcome: StopOutcome {
                        reason: failure.reason,
                        unsaved: true,
                        reference: Some(InstanceRef {
                            account_id: account_id.clone(),
                            instance_id: instance_id.to_owned(),
                            incarnation,
                        }),
                    },
                })
                .await;
                Err(failure)
            }
        }
    }

    /// Full confinement verification of a freshly started instance.
    fn verify_started(&self, instance: &Instance, main_pid: u32) -> Result<(), Failure> {
        let bootstrap = host::enforce_label(BOOTSTRAP_PROFILE);
        let sealed = host::enforce_label(SEALED_PROFILE);
        let verified = verify::verify(
            &self.expectations(instance, main_pid, &bootstrap, &sealed),
            &instance.report,
        )
        .map_err(|violation| Failure {
            reason: Reason::ConfinementUnverified,
            detail: violation.0,
        })?;
        let mut live = instance.live.lock();
        live.verified = verified;
        live.phase = Phase::Detached;
        Ok(())
    }

    pub(super) async fn hibernate(
        self: &Arc<Self>,
        request_id: &str,
        reference: &InstanceRef,
    ) -> StateEvent {
        match self.current(reference) {
            Ok(instance) => {
                self.stop_instance(&instance, Reason::OwnerRequest, true, Some(request_id))
                    .await
            }
            Err(event) => (*event).with_request(request_id),
        }
    }

    /// Stop with the account lifecycle lock taken here. A graceful stop first
    /// lets a running snapshot copy finish (outside the lock, with the
    /// deadline and authority still enforced); a stop that must be forced
    /// meanwhile cancels it instead.
    pub(super) async fn stop_instance(
        &self,
        instance: &Arc<Instance>,
        reason: Reason,
        graceful: bool,
        request_id: Option<&str>,
    ) -> StateEvent {
        let Ok(event) = self
            .stop_instance_if(instance, reason, graceful, request_id, || {
                Ok::<(), std::convert::Infallible>(())
            })
            .await;
        event
    }

    /// [`Self::stop_instance`] that evaluates `authorized` after every wait
    /// (copy, lifecycle lock), immediately before the stop changes anything;
    /// a refusal stops nothing and is returned.
    pub(super) async fn stop_instance_if<E>(
        &self,
        instance: &Arc<Instance>,
        reason: Reason,
        graceful: bool,
        request_id: Option<&str>,
        authorized: impl FnOnce() -> Result<(), E>,
    ) -> Result<StateEvent, E> {
        self.stop_instance_when(instance, reason, graceful, request_id, authorized, |_| {
            Ok(())
        })
        .await
    }

    /// [`Self::stop_instance_if`] that additionally evaluates `still` on the
    /// instance's live state under the same live lock that changes its phase
    /// to `Stopping`, so nothing (an attach, a detach) can change it between
    /// the check and the transition; a refusal stops nothing and is returned.
    pub(super) async fn stop_instance_when<E>(
        &self,
        instance: &Arc<Instance>,
        reason: Reason,
        graceful: bool,
        request_id: Option<&str>,
        authorized: impl FnOnce() -> Result<(), E>,
        still: impl FnOnce(&Live) -> Result<(), E>,
    ) -> Result<StateEvent, E> {
        let (reason, graceful) = if graceful {
            match self.await_copy_idle(instance).await {
                Ok(()) => (reason, true),
                Err(forced) => (forced, false),
            }
        } else {
            (reason, false)
        };
        let slot = self.slot(&instance.account_id);
        let _lifecycle = slot.lifecycle.lock().await;
        // Nothing below awaits before the phase changes.
        authorized()?;
        self.stop_locked_when(instance, reason, graceful, request_id, still)
            .await
    }

    /// Stop while the caller holds the account lifecycle lock. Any running
    /// copy is cancelled. A graceful stop lets the browser flush and copies
    /// the profile at the barrier; a forced stop keeps the last published
    /// generation and reports that unsaved state was lost. The instance is
    /// forgotten only once its unit's exit is confirmed; otherwise it stays
    /// registered (stopping) while a retry escalates.
    pub(super) async fn stop_locked(
        &self,
        instance: &Arc<Instance>,
        reason: Reason,
        graceful: bool,
        request_id: Option<&str>,
    ) -> StateEvent {
        let Ok(event) = self
            .stop_locked_when(instance, reason, graceful, request_id, |_| {
                Ok::<(), std::convert::Infallible>(())
            })
            .await;
        event
    }

    /// [`Self::stop_locked`] that first evaluates `still` under the live
    /// lock it changes the phase under; a refusal changes nothing.
    pub(super) async fn stop_locked_when<E>(
        &self,
        instance: &Arc<Instance>,
        reason: Reason,
        graceful: bool,
        request_id: Option<&str>,
        still: impl FnOnce(&Live) -> Result<(), E>,
    ) -> Result<StateEvent, E> {
        let stop_began = Instant::now();
        let (already_stopping, before, idle_ms, forced) = {
            let mut live = instance.live.lock();
            // A latched forced stop is never abandoned for renewed use.
            let forced = *instance.latch.lock();
            if forced.is_none() {
                still(&live)?;
            }
            let before = live.phase;
            let was = before == Phase::Stopping;
            let idle_ms = live
                .detached_since
                .map(|since| u64::try_from(since.elapsed().as_millis()).unwrap_or(u64::MAX));
            live.phase = Phase::Stopping;
            if let Some(attachment) = live.attachment.take()
                && let Some(stream) = attachment.stream
            {
                stream.cancel();
            }
            (was, before, idle_ms, forced)
        };
        // A latched forced stop overrides a graceful one decided earlier.
        let (reason, graceful) = forced.map_or((reason, graceful), |forced| (forced, false));
        if already_stopping {
            let registered = self
                .slot(&instance.account_id)
                .instance
                .lock()
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, instance));
            if !registered {
                // Another path stopped it while this one waited for the lock:
                // its outcome (reason, unsaved state) is the instance's.
                let (reason, unsaved) = instance.outcome.lock().unwrap_or((reason, false));
                return Ok(self.hibernated_after(instance, reason, request_id, unsaved));
            }
            // Only an instance whose exit is unconfirmed stays registered
            // while stopping; its retry owns the stop.
            return Ok(instance
                .state(request_id.map(ToOwned::to_owned))
                .with_reason(reason));
        }
        tracing::info!(event_name = "browser.stop.begin",
            browser_instance = %instance.instance_id,
            browser_account = %instance.account_id.as_str(),
            browser_request = request_id.unwrap_or(""),
            reason = ?reason, graceful, browser_phase = ?before,
            age_ms = u64::try_from(instance.started.elapsed().as_millis()).unwrap_or(u64::MAX),
            idle_ms = idle_ms.unwrap_or(0),
            memory_bytes = unit::memory_current(&instance.cgroup_dir).unwrap_or(0),
            "browser stopping");
        instance.cancel_copy();
        if let Err(error) = unit::set_frozen(&instance.cgroup_dir.join("browser"), false) {
            tracing::warn!(%error, "browser cgroup not thawed before its stop");
        }
        self.send_state(instance.state(None).with_reason(reason));
        if !instance.copy_released().await {
            tracing::warn!(instance = %instance.instance_id, "cancelled browser snapshot did not let go in time");
        }
        // A running browser may have written since its last published
        // snapshot; only a flushed stop-time copy proves nothing was lost.
        let mut unsaved = true;
        let mut published = None;
        if graceful {
            match self.stop_with_snapshot(instance).await {
                Ok(Some((generation, flushed))) => {
                    published = Some(generation);
                    unsaved = !flushed;
                }
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(%error, "browser graceful stop did not publish a snapshot");
                }
            }
        }
        if let Some(generation) = published {
            instance.live.lock().last_snapshot = Some((generation, unix_ms()));
        }
        // A latch set during this stop refused its copy: report it as such.
        let reason = instance.finalize_outcome(reason, unsaved);
        if !self
            .units
            .terminate(
                &instance.unit,
                Some(&instance.cgroup_dir),
                Some(&instance.invocation_id),
            )
            .await
        {
            tracing::error!(event_name = "browser.stop.unconfirmed",
                browser_instance = %instance.instance_id, unit = %instance.unit,
                "browser unit exit unconfirmed; instance retained");
            self.retain_unconfirmed(Retained {
                unit: instance.unit.clone(),
                cgroup_dir: Some(instance.cgroup_dir.clone()),
                invocation_id: Some(instance.invocation_id.clone()),
                account: Some(instance.account_id.clone()),
                instance: Some(Arc::clone(instance)),
                record: None,
                capacity: None,
                generation: Some(instance.plan.generation_digest.clone()),
                // The confirmed exit reports the stop's own reason.
                outcome: StopOutcome {
                    reason,
                    unsaved,
                    reference: None,
                },
            });
            return Ok(instance
                .state(request_id.map(ToOwned::to_owned))
                .with_reason(Reason::Failed));
        }
        // Finalized again at the confirmed exit: the latch may have been
        // set while termination was pending.
        let reason = instance.finalize_outcome(reason, unsaved);
        self.finish_stopped(instance);
        tracing::info!(event_name = "browser.stop.done",
            browser_instance = %instance.instance_id,
            browser_account = %instance.account_id.as_str(),
            reason = ?reason, unsaved, generation = published.unwrap_or(0),
            duration_ms = u64::try_from(stop_began.elapsed().as_millis()).unwrap_or(u64::MAX),
            "browser hibernated");
        Ok(self.hibernated_after(instance, reason, request_id, unsaved))
    }

    pub(super) fn hibernated_after(
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
    /// copy is flushed) or was killed (crash-consistent). A staging
    /// generation the launcher may still write is left with the instance
    /// until its unit's exit is confirmed.
    pub(super) async fn stop_with_snapshot(
        &self,
        instance: &Arc<Instance>,
    ) -> Result<Option<(u64, bool)>> {
        let account_dir = self.account_dir(&instance.account_id)?;
        let mut record = state_file::read_control(&account_dir)?;
        if record.pending.is_some() || record.profile_revision != instance.profile_revision {
            plain_stop(instance).await;
            return Ok(None);
        }
        let generation = record.allocate_generation();
        state_file::write_control(&account_dir, &record)?;
        let reservation = match self.admit_staging(
            &instance.account_id,
            &account_dir,
            generation,
            &instance.settings,
        ) {
            Ok(reservation) => reservation,
            Err(error) => {
                tracing::warn!(%error, "stop-time snapshot refused");
                plain_stop(instance).await;
                return Ok(None);
            }
        };
        if let Err(error) = snapshot::create_staging(&account_dir, generation) {
            self.retire_staging(&account_dir, generation, reservation);
            plain_stop(instance).await;
            return Err(error);
        }
        let stopped = instance
            .control
            .lock()
            .await
            .stop(GRACEFUL_WAIT, Some(generation), unit::STOP_TIMEOUT)
            .await;
        let outcome = match stopped {
            Ok(outcome) => outcome,
            Err(error) => {
                *instance.abandoned.lock() = Some((generation, reservation));
                return Err(error);
            }
        };
        // The launcher replied: its inline copy has ended.
        if !outcome.copied {
            self.retire_staging(&account_dir, generation, reservation);
            bail!("launcher made no stop-time copy");
        }
        self.publish_snapshot(instance, &account_dir, generation, reservation)?;
        Ok(Some((generation, outcome.graceful)))
    }

    pub(super) async fn reset_or_revoke(
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
        if slot.instance.lock().is_some() || self.fence_blocked(&account_id) {
            // A browser of this account may still run: the profile is not
            // deleted and the fence stays raised until its exit is confirmed.
            tracing::error!(account = %account_id.as_str(), "browser exit unconfirmed; fence retained");
            return hibernated(Some(Reason::Failed), Some(record.profile_revision));
        }
        match complete_fenced(&account_dir, &mut record) {
            Ok(()) => hibernated(Some(reason), Some(record.profile_revision)),
            Err(error) => {
                tracing::warn!(%error, "browser reset or revocation incomplete; fence retained");
                hibernated(Some(Reason::Failed), Some(record.profile_revision))
            }
        }
    }

    pub(super) fn query(&self, request_id: &str, account_id: &ProviderAccountId) -> StateEvent {
        let slot = self.slot(account_id);
        let running = slot.instance.lock().clone();
        if let Some(instance) = running {
            return instance.state(Some(request_id.to_owned()));
        }
        let record = self
            .account_dir(account_id)
            .and_then(|dir| state_file::read_control(&dir))
            .unwrap_or_default();
        // A browser that may still run is never reported absent.
        if let Some(unresolved) = self.unresolved_state(account_id) {
            return StateEvent {
                request_id: Some(request_id.to_owned()),
                snapshot_generation: record.current_generation,
                snapshot_at_ms: record.current_generation_at_ms,
                profile_revision: Some(record.profile_revision),
                ..unresolved
            };
        }
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
}

/// Ask the launcher for a stop without a copy; the unit's termination follows.
async fn plain_stop(instance: &Instance) {
    let _ = instance
        .control
        .lock()
        .await
        .stop(GRACEFUL_WAIT, None, unit::STOP_TIMEOUT)
        .await;
}

/// Delete the profile tree and lower the fence; the control record survives
/// with the advanced revision. The writer-version fence goes with the data it
/// guarded, so a reset or revocation also lifts `profile_newer_than_runtime`.
pub(super) fn complete_fenced(account_dir: &Path, record: &mut ControlRecord) -> Result<()> {
    let pending = record
        .pending
        .clone()
        .context("no fenced operation to complete")?;
    snapshot::remove_browser_dir(account_dir)?;
    record.profile_revision = pending.next_profile_revision;
    record.current_generation = None;
    record.current_generation_at_ms = None;
    record.browser_version = None;
    record.generation_digest = None;
    record.pending = None;
    record.last_operation = Some(CompletedOperation {
        kind: pending.kind,
        operation_id: pending.operation_id,
        profile_revision: record.profile_revision,
        completed_at_ms: unix_ms(),
    });
    state_file::write_control(account_dir, record)
}
