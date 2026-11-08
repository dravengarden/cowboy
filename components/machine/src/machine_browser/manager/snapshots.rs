//! Profile snapshots taken while an instance runs: admission against the
//! staging budget, the frozen copy as a cancellable job outside the account
//! lifecycle lock, and its publication under the fence.

use super::*;

/// How long a stop waits for a cancelled copy to let go of the control
/// connection and its staging generation.
const COPY_RELEASE_WAIT: Duration = Duration::from_secs(5);

/// A running snapshot copy. Any stop cancels it and waits until it has let
/// go of the control connection and handed its staging over.
pub(super) struct CopyJob {
    cancel: tokio::sync::watch::Sender<bool>,
    done: tokio::sync::watch::Receiver<bool>,
}

/// Unregisters the job and reports its completion on every path.
struct JobRegistration<'a> {
    instance: &'a Instance,
    done: tokio::sync::watch::Sender<bool>,
}

impl Drop for JobRegistration<'_> {
    fn drop(&mut self) {
        *self.instance.copy.lock() = None;
        let _ = self.done.send(true);
    }
}

/// A generation admitted for one copy, with the job registered.
struct AdmittedCopy<'a> {
    account_dir: PathBuf,
    generation: u64,
    reservation: StagingReservation,
    cancel: tokio::sync::watch::Receiver<bool>,
    registration: JobRegistration<'a>,
}

/// Why a copy did not produce a publishable staging tree.
enum CopyFailure {
    /// No copier runs any more (it never started, finished, or was killed
    /// and reaped by the launcher): the staging can be removed now.
    Settled(anyhow::Error),
    /// A copier may still be writing until the unit exits.
    Unsettled(anyhow::Error),
}

impl Instance {
    pub(super) fn copy_running(&self) -> bool {
        self.copy.lock().is_some()
    }

    pub(super) fn cancel_copy(&self) {
        if let Some(job) = self.copy.lock().as_ref() {
            let _ = job.cancel.send(true);
        }
    }

    fn copy_done(&self) -> Option<tokio::sync::watch::Receiver<bool>> {
        self.copy.lock().as_ref().map(|job| job.done.clone())
    }

