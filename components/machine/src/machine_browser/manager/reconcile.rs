//! Restart reconciliation: instance records, surviving units, adoption of
//! `Tab` instances, fenced operations and snapshot storage. A unit is
//! forgotten only once its exit is confirmed; until then its record stays and
//! its account's fence stays raised.

use super::*;

/// The `unconfirmed` entry that stands for an incomplete unit inventory:
/// any unit may still run for any account until a listing succeeds.
pub(super) const INVENTORY_UNKNOWN: &str = "<browser unit inventory>";

impl BrowserManager {
    pub(super) async fn reconcile_units(self: &Arc<Self>) -> Result<()> {
        // Unreadable records leave every live unit unrecorded: it is stopped
        // rather than adopted, which is always safe.
        let records = state_file::read_instances(&self.state_dir).unwrap_or_else(|error| {
            tracing::warn!(%error, "browser instance records unreadable; stopping every unit");
            Vec::new()
        });
        let mut recorded = BTreeSet::new();
        for record in records {
            recorded.insert(record.unit.clone());
            if let Err(error) = self.reconcile_record(record).await {
                tracing::warn!(%error, "browser instance record reconciliation failed");
            }
        }
        let units = match unit::list_units().await {
            Ok(units) => units,
            Err(error) => {
                self.retry_inventory();
                return Err(error.context("browser unit inventory unavailable; every fence held"));
            }
        };
        self.stop_unrecorded(units, &recorded).await;
        Ok(())
    }

    /// Stop every listed unit not in `known`. An unrecorded unit may have
    /// run for any account, so an unconfirmed one holds every account's
    /// fence.
    async fn stop_unrecorded(&self, units: Vec<String>, known: &BTreeSet<String>) {
        for unit_name in units {
            if known.contains(&unit_name) {
                continue;
            }
            tracing::warn!(unit = %unit_name, "stopping unrecorded browser unit");
            let (cgroup, invocation) = thaw_browser_cgroup(&unit_name).await;
            if !unit::terminate(&unit_name, cgroup.as_deref(), invocation.as_deref()).await {
                self.retain_unconfirmed(Retained {
                    unit: unit_name,
                    cgroup_dir: cgroup,
                    invocation_id: invocation,
                    account: None,
                    instance: None,
                    record: None,
                    capacity: Some(self.capacity.force_reserve()),
                    outcome: StopOutcome::unreported(Reason::Failed),
                });
            }
        }
    }

