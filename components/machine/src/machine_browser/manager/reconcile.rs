//! Restart reconciliation: instance records, surviving units, adoption of
//! `Tab` instances, fenced operations and snapshot storage. A unit is
//! forgotten only once its exit is confirmed; until then its record stays and
//! its account's fence stays raised.
//!
//! The unit inventory is one explicit phase: an inventory-wide fence is raised
//! before any record is read and lowered only once a pass has accounted for
//! every live unit invocation (registered as an adopted instance, or retained
//! until its confirmed exit). While it is raised every account's reset and
//! revocation completion and every open are held, so a termination retry that
//! finishes meanwhile cannot complete a fence of an account whose browser the
//! inventory has not reached yet.

use super::*;

impl BrowserManager {
    /// Raise the inventory fence and run the first inventory pass; an
    /// incomplete pass keeps the fence raised and is retried in full in the
    /// background.
    pub(super) async fn reconcile_units(self: &Arc<Self>) -> Result<()> {
        self.inventory_pending
            .store(true, std::sync::atomic::Ordering::SeqCst);
        match self.inventory_pass().await {
            Ok(()) => {
                self.inventory_pending
                    .store(false, std::sync::atomic::Ordering::SeqCst);
                self.pins.inventoried();
                Ok(())
            }
            Err(error) => {
                self.retry_inventory();
                Err(error.context("browser unit inventory incomplete; every fence held"))
            }
        }
    }

