//! Profile snapshots taken while an instance runs: admission against the
//! staging budget, the frozen copy as a cancellable job outside the account
//! lifecycle lock, and its publication under the fence.

use super::*;

/// How long a stop waits for a cancelled copy to let go of the control
/// connection and its staging generation.
const COPY_RELEASE_WAIT: Duration = Duration::from_secs(5);
/// How long a navigation waits for a running copy to finish before it
/// cancels it: the copy's own bound (the browser is frozen meanwhile and
/// could not navigate anyway).
const COPY_QUIESCE_WAIT: Duration = SNAPSHOT_TIMEOUT.saturating_add(FREEZE_TIMEOUT);

/// Refuses new copies of one instance while held (a navigation verifies
/// and navigates).
pub(super) struct CopyHold<'a>(&'a Instance);

impl Drop for CopyHold<'_> {
    fn drop(&mut self) {
        self.0
            .copy_holds
            .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

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

    /// Refuse new copies until the hold drops. Taken under the copy lock,
    /// where admission registers a job, so a copy is either visible to the
    /// holder or refused.
    pub(super) fn hold_copies(&self) -> CopyHold<'_> {
        let _copy = self.copy.lock();
        self.copy_holds
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        CopyHold(self)
    }

    pub(super) fn copies_held(&self) -> bool {
        self.copy_holds.load(std::sync::atomic::Ordering::SeqCst) > 0
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
    /// Refuse new copies of `instance` and wait until none runs: a running
    /// copy is awaited within its own bound, then cancelled. The caller must
    /// not hold the account lifecycle lock while a copy may run.
    pub(super) async fn quiesce_copies<'a>(&self, instance: &'a Arc<Instance>) -> CopyHold<'a> {
        let hold = instance.hold_copies();
        if let Some(mut done) = instance.copy_done()
            && tokio::time::timeout(COPY_QUIESCE_WAIT, done.wait_for(|done| *done))
                .await
                .is_err()
        {
            tracing::warn!(instance = %instance.instance_id, "browser snapshot overran; cancelled for a navigation");
            instance.cancel_copy();
            if !instance.copy_released().await {
                tracing::warn!(instance = %instance.instance_id, "cancelled browser snapshot did not let go in time");
            }
        }
        hold
    }

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
        // The copier finished: nothing writes the staging any more.
        let published = match lifecycle {
            Some(_lifecycle) => {
                self.publish_snapshot(instance, &account_dir, generation, reservation)
            }
            None => {
                self.retire_staging(&account_dir, generation, reservation);
                Err(anyhow!("snapshot cancelled before publication"))
            }
        };
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
        ensure!(
            !instance.copies_held(),
            "a browser navigation is being verified"
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
        {
            let mut copy = instance.copy.lock();
            if instance.copies_held() {
                drop(copy);
                self.retire_staging(&account_dir, generation, reservation);
                bail!("a browser navigation is being verified");
            }
            *copy = Some(CopyJob {
                cancel: cancel_tx,
                done: done_rx,
            });
        }
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
    /// copy can be admitted, while still enforcing the deadline and the
    /// confinement checks. `Err` is the reason the stop
    /// must become forced (cancelling the copy).
    pub(super) async fn await_copy_idle(&self, instance: &Arc<Instance>) -> Result<(), Reason> {
        loop {
            let Some(mut done) = instance.copy_done() else {
                return Ok(());
            };
            if Instant::now() >= instance.deadline() {
                return Err(Reason::Deadline);
            }
            // Counts denials as the supervisor would: it may be the one
            // waiting here, so the limit is enforced during the wait too.
            self.recheck_periodic(instance)?;
            let _ = tokio::time::timeout(SUPERVISOR_TICK, done.wait_for(|done| *done)).await;
        }
    }

    /// Publish a finished copy and settle its staging accounting on every
    /// path: a refused or failed publication removes the staging before the
    /// budget is released; one that failed after its rename is reconciled
    /// against the durable control record first (it may be published after
    /// all, or be an orphan to remove), and keeps its budget reserved while
    /// that cannot be done. The caller holds the account lifecycle lock.
    pub(super) fn publish_snapshot(
        &self,
        instance: &Arc<Instance>,
        account_dir: &Path,
        generation: u64,
        reservation: StagingReservation,
    ) -> Result<()> {
        let mut record = match state_file::read_control(account_dir) {
            Ok(record)
                if record.pending.is_none()
                    && record.profile_revision == instance.profile_revision =>
            {
                record
            }
            Ok(_) => {
                self.retire_staging(account_dir, generation, reservation);
                bail!("browser profile was reset during the snapshot; staging discarded");
            }
            Err(error) => {
                self.retire_staging(account_dir, generation, reservation);
                return Err(error);
            }
        };
        let stats = match snapshot::publish(
            account_dir,
            generation,
            &mut record,
            snapshot::Bounds::for_profile(instance.settings.profile_max_bytes),
            unix_ms(),
        ) {
            Ok(stats) => {
                drop(reservation);
                stats
            }
            Err(failure) if failure.renamed => {
                if !self.settle_renamed(account_dir, generation, reservation) {
                    return Err(failure.error);
                }
                tracing::warn!(error = %failure, generation, "browser snapshot publication reported a failure but is recorded");
                snapshot::TreeStats::default()
            }
            Err(failure) => {
                self.retire_staging(account_dir, generation, reservation);
                return Err(failure.error);
            }
        };
        // The profile stays potentially dirty: the browser keeps writing (RFB
        // input, timers, sync) after a copy taken while it runs.
        let mut live = instance.live.lock();
        live.last_snapshot = Some((generation, unix_ms()));
        live.last_periodic = Instant::now();
        tracing::info!(
            event_name = "browser.snapshot.published",
            browser_instance = %instance.instance_id,
            generation,
            bytes = stats.bytes,
            entries = stats.entries,
            "browser profile snapshot published"
        );
        Ok(())
    }

    /// Settle a generation renamed into `snapshots/` whose publication then
    /// failed: `true` when the durable control record names it (published),
    /// otherwise it is removed by reconciliation against that record. Only
    /// then is its budget released; a reconciliation that fails keeps the
    /// budget reserved for a retry before the account's next admission.
    pub(super) fn settle_renamed(
        &self,
        account_dir: &Path,
        generation: u64,
        reservation: StagingReservation,
    ) -> bool {
        let account = account_dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let settled = state_file::read_control(account_dir).and_then(|record| {
            if record.current_generation == Some(generation) {
                return Ok(true);
            }
            self.reconcile_storage(&account, account_dir, &record)?;
            Ok(false)
        });
        match settled {
            Ok(published) => {
                drop(reservation);
                published
            }
            Err(error) => {
                tracing::warn!(%error, generation, "unpublished browser snapshot not reconciled; budget retained");
                self.leftover_staging.lock().push(Leftover {
                    account_dir: account_dir.to_path_buf(),
                    generation,
                    reservation,
                    renamed: true,
                });
                false
            }
        }
    }

    /// Remove `account`'s orphan generations and abandoned staging against
    /// its control record. Staging is removed only when nothing outside the
    /// staging ledger may still write it: while the restart inventory is
    /// incomplete, or a unit that may have run for this account (its own, or
    /// one of unknown ownership) has an unconfirmed exit, a surviving
    /// launcher may still be copying into a generation the (restarted)
    /// ledger does not know. That staging is left for the reconciliation
    /// that follows the inventory's completion or the confirmed exit.
    pub(super) fn reconcile_storage(
        &self,
        account: &str,
        account_dir: &Path,
        record: &ControlRecord,
    ) -> Result<Vec<String>> {
        let unresolved = ProviderAccountId::try_from(account.to_owned())
            .map_or(true, |account_id| self.fence_blocked(&account_id));
        if unresolved {
            tracing::info!(
                account,
                "browser staging cleanup deferred until every unit is resolved"
            );
            return snapshot::reconcile_published(account_dir, record);
        }
        let live: BTreeSet<u64> = self
            .staging
            .live()
            .get(account)
            .copied()
            .into_iter()
            .collect();
        snapshot::reconcile(account_dir, record, &live)
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
