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
        let (account_dir, record) = match self.prepare_record(&account_id, &plan) {
            Ok(prepared) => prepared,
            Err(reason) => return hibernated(reason),
        };
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
        snapshot::reconcile(&account_dir, &record, &BTreeSet::new()).map_err(|error| {
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
                &instance.record(status.invocation_id.clone(), &self.boot_id),
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

    /// Stop with the account lifecycle lock taken here.
    pub(super) async fn stop_instance(
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
    pub(super) async fn stop_locked(
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
    /// copy is flushed) or was killed (crash-consistent).
    pub(super) async fn stop_with_snapshot(
        &self,
        instance: &Arc<Instance>,
    ) -> Result<Option<(u64, bool)>> {
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
    pub(super) fn complete_fenced(
        &self,
        account_dir: &Path,
        record: &mut ControlRecord,
    ) -> Result<()> {
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