    /// Keep the inventory fence raised and repeat full passes until one
    /// accounts for every live invocation, then lower it and resume the
    /// fenced operations it held.
    pub(super) fn retry_inventory(&self) {
        self.inventory_pending
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let Some(manager) = self.this.upgrade() else {
            return;
        };
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(termination::RETRY_INTERVAL).await;
                match manager.inventory_pass().await {
                    Ok(()) => break,
                    Err(error) => {
                        tracing::error!(%error, "browser unit inventory still incomplete");
                    }
                }
            }
            manager
                .inventory_pending
                .store(false, std::sync::atomic::Ordering::SeqCst);
            manager.pins.inventoried();
            tracing::info!("browser unit inventory completed after retry");
            if let Err(error) = manager.reconcile_accounts().await {
                tracing::warn!(%error, "browser fence resumption incomplete");
            }
        });
    }

    /// One full inventory pass. Every readable record the manager does not
    /// already account for is reconciled (adopted, cleared once its
    /// invocation is gone, or retained), then every listed unit's live
    /// invocation is classified: one registered or retained by the manager
    /// is accounted for, any other (unrecorded, or a replacement of a
    /// recorded invocation) is stopped, or retained when its exit is
    /// unconfirmed. `Ok` only when every live invocation is accounted for.
    pub(super) async fn inventory_pass(self: &Arc<Self>) -> Result<()> {
        // Unreadable records leave every live unit unrecorded: it is stopped
        // rather than adopted, which is always safe.
        let records = state_file::read_instances(&self.state_dir).unwrap_or_else(|error| {
            tracing::warn!(%error, "browser instance records unreadable; stopping every unit");
            Vec::new()
        });
        for record in records {
            if self.record_accounted(&record) {
                continue;
            }
            if let Err(error) = self.reconcile_record(record).await {
                tracing::warn!(%error, "browser instance record reconciliation failed");
            }
        }
        let units = self
            .units
            .list()
            .await
            .context("browser unit listing unavailable")?;
        let mut unclassified = 0_usize;
        for unit_name in units {
            if !self.reconcile_listed(&unit_name).await {
                unclassified += 1;
            }
        }
        ensure!(
            unclassified == 0,
            "{unclassified} browser unit(s) could not be classified"
        );
        Ok(())
    }

    /// Whether a record's instance is registered or retained already (an
    /// earlier, incomplete pass reached it).
    fn record_accounted(&self, record: &InstanceRecord) -> bool {
        self.running()
            .iter()
            .any(|instance| instance.instance_id == record.instance_id)
            || self
                .unconfirmed
                .lock()
                .values()
                .any(|retained| retained.record.as_deref() == Some(&record.instance_id))
    }

    /// Classify one listed unit; `false` when its invocation cannot be told
    /// apart from one the manager already accounts for.
    async fn reconcile_listed(&self, unit_name: &str) -> bool {
        let status = match self.units.show(unit_name).await {
            Ok(status) => status,
            Err(error) if self.unit_claimed(unit_name) => {
                // A replacement cannot be told from the claimed invocation.
                tracing::warn!(%error, unit = %unit_name, "claimed browser unit unobservable; inventory incomplete");
                return false;
            }
            Err(error) => {
                // Unclaimed: whatever runs under the name is stopped.
                tracing::warn!(%error, unit = %unit_name, "stopping unobservable unrecorded browser unit");
                self.stop_unrecorded(unit_name, None, None).await;
                return true;
            }
        };
        if status.gone() {
            return true;
        }
        if status.invocation_id.is_empty() {
            if self.unit_claimed(unit_name) {
                tracing::warn!(unit = %unit_name, "claimed browser unit reports no invocation; inventory incomplete");
                return false;
            }
            self.stop_unrecorded(unit_name, status.cgroup_dir(), None)
                .await;
            return true;
        }
        if self.invocation_accounted(unit_name, &status.invocation_id) {
            return true;
        }
        tracing::warn!(unit = %unit_name, invocation = %status.invocation_id, "stopping unrecorded browser unit invocation");
        let invocation = status.invocation_id.clone();
        self.stop_unrecorded(unit_name, status.cgroup_dir(), Some(invocation))
            .await;
        true
    }

    /// Stop a unit invocation no record accounts for; one whose exit is
    /// unconfirmed is retained with every account's fence held (it may have
    /// run for any account).
    async fn stop_unrecorded(
        &self,
        unit_name: &str,
        cgroup: Option<PathBuf>,
        invocation: Option<String>,
    ) {
        if let Some(cgroup) = &cgroup {
            let _ = unit::set_frozen(&cgroup.join("browser"), false);
        }
        if self
            .units
            .terminate(unit_name, cgroup.as_deref(), invocation.as_deref())
            .await
        {
            return;
        }
        self.retain_unconfirmed(Retained {
            unit: unit_name.to_owned(),
            cgroup_dir: cgroup,
            invocation_id: invocation,
            account: None,
            instance: None,
            record: None,
            capacity: Some(self.capacity.force_reserve()),
            outcome: StopOutcome::unreported(Reason::Failed),
            generation: None,
        });
    }

    pub(super) async fn reconcile_record(self: &Arc<Self>, record: InstanceRecord) -> Result<()> {
        if record.boot_id != self.boot_id {
            return self.discard_record(&record, "other boot");
        }
        let status = match self.units.show(&record.unit).await {
            Ok(status) => status,
            Err(error) => {
                // Unobservable is not gone: the unit keeps its record,
                // capacity and its account's fences and admission held
                // while a retry bound to its recorded invocation stops it.
                self.retain_recorded(&record, Some(record.cgroup_dir.clone()), Reason::Failed);
                return Err(error.context("surviving browser unit unobservable; retained"));
            }
        };
        // The recorded invocation ended (a reused unit name runs another
        // one only after it did). A replacement invocation is not this
        // record's: the unit listing classifies it as unrecorded.
        if unit::invocation_gone(&status, Some(&record.invocation_id)) {
            return self.discard_record(&record, "unit ended or replaced");
        }
        if !status.live() {
            return self
                .stop_recorded(&record, "unit ending", Reason::Failed)
                .await;
        }
        // An instance recorded by an earlier Machine under the retired
        // sign-in purpose is not adopted.
        if record.purpose != Purpose::Tab {
            return self
                .stop_recorded(&record, "retired sign-in browser", Reason::NotAvailable)
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
        let _ = unit::set_frozen(&record.cgroup_dir.join("browser"), false);
        let cgroup = Some(record.cgroup_dir.clone());
        if self
            .units
            .terminate(&record.unit, cgroup.as_deref(), Some(&record.invocation_id))
            .await
        {
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
            generation: Some(record.generation_digest.clone()),
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
                settings.memory_reserve,
                self.headroom,
                || false,
                Duration::ZERO,
                MEMORY_POLL,
            )
            .await
        {
            Admission::Reserved { reservation, .. } => reservation,
            _ => bail!(
                "no memory headroom to adopt: headroom {:?} bytes, reserved {} bytes, requested {} bytes, floor {FLOOR_BYTES} bytes",
                (self.headroom)(),
                LEDGER.reserved_bytes(),
                settings.memory_reserve
            ),
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
            // Staging cleanup is deferred, for every account, while a unit
            // that may write this account's staging is unresolved.
            let removed = self.reconcile_storage(account_id.as_str(), &account_dir, &record)?;
            if !removed.is_empty() {
                tracing::info!(account = %account_id.as_str(), ?removed, "reconciled browser snapshot storage");
            }
        }
        Ok(())
    }
}
