//! Restart reconciliation: instance records, surviving units, adoption of
//! `Tab` instances, fenced operations and snapshot storage. A unit is
//! forgotten only once its exit is confirmed; until then its record stays and
//! its account's fence stays raised.

use super::*;

impl BrowserManager {
    pub(super) async fn reconcile_units(self: &Arc<Self>) -> Result<()> {
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
            let cgroup = thaw_browser_cgroup(&unit_name).await;
            if !unit::terminate(&unit_name, cgroup.as_deref()).await {
                // Its account is unknown: every account's fence waits for it.
                self.retain_unconfirmed(Retained {
                    unit: unit_name,
                    cgroup_dir: cgroup,
                    account: None,
                    instance: None,
                    record: None,
                    capacity: Some(self.capacity.force_reserve()),
                });
            }
        }
        Ok(())
    }

    pub(super) async fn reconcile_record(self: &Arc<Self>, record: InstanceRecord) -> Result<()> {
        if record.boot_id != self.boot_id {
            return self.discard_record(&record, "other boot");
        }
        let status = unit::show(&record.unit).await?;
        if status.gone() {
            return self.discard_record(&record, "unit ended");
        }
        if !status.live() || status.invocation_id != record.invocation_id {
            return self.stop_recorded(&record, "unit ended or replaced").await;
        }
        // A sign-in instance's login did not survive the restart.
        if record.purpose != Purpose::Tab {
            return self
                .stop_recorded(&record, "sign-in login ended with the Machine restart")
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
                self.stop_recorded(&record, "adoption failed").await
            }
        }
    }

    fn discard_record(&self, record: &InstanceRecord, why: &str) -> Result<()> {
        tracing::info!(instance = %record.instance_id, why, "clearing browser instance record");
        state_file::remove_instance(&self.state_dir, &record.instance_id)
    }

    /// Stop a recorded, unadopted unit; its record is cleared only once the
    /// exit is confirmed.
    async fn stop_recorded(&self, record: &InstanceRecord, why: &str) -> Result<()> {
        let cgroup = thaw_browser_cgroup(&record.unit)
            .await
            .or_else(|| Some(record.cgroup_dir.clone()));
        if unit::terminate(&record.unit, cgroup.as_deref()).await {
            return self.discard_record(record, why);
        }
        self.retain_unconfirmed(Retained {
            unit: record.unit.clone(),
            cgroup_dir: cgroup,
            account: ProviderAccountId::try_from(record.account_id.clone()).ok(),
            instance: None,
            record: Some(record.instance_id.clone()),
            capacity: Some(self.capacity.force_reserve()),
        });
        bail!(
            "browser unit {} exit unconfirmed; record retained",
            record.unit
        )
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
/// waits on frozen processes; returns the unit's cgroup when systemd reports
/// one.
async fn thaw_browser_cgroup(unit_name: &str) -> Option<PathBuf> {
    let cgroup = unit::show(unit_name).await.ok()?.cgroup_dir()?;
    let _ = unit::set_frozen(&cgroup.join("browser"), false);
    Some(cgroup)
}
