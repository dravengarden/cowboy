//! Confirmed exit. An instance is forgotten (record removed, slot cleared,
//! abandoned staging removed, capacity released) only once its unit is
//! verified gone. A unit whose exit cannot be confirmed stays registered and
//! keeps its account's reset or revocation fence raised while a background
//! retry escalates its termination.

use super::*;

/// How often an unconfirmed unit's termination (or an incomplete restart
/// inventory) is retried.
#[cfg(not(test))]
pub(super) const RETRY_INTERVAL: Duration = Duration::from_secs(15);
#[cfg(test)]
pub(super) const RETRY_INTERVAL: Duration = Duration::from_millis(50);

/// A unit invocation whose processes may still be running.
pub(super) struct Unconfirmed {
    pub(super) unit: String,
    /// The invocation its termination is bound to; `None` stands for
    /// whatever runs under the unit name.
    pub(super) invocation_id: Option<String>,
    /// The account it ran for; `None` for an unrecorded unit, which may have
    /// run for any account.
    pub(super) account: Option<String>,
    /// The incarnation that may still run, when known.
    pub(super) incarnation: Option<InstanceRef>,
    /// The instance record kept until the exit is confirmed.
    pub(super) record: Option<String>,
    /// Why it is being stopped.
    pub(super) reason: Reason,
    /// The runtime generation it was launched from; `None` when unknown,
    /// which keeps every browser generation from retiring.
    pub(super) generation: Option<String>,
}

#[cfg(test)]
impl Unconfirmed {
    /// A record-less entry of unknown invocation.
    pub(super) fn unit(unit: &str, account: Option<&ProviderAccountId>) -> Self {
        Self {
            unit: unit.to_owned(),
            invocation_id: None,
            account: account.map(|account| account.as_str().to_owned()),
            incarnation: None,
            record: None,
            reason: Reason::Failed,
            generation: None,
        }
    }
}

/// A generation whose cleanup failed; its staging budget stays reserved
/// until a retry before the account's next admission completes it.
pub(super) struct Leftover {
    pub(super) account_dir: PathBuf,
    pub(super) generation: u64,
    pub(super) reservation: StagingReservation,
    /// Renamed into `snapshots/` before its publication failed: settled
    /// against the durable control record rather than removed from staging.
    pub(super) renamed: bool,
}

/// What to release once an unconfirmed unit's exit is finally confirmed.
pub(super) struct Retained {
    pub(super) unit: String,
    pub(super) cgroup_dir: Option<PathBuf>,
    /// The systemd invocation the termination is bound to, when known: a
    /// later invocation of the same unit name is never stopped by the retry.
    pub(super) invocation_id: Option<String>,
    pub(super) account: Option<ProviderAccountId>,
    /// The registered instance, still in its account slot.
    pub(super) instance: Option<Arc<Instance>>,
    /// An instance record without a registered instance (restart
    /// reconciliation).
    pub(super) record: Option<String>,
    /// A capacity slot held for an unregistered unit.
    pub(super) capacity: Option<CapacityGuard>,
    /// The stop the confirmed exit completes, reported then as the
    /// incarnation's terminal `hibernated` observation.
    pub(super) outcome: StopOutcome,
    /// The runtime generation the unit was launched from, pinned until its
    /// exit is confirmed; `None` when unknown (an unrecorded unit).
    pub(super) generation: Option<String>,
}

/// Why a retained incarnation was stopped, and whose observation its
/// confirmed exit completes.
pub(super) struct StopOutcome {
    pub(super) reason: Reason,
    pub(super) unsaved: bool,
    /// The incarnation to report when no instance is registered (a recorded
    /// unit at restart); `None` when nobody awaits a report (a failed start
    /// already answered its open).
    pub(super) reference: Option<InstanceRef>,
}

impl StopOutcome {
    /// A stop nobody awaits a separate report of.
    pub(super) fn unreported(reason: Reason) -> Self {
        Self {
            reason,
            unsaved: true,
            reference: None,
        }
    }
}

