//! Confirmed exit. An instance is forgotten (record removed, slot cleared,
//! abandoned staging removed, capacity released) only once its unit is
//! verified gone. A unit whose exit cannot be confirmed stays registered and
//! keeps its account's reset or revocation fence raised while a background
//! retry escalates its termination.

use super::*;

/// How often an unconfirmed unit's termination is retried.
const RETRY_INTERVAL: Duration = Duration::from_secs(15);

/// A unit whose processes may still be running.
pub(super) struct Unconfirmed {
    /// The account it ran for; `None` for an unrecorded unit, which may have
    /// run for any account.
    pub(super) account: Option<String>,
}

/// What to release once an unconfirmed unit's exit is finally confirmed.
pub(super) struct Retained {
    pub(super) unit: String,
    pub(super) cgroup_dir: Option<PathBuf>,
    pub(super) account: Option<ProviderAccountId>,
    /// The registered instance, still in its account slot.
    pub(super) instance: Option<Arc<Instance>>,
    /// An instance record without a registered instance (restart
    /// reconciliation).
    pub(super) record: Option<String>,
    /// A capacity slot held for an unregistered unit.
    pub(super) capacity: Option<CapacityGuard>,
}

impl BrowserManager {
    /// Whether a unit that may still run for `account` (or for an unknown
    /// account) keeps that account's fence from completing.
    pub(super) fn fence_blocked(&self, account: &ProviderAccountId) -> bool {
        self.unconfirmed.lock().values().any(|unit| {
            unit.account
                .as_deref()
                .is_none_or(|owner| owner == account.as_str())
        })
    }

    /// Keep treating `retained.unit` as running and retry its termination in
    /// the background until its exit is confirmed, then release what it
    /// held and resume any fenced operation it blocked.
    pub(super) fn retain_unconfirmed(&self, retained: Retained) {
        self.unconfirmed.lock().insert(
            retained.unit.clone(),
            Unconfirmed {
                account: retained
                    .account
                    .as_ref()
                    .map(|account| account.as_str().to_owned()),
            },
        );
        let Some(manager) = self.this.upgrade() else {
            return;
        };
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(RETRY_INTERVAL).await;
                if unit::terminate(&retained.unit, retained.cgroup_dir.as_deref()).await {
                    break;
                }
                tracing::error!(unit = %retained.unit, "browser unit exit still unconfirmed; retrying");
            }
            tracing::info!(unit = %retained.unit, "browser unit exit confirmed after retry");
            manager.release_retained(retained).await;
        });
    }

    async fn release_retained(&self, retained: Retained) {
        if let Some(instance) = &retained.instance {
            let slot = self.slot(&instance.account_id);
            let _lifecycle = slot.lifecycle.lock().await;
            self.finish_stopped(instance);
        }
        if let Some(instance_id) = &retained.record
            && let Err(error) = state_file::remove_instance(&self.state_dir, instance_id)
        {
            tracing::warn!(%error, "browser instance record not removed");
        }
        drop(retained.capacity);
        self.unconfirmed.lock().remove(&retained.unit);
        if let Err(error) = self.reconcile_accounts().await {
            tracing::warn!(%error, "browser fence resumption incomplete");
        }
    }

    /// Forget an instance whose unit is confirmed gone: its record, its
    /// account slot, the staging a copier of it may have been writing, and
    /// its capacity slot. The caller holds the account lifecycle lock.
    pub(super) fn finish_stopped(&self, instance: &Arc<Instance>) {
        if let Err(error) = state_file::remove_instance(&self.state_dir, &instance.instance_id) {
            tracing::warn!(%error, "browser instance record not removed");
        }
        {
            let slot = self.slot(&instance.account_id);
            let mut current = slot.instance.lock();
            if current
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, instance))
            {
                *current = None;
            }
        }
        let abandoned = instance.abandoned.lock().take();
        if let Some((generation, reservation)) = abandoned {
            self.retire_staging(
                &self
                    .providers
                    .accounts_root()
                    .join(instance.account_id.as_str()),
                generation,
                reservation,
            );
        }
        drop(instance.capacity.lock().take());
    }

    /// Remove a staging generation no copier can write any more, then
    /// release its budget; a failed removal keeps the budget reserved for a
    /// retry.
    pub(super) fn retire_staging(
        &self,
        account_dir: &Path,
        generation: u64,
        reservation: StagingReservation,
    ) {
        match snapshot::remove_staging(account_dir, generation) {
            Ok(()) => drop(reservation),
            Err(error) => {
                tracing::warn!(%error, generation, "abandoned browser staging not removed; budget retained");
                self.leftover_staging.lock().push((
                    account_dir.to_path_buf(),
                    generation,
                    reservation,
                ));
            }
        }
    }

    /// Retry the removal of the account's leftover staging generations. The
    /// caller holds that account's lifecycle lock.
    pub(super) fn sweep_staging(&self, account_dir: &Path) {
        let leftovers: Vec<_> = {
            let mut all = self.leftover_staging.lock();
            let (mine, others) = std::mem::take(&mut *all)
                .into_iter()
                .partition(|(dir, _, _)| dir == account_dir);
            *all = others;
            mine
        };
        for (account_dir, generation, reservation) in leftovers {
            self.retire_staging(&account_dir, generation, reservation);
        }
    }
}