    /// The unit listing failed: hold every account's fence and every open
    /// (as an unrecorded unit would) and retry the listing until it succeeds,
    /// then stop whatever no record, registered instance or retained unit
    /// accounts for.
    pub(super) fn retry_inventory(&self) {
        self.unconfirmed
            .lock()
            .insert(INVENTORY_UNKNOWN.to_owned(), Unconfirmed { account: None });
        let Some(manager) = self.this.upgrade() else {
            return;
        };
        tokio::spawn(async move {
            let units = loop {
                tokio::time::sleep(termination::RETRY_INTERVAL).await;
                match unit::list_units().await {
                    Ok(units) => break units,
                    Err(error) => {
                        tracing::error!(%error, "browser unit inventory still unavailable")
                    }
                }
            };
            let mut known: BTreeSet<String> = manager.unconfirmed.lock().keys().cloned().collect();
            known.extend(
                manager
                    .running()
                    .iter()
                    .map(|instance| instance.unit.clone()),
            );
            known.extend(
                state_file::read_instances(&manager.state_dir)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|record| record.unit),
            );
            manager.stop_unrecorded(units, &known).await;
            manager.unconfirmed.lock().remove(INVENTORY_UNKNOWN);
            tracing::info!("browser unit inventory completed after retry");
            if let Err(error) = manager.reconcile_accounts().await {
                tracing::warn!(%error, "browser fence resumption incomplete");
            }
        });
    }

    pub(super) async fn reconcile_record(self: &Arc<Self>, record: InstanceRecord) -> Result<()> {
        if record.boot_id != self.boot_id {
            return self.discard_record(&record, "other boot");
        }
        let status = match unit::show(&record.unit).await {
            Ok(status) => status,
            Err(error) => {
                // Unobservable is not gone: the unit keeps its record,
                // capacity and its account's fences and admission held
                // while a retry bound to its recorded invocation stops it.
                self.retain_recorded(&record, Some(record.cgroup_dir.clone()), Reason::Failed);
                return Err(error.context("surviving browser unit unobservable; retained"));
            }
        };
        if status.gone() {
            return self.discard_record(&record, "unit ended");
        }
        if !status.live() || status.invocation_id != record.invocation_id {
            return self
                .stop_recorded(&record, "unit ended or replaced", Reason::Failed)
                .await;
        }
        // A sign-in instance's login did not survive the restart.
        if record.purpose != Purpose::Tab {
            return self
                .stop_recorded(
                    &record,
                    "sign-in login ended with the Machine restart",
                    Reason::LoginEnded,
                )
                .await;
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
                self.stop_recorded(&record, "adoption failed", Reason::Failed)
                    .await
            }
        }
    }

    fn discard_record(&self, record: &InstanceRecord, why: &str) -> Result<()> {
        tracing::info!(instance = %record.instance_id, why, "clearing browser instance record");
        state_file::remove_instance(&self.state_dir, &record.instance_id)
    }

    /// Stop a recorded, unadopted unit; its record is cleared only once the
    /// exit is confirmed.
    async fn stop_recorded(
        &self,
        record: &InstanceRecord,
        why: &str,
        reason: Reason,
    ) -> Result<()> {
        let (cgroup, _) = thaw_browser_cgroup(&record.unit).await;
        let cgroup = cgroup.or_else(|| Some(record.cgroup_dir.clone()));
        if unit::terminate(&record.unit, cgroup.as_deref(), Some(&record.invocation_id)).await {
            return self.discard_record(record, why);
        }
        self.retain_recorded(record, cgroup, reason);
        bail!(
            "browser unit {} exit unconfirmed; record retained",
            record.unit
        )
    }

    /// Keep a recorded unit as running (record, capacity slot, its account's
    /// fence and admission) while a retry bound to its recorded invocation
    /// confirms its exit.
    pub(super) fn retain_recorded(
        &self,
        record: &InstanceRecord,
        cgroup_dir: Option<PathBuf>,
        reason: Reason,
    ) {
        let account = ProviderAccountId::try_from(record.account_id.clone()).ok();
        self.retain_unconfirmed(Retained {
            unit: record.unit.clone(),
            cgroup_dir,
            invocation_id: Some(record.invocation_id.clone()),
            account: account.clone(),
            instance: None,
            record: Some(record.instance_id.clone()),
            capacity: Some(self.capacity.force_reserve()),
            outcome: StopOutcome {
                reason,
                unsaved: true,
                reference: account.map(|account_id| InstanceRef {
                    account_id,
                    instance_id: record.instance_id.clone(),
                    incarnation: record.incarnation,
                }),
            },
        });
    }

    pub(super) async fn adopt(
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
        let instance = Instance::new(InstanceParts {
            account_id,
            instance_id: record.instance_id.clone(),
            incarnation: record.incarnation,
            purpose: record.purpose.clone(),
            deadline_ms: record.deadline_ms,
            unit: record.unit.clone(),
            runtime_dir: record.runtime_dir.clone(),
            cgroup_dir: record.cgroup_dir.clone(),
            invocation_id: record.invocation_id.clone(),
            scale: record.scale,
            profile_revision: record.profile_revision,
            plan: Arc::new(plan),
            settings,
            report: report.clone(),
            control,
            phase: Phase::Detached,
            geometry: record.geometry,
            last_snapshot: control_record
                .current_generation
                .zip(control_record.current_generation_at_ms),
            // A survivor counts against the limit whatever it is now.
            capacity: Some(self.capacity.force_reserve()),
        });
        let bootstrap = host::enforce_label(BOOTSTRAP_PROFILE);
        let sealed = host::enforce_label(SEALED_PROFILE);
        let verified = verify::verify(
            &self.expectations(&instance, status.main_pid, &bootstrap, &sealed),
            &report,
        )
        .map_err(|violation| anyhow!("confinement: {violation}"))?;
        instance.live.lock().verified = verified;
        drop(reservation);
        Ok(Arc::new(instance))
    }

    /// Resume fenced operations and remove orphan generations and abandoned
    /// staging of every account, with the control records as authority. A
    /// fence whose account may still have a running unit stays raised.
    pub(super) async fn reconcile_accounts(&self) -> Result<()> {
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
                if slot.instance.lock().is_some() || self.fence_blocked(&account_id) {
                    tracing::error!(account = %account_id.as_str(), "browser exit unconfirmed; fenced operation deferred");
                    continue;
                }
                complete_fenced(&account_dir, &mut record)?;
                tracing::info!(account = %account_id.as_str(), "resumed fenced browser operation");
                if let Some(completed) = &record.last_operation {
                    self.report_fence_completed(&account_id, completed);
                }
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
}

/// Thaw a unit's `browser/` sub-cgroup before stopping it, so a stop never
/// waits on frozen processes; returns the unit's cgroup and invocation when
/// systemd reports them.
async fn thaw_browser_cgroup(unit_name: &str) -> (Option<PathBuf>, Option<String>) {
    let Ok(status) = unit::show(unit_name).await else {
        return (None, None);
    };
    let cgroup = status.cgroup_dir();
    if let Some(cgroup) = &cgroup {
        let _ = unit::set_frozen(&cgroup.join("browser"), false);
    }
    let invocation = (!status.invocation_id.is_empty()).then_some(status.invocation_id);
    (cgroup, invocation)
}