    /// Wait, bounded, until no copy job holds the control connection or a
    /// staging generation outside `abandoned`.
    pub(super) async fn copy_released(&self) -> bool {
        let Some(mut done) = self.copy_done() else {
            return true;
        };
        tokio::time::timeout(COPY_RELEASE_WAIT, done.wait_for(|done| *done))
            .await
            .is_ok()
    }
}

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
        match self.snapshot_job(&instance).await {
            Ok(()) => instance.state(Some(request_id.to_owned())),
            Err(error) => {
                tracing::warn!(%error, "browser snapshot failed");
                instance
                    .state(Some(request_id.to_owned()))
                    .with_reason(Reason::StorageExceeded)
            }
        }
    }

    /// Start a periodic copy in the background; supervision never waits for
    /// it, so leases, scans, deadlines and authority checks keep running.
    pub(super) fn spawn_snapshot(self: &Arc<Self>, instance: &Arc<Instance>) {
        let manager = Arc::clone(self);
        let instance = Arc::clone(instance);
        tokio::spawn(async move {
            if let Err(error) = manager.snapshot_job(&instance).await {
                tracing::warn!(%error, instance = %instance.instance_id, "periodic browser snapshot failed");
            }
        });
    }

    /// One frozen snapshot of a running instance. The account lifecycle lock
    /// is held only to admit the generation and to publish it; the freeze and
    /// copy run without it, and any stop cancels them. The browser cgroup is
    /// thawed on every path, and a staging generation a copier may still
    /// write stays reserved until the unit's exit is confirmed.
    pub(super) async fn snapshot_job(&self, instance: &Arc<Instance>) -> Result<()> {
        let slot = self.slot(&instance.account_id);
        let AdmittedCopy {
            account_dir,
            generation,
            reservation,
            mut cancel,
            registration,
        } = {
            let _lifecycle = slot.lifecycle.lock().await;
            self.admit_copy(instance)?
        };
        let copied = tokio::select! {
            biased;
            _ = cancel.wait_for(|cancelled| *cancelled) => {
                Err(CopyFailure::Unsettled(anyhow!("snapshot cancelled by a stop")))
            }
            copied = self.frozen_copy(instance, generation) => copied,
        };
        if let Err(error) = unit::set_frozen(&instance.cgroup_dir.join("browser"), false) {
            tracing::error!(%error, instance = %instance.instance_id, "browser cgroup not thawed");
        }
        match copied {
            Ok(()) => {}
            Err(CopyFailure::Unsettled(error)) => {
                *instance.abandoned.lock() = Some((generation, reservation));
                return Err(error);
            }
            Err(CopyFailure::Settled(error)) => {
                self.retire_staging(&account_dir, generation, reservation);
                return Err(error);
            }
        }
        let lifecycle = tokio::select! {
            biased;
            _ = cancel.wait_for(|cancelled| *cancelled) => None,
            guard = slot.lifecycle.lock() => Some(guard),
        };
        let published = match lifecycle {
            Some(_lifecycle) => self.publish_snapshot(instance, &account_dir, generation),
            None => Err(anyhow!("snapshot cancelled before publication")),
        };
        match published {
            Ok(()) => drop(reservation),
            // The copier finished: nothing writes the staging any more.
            Err(_) => self.retire_staging(&account_dir, generation, reservation),
        }
        drop(registration);
        published
    }

    /// Under the account lifecycle lock: allocate and reserve a staging
    /// generation and register the job.
    fn admit_copy<'a>(&self, instance: &'a Arc<Instance>) -> Result<AdmittedCopy<'a>> {
        ensure!(
            matches!(
                instance.live.lock().phase,
                Phase::Detached | Phase::Attached
            ),
            "browser is not running"
        );
        ensure!(
            !instance.copy_running(),
            "a browser snapshot is already in progress"
        );
        let account_dir = self.account_dir(&instance.account_id)?;
        let mut record = state_file::read_control(&account_dir)?;
        ensure!(
            record.pending.is_none() && record.profile_revision == instance.profile_revision,
            "browser profile was reset since this instance started"
        );
        let generation = record.allocate_generation();
        state_file::write_control(&account_dir, &record)?;
        let reservation = self.admit_staging(
            &instance.account_id,
            &account_dir,
            generation,
            &instance.settings,
        )?;
        if let Err(error) = snapshot::create_staging(&account_dir, generation) {
            self.retire_staging(&account_dir, generation, reservation);
            return Err(error);
        }
        let (cancel_tx, cancel) = tokio::sync::watch::channel(false);
        let (done, done_rx) = tokio::sync::watch::channel(false);
        *instance.copy.lock() = Some(CopyJob {
            cancel: cancel_tx,
            done: done_rx,
        });
        Ok(AdmittedCopy {
            account_dir,
            generation,
            reservation,
            cancel,
            registration: JobRegistration {
                instance: instance.as_ref(),
                done,
            },
        })
    }

    /// Freeze the browser sub-cgroup and let the launcher copy the profile.
    async fn frozen_copy(
        &self,
        instance: &Arc<Instance>,
        generation: u64,
    ) -> Result<(), CopyFailure> {
        let browser_cgroup = instance.cgroup_dir.join("browser");
        unit::set_frozen(&browser_cgroup, true).map_err(CopyFailure::Settled)?;
        match unit::wait_frozen(&browser_cgroup, FREEZE_TIMEOUT).await {
            Ok(true) => {}
            Ok(false) => {
                return Err(CopyFailure::Settled(anyhow!(
                    "browser did not freeze in time"
                )));
            }
            Err(error) => return Err(CopyFailure::Settled(error)),
        }
        instance
            .control
            .lock()
            .await
            .snapshot(generation, SNAPSHOT_TIMEOUT)
            .await
            .map(|_| ())
            .map_err(|error| {
                // A launcher reply means its copier has ended; a transport
                // failure or timeout leaves it possibly running.
                if error.downcast_ref::<LauncherRefusal>().is_some() {
                    CopyFailure::Settled(error)
                } else {
                    CopyFailure::Unsettled(error)
                }
            })
    }

    /// A graceful stop first lets a running copy finish, so the stop-time
    /// copy can be admitted, while still enforcing the deadline, the login
    /// binding and the confinement checks. `Err` is the reason the stop
    /// must become forced (cancelling the copy).
    pub(super) async fn await_copy_idle(&self, instance: &Arc<Instance>) -> Result<(), Reason> {
        loop {
            let Some(mut done) = instance.copy_done() else {
                return Ok(());
            };
            if Instant::now() >= instance.deadline() {
                return Err(Reason::Deadline);
            }
            if let Some(Reason::LoginMismatch) = self.login_binding_broken(instance) {
                return Err(Reason::LoginMismatch);
            }
            self.recheck(instance)?;
            let _ = tokio::time::timeout(SUPERVISOR_TICK, done.wait_for(|done| *done)).await;
        }
    }

    pub(super) fn publish_snapshot(
        &self,
        instance: &Arc<Instance>,
        account_dir: &Path,
        generation: u64,
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
        // The profile stays potentially dirty: the browser keeps writing (RFB
        // input, timers, sync) after a copy taken while it runs.
        let mut live = instance.live.lock();
        live.last_snapshot = Some((generation, unix_ms()));
        live.last_periodic = Instant::now();
        tracing::info!(
            instance = %instance.instance_id,
            generation,
            bytes = stats.bytes,
            entries = stats.entries,
            "browser profile snapshot published"
        );
        Ok(())
    }

    /// Reserve one staging generation, after retrying the removal of the
    /// account's leftover staging. The caller holds the lifecycle lock.
    pub(super) fn admit_staging(
        &self,
        account_id: &ProviderAccountId,
        account_dir: &Path,
        generation: u64,
        settings: &Settings,
    ) -> Result<StagingReservation> {
        self.sweep_staging(account_dir);
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
