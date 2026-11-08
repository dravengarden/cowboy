//! Profile snapshots taken while an instance runs: admission against the
//! staging budget, the frozen copy and its publication under the fence.

use super::*;

impl BrowserManager {
    pub(super) async fn snapshot_now(
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
    pub(super) async fn frozen_snapshot(&self, instance: &Arc<Instance>) -> Result<()> {
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

    pub(super) fn publish_snapshot(
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

    pub(super) fn admit_staging(
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
}