impl BrowserManager {
    /// Whether a unit that may still run for `account` (or for an unknown
    /// account) keeps that account's fence from completing.
    pub(super) fn fence_blocked(&self, account: &ProviderAccountId) -> bool {
        if self.inventory_unresolved() {
            return true;
        }
        self.unconfirmed.lock().values().any(|unit| {
            unit.account
                .as_deref()
                .is_none_or(|owner| owner == account.as_str())
        })
    }

    /// Whether the restart inventory has not yet accounted for every live
    /// unit invocation.
    pub(super) fn inventory_unresolved(&self) -> bool {
        self.inventory_pending
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Keep treating `retained.unit` as running and retry its termination in
    /// the background until its exit is confirmed, then release what it
    /// held and resume any fenced operation it blocked. Returns the
    /// retention's ticket.
    pub(super) fn retain_unconfirmed(&self, retained: Retained) -> u64 {
        let ticket = self
            .next_retention
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.unconfirmed.lock().insert(
            ticket,
            Unconfirmed {
                unit: retained.unit.clone(),
                invocation_id: retained.invocation_id.clone(),
                account: retained
                    .account
                    .as_ref()
                    .map(|account| account.as_str().to_owned()),
                incarnation: retained.outcome.reference.clone().or_else(|| {
                    retained
                        .instance
                        .as_ref()
                        .map(|instance| instance.reference())
                }),
                record: retained.record.clone(),
                reason: retained.outcome.reason,
                generation: retained.generation.clone(),
            },
        );
        let Some(manager) = self.this.upgrade() else {
            return ticket;
        };
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(RETRY_INTERVAL).await;
                if manager
                    .units
                    .terminate(
                        &retained.unit,
                        retained.cgroup_dir.as_deref(),
                        retained.invocation_id.as_deref(),
                    )
                    .await
                {
                    break;
                }
                tracing::error!(unit = %retained.unit, "browser unit exit still unconfirmed; retrying");
            }
            tracing::info!(unit = %retained.unit, "browser unit exit confirmed after retry");
            manager.release_retained(ticket, retained).await;
        });
        ticket
    }

    /// Whether the manager already accounts for `invocation` of `unit_name`:
    /// a registered instance runs it, or a retention's termination is bound
    /// to it (or to whatever runs under the name).
    pub(super) fn invocation_accounted(&self, unit_name: &str, invocation: &str) -> bool {
        self.running()
            .iter()
            .any(|instance| instance.unit == unit_name && instance.invocation_id == invocation)
            || self.unconfirmed.lock().values().any(|retained| {
                retained.unit == unit_name
                    && retained
                        .invocation_id
                        .as_deref()
                        .is_none_or(|bound| bound == invocation)
            })
    }

    /// Whether an invocation of `unit_name` is retained until its exit is
    /// confirmed.
    pub(super) fn unit_retained(&self, unit_name: &str) -> bool {
        self.unconfirmed
            .lock()
            .values()
            .any(|retained| retained.unit == unit_name)
    }

    /// Whether a registered instance or a retention claims `unit_name`.
    pub(super) fn unit_claimed(&self, unit_name: &str) -> bool {
        self.running()
            .iter()
            .any(|instance| instance.unit == unit_name)
            || self.unit_retained(unit_name)
    }

    /// The `stopping` observation `Query` answers for an account without a
    /// registered instance while a browser may still run for it: the
    /// retained incarnation (with the reason it is being stopped) when one of
    /// this account is known; otherwise, while a unit of unknown ownership or
    /// incarnation is unconfirmed or the restart inventory is incomplete, an
    /// incarnation-less `stopping` with reason `busy`. `None` when nothing
    /// may run for the account.
    pub(super) fn unresolved_state(&self, account_id: &ProviderAccountId) -> Option<StateEvent> {
        let (instance, reason) = {
            let unconfirmed = self.unconfirmed.lock();
            let mine = |retained: &&Unconfirmed| {
                retained
                    .account
                    .as_deref()
                    .is_some_and(|owner| owner == account_id.as_str())
            };
            if let Some(known) = unconfirmed
                .values()
                .filter(mine)
                .find(|retained| retained.incarnation.is_some())
            {
                (known.incarnation.clone(), known.reason)
            } else if self.inventory_unresolved()
                || unconfirmed.values().any(|retained| {
                    retained
                        .account
                        .as_deref()
                        .is_none_or(|owner| owner == account_id.as_str())
                })
            {
                (None, Reason::Busy)
            } else {
                return None;
            }
        };
        let empty = hibernated_event("", account_id, "", reason, None);
        Some(StateEvent {
            instance: instance.unwrap_or(empty.instance.clone()),
            state: State::Stopping,
            ..empty
        })
    }

    /// Release what a unit held once its exit is confirmed, report the
    /// incarnation's terminal `hibernated` observation (so the Controller
    /// leaves `stopping` and any reclaim waiting on it settles), then
    /// complete the fenced operations it blocked, reporting each completion.
    pub(super) async fn release_retained(&self, ticket: u64, retained: Retained) {
        let outcome = retained.outcome;
        let terminal = if let Some(instance) = &retained.instance {
            let slot = self.slot(&instance.account_id);
            let _lifecycle = slot.lifecycle.lock().await;
            self.finish_stopped(instance);
            Some(self.hibernated_after(instance, outcome.reason, None, outcome.unsaved))
        } else {
            outcome.reference.map(|reference| {
                let event = hibernated_event(
                    "",
                    &reference.account_id,
                    &reference.instance_id,
                    outcome.reason,
                    None,
                );
                StateEvent {
                    instance: reference,
                    unsaved_state_lost: outcome.unsaved,
                    ..event
                }
            })
        };
        if let Some(instance_id) = &retained.record
            && let Err(error) = state_file::remove_instance(&self.state_dir, instance_id)
        {
            tracing::warn!(%error, "browser instance record not removed");
        }
        drop(retained.capacity);
        self.unconfirmed.lock().remove(&ticket);
        if let Some(event) = terminal {
            self.send_state(event);
        }
        if let Err(error) = self.reconcile_accounts().await {
            tracing::warn!(%error, "browser fence resumption incomplete");
        }
    }

    /// Report a fenced operation completed after the fact (startup or a
    /// confirmed exit): the account's `hibernated` observation with the
    /// operation's reason and the advanced revision. A reset's request ID is
    /// its operation ID, so its earlier `failed` receipt is replaced and a
    /// replay answers with the completion.
    pub(super) fn report_fence_completed(
        &self,
        account_id: &ProviderAccountId,
        completed: &CompletedOperation,
    ) {
        let reason = match completed.kind {
            OperationKind::Reset => Reason::OwnerRequest,
            OperationKind::Revoke => Reason::Revoked,
        };
        let mut event =
            hibernated_event("", account_id, "", reason, Some(completed.profile_revision));
        if completed.kind == OperationKind::Reset {
            let mut receipts = self.receipts.lock();
            if let Some((_, receipt)) = receipts
                .iter_mut()
                .find(|(id, _)| *id == completed.operation_id)
            {
                event.request_id = Some(completed.operation_id.clone());
                *receipt = event.clone();
            }
        }
        self.send_state(event);
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
                self.leftover_staging.lock().push(Leftover {
                    account_dir: account_dir.to_path_buf(),
                    generation,
                    reservation,
                    renamed: false,
                });
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
                .partition(|leftover: &Leftover| leftover.account_dir == account_dir);
            *all = others;
            mine
        };
        for leftover in leftovers {
            if leftover.renamed {
                self.settle_renamed(
                    &leftover.account_dir,
                    leftover.generation,
                    leftover.reservation,
                );
            } else {
                self.retire_staging(
                    &leftover.account_dir,
                    leftover.generation,
                    leftover.reservation,
                );
            }
        }
    }
}
