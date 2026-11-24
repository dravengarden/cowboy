//! The account-controls reconciler: applies backend signals from governed
//! sessions' turn endings, records uncertain effects the workers report, and
//! delivers pending Machine fences and logouts. It runs at Controller start,
//! then every few seconds; every step is idempotent.
use super::*;
use crate::remote_runtime::turn_end::TurnEnd;

const TICK: std::time::Duration = std::time::Duration::from_secs(3);

pub fn start(state: &Arc<AppState>) {
    // Every launch, revive, reset, reload and replacement of a governed
    // session's worker passes the account gate (`Supervisor::start_session`).
    let gate = Arc::downgrade(state);
    state.supervisor.set_launch_gate(Arc::new(move |session| {
        let state = gate.upgrade().ok_or("Controller is stopping")?;
        let Some(info) = state.hub.session_info(session) else {
            return Ok(());
        };
        revival_gate(&state, &info.meta)
    }));
    let weak = Arc::downgrade(state);
    tokio::spawn(async move {
        loop {
            let Some(state) = weak.upgrade() else { return };
            if *state.shutdown.borrow() {
                return;
            }
            if let Some(controls) = of(&state) {
                drop(state);
                controls.tick().await;
            } else {
                drop(state);
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

/// The listener a runtime calls with every turn ending: a governed
/// session's ending moves its account to the state the proxy's structured
/// result names (a backend signal; nothing is settled by it).
pub fn turn_end_listener(state: &Arc<AppState>) -> crate::remote_runtime::turn_end::Listener {
    let weak = Arc::downgrade(state);
    Arc::new(move |end: TurnEnd| {
        let Some(state) = weak.upgrade() else { return };
        let Some(meta) = state
            .hub
            .session_info(&end.session_id)
            .map(|info| info.meta)
            .filter(governed)
        else {
            return;
        };
        let Some(transition) =
            policy::signal_transition(&policy::classify_stop_reason(&end.stop_reason))
        else {
            return;
        };
        let Some(controls) = of(&state) else { return };
        drop(state);
        // Registered before this listener returns (before the turn ending
        // is acknowledged): the account's next dispatch or revival waits
        // until the signal is committed.
        controls.begin_signal(&meta);
        tokio::spawn(async move { controls.signal(&meta, &transition).await });
    })
}

impl Controls {
    pub fn begin_signal(&self, meta: &SessionMeta) {
        *self
            .cache
            .pending_signals
            .lock()
            .entry(meta.provider_account_id.as_str().to_owned())
            .or_default() += 1;
    }

    /// Apply one backend signal of `meta`'s account, registered by
    /// [`Self::begin_signal`]. A store failure is retried until the signal
    /// commits; the account stays parked meanwhile.
    pub async fn signal(&self, meta: &SessionMeta, transition: &policy::Transition) {
        let account = meta.provider_account_id.as_str().to_owned();
        if !meta.provider_account_id.is_default() {
            let owner = meta.owner_user_id.clone().unwrap_or_default();
            let transition = self.scope_rate_limit(meta, transition).await;
            while let Err(error) = self
                .raise(&account, &owner, &meta.provider, &transition)
                .await
            {
                tracing::warn!(
                    account,
                    session = %meta.id,
                    error,
                    "recording an account's backend signal failed; retrying"
                );
                tokio::time::sleep(TICK).await;
            }
        }
        let mut pending = self.cache.pending_signals.lock();
        if let Some(count) = pending.get_mut(&account) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                pending.remove(&account);
            }
        }
    }

    /// A rate limit of `meta`'s turn holds for the quota group of the
    /// session's model (account-wide when the model has none) until that
    /// group's exhausted windows reset, as the account's last usage
    /// observation shows them; without one it holds the default hour.
    async fn scope_rate_limit(
        &self,
        meta: &SessionMeta,
        transition: &policy::Transition,
    ) -> policy::Transition {
        let mut scoped = transition.clone();
        if transition.state != AccountState::RateLimited {
            return scoped;
        }
        // Reread after the turn: a model switch during it shows as a
        // disagreement (no group, account-wide) or, at worst, scopes the
        // wrong group, whose next refusal widens the limit to the account.
        let group = self.quota_group(&meta.id);
        scoped.reason = policy::rate_limit_reason(group);
        if let Some(group) = group {
            scoped.until_ms = self
                .group_reset_ms(
                    meta.owner_user_id.clone().unwrap_or_default(),
                    &meta.provider_account_id,
                    group,
                )
                .await;
        }
        scoped
    }

    /// When `group`'s exhausted quota windows of `account` reset, from the
    /// account's last usage observation.
    pub(super) async fn group_reset_ms(
        &self,
        owner: String,
        account: &ProviderAccountId,
        group: &str,
    ) -> Option<i64> {
        let limits = self.env.rate_limits(owner, account.clone()).await?;
        policy::group_reset_ms(&limits, group, now_ms())
    }

    /// One reconciliation pass: record uncertain effects, deliver fences and
    /// logouts.
    pub async fn tick(&self) {
        if self.refresh().await.is_err() {
            return;
        }
        self.observe_uncertain_effects().await;
        self.close_stop_fences().await;
        self.run_fences();
        self.run_logouts();
    }

    /// A stopped account's fence list is computed when its state is
    /// written; a Machine that gained the account concurrently (a first
    /// sign-in racing the block) is missing from it. Every pass adds a stop
    /// fence of the current revision for such a Machine.
    pub async fn close_stop_fences(&self) {
        let stopped: Vec<Control> = self
            .cache
            .controls
            .read()
            .values()
            .filter(|control| control.state.stops_units())
            .cloned()
            .collect();
        for control in stopped {
            let Ok(machines) = self.account_machines(&control.account_id).await else {
                continue;
            };
            let fenced: HashSet<String> = self
                .cache
                .account_fences(&control.account_id)
                .into_iter()
                .map(|fence| fence.machine_id)
                .collect();
            let missing: Vec<String> = machines
                .into_iter()
                .filter(|machine| !fenced.contains(machine))
                .collect();
            if missing.is_empty() {
                continue;
            }
            let _write = self.cache.writes.lock().await;
            match self
                .store
                .add_missing_stop_fences(&control.account_id, &missing, control.revision, now_ms())
                .await
            {
                Ok(0) => {}
                Ok(_) => {
                    tracing::warn!(
                        account = %control.account_id,
                        machines = ?missing,
                        "stop fence added for a Machine that gained a stopped account"
                    );
                    let _ = self.refresh().await;
                }
                Err(error) => tracing::warn!(%error, "adding a missing stop fence failed"),
            }
        }
    }

    /// Record each governed session's newest uncertain effect durably, so a
    /// dormant session stays parked until the owner acknowledges it.
    async fn observe_uncertain_effects(&self) {
        // Serialized with the owner's acknowledgement of the same record.
        let _write = self.cache.writes.lock().await;
        for meta in self.env.sessions() {
            if !governed(&meta) {
                continue;
            }
            let Some(effect) = self
                .env
                .worker(&meta.id)
                .and_then(|worker| worker.uncertain_effect)
            else {
                continue;
            };
            let key = uncertain_key(&meta.id);
            let mut record = self.env.setting(&key).unwrap_or_else(|| json!({}));
            let sequence = effect.sequence.max(1);
            if record["observed"].as_u64().unwrap_or(0) >= sequence {
                continue;
            }
            record["observed"] = json!(sequence);
            record["method"] = json!(effect.method);
            record["observed_at_ms"] = json!(now_ms());
            // Durable, then published, both under the writer lock the
            // acknowledgement holds: the record only ever moves forward.
            if let Err(error) = self.store.put_setting(&key, &record).await {
                tracing::warn!(session = %meta.id, %error, "recording an uncertain effect failed");
                continue;
            }
            self.env.set_setting(key, record);
        }
    }

    /// Deliver every pending fence whose Machine is connected, one command
    /// per operation at a time. The same operation ID is retried until
    /// confirmed.
    pub fn run_fences(&self) {
        let pending: Vec<Fence> = self
            .cache
            .fences
            .read()
            .iter()
            .filter(|fence| fence.state == FenceState::Pending)
            .cloned()
            .collect();
        for fence in pending {
            if !self.env.machine_connected(&fence.machine_id)
                || !self.cache.fencing.lock().insert(fence.operation_id.clone())
            {
                continue;
            }
            let controls = self.clone();
            tokio::spawn(async move {
                controls.deliver(&fence).await;
                controls.cache.fencing.lock().remove(&fence.operation_id);
                let _ = controls.refresh().await;
            });
        }
    }

    pub async fn deliver(&self, fence: &Fence) {
        let Ok(account_id) = fence.account_id.parse::<ProviderAccountId>() else {
            return;
        };
        // The daemon orders fences by the control revision that wrote them
        // and refuses an older one (`conflict`): a delayed admit can never
        // undo a newer confirmed stop. A fence whose originating revision
        // cannot be established is never sent unordered; it stays pending
        // and is retried.
        let Some((revision, provider_id)) =
            self.cache.control(&fence.account_id).and_then(|control| {
                Some((
                    policy::fence_revision(fence, control.revision)?,
                    control.provider_id,
                ))
            })
        else {
            tracing::warn!(
                account = %fence.account_id,
                machine = %fence.machine_id,
                "account fence revision unknown; retrying"
            );
            let _ = self
                .store
                .record_account_fence(fence, false, "fence revision unknown", now_ms())
                .await;
            return;
        };
        let request = AccountFenceRequest {
            operation_id: fence.operation_id.clone(),
            provider_id,
            account_id,
            action: fence.action,
            revision: Some(revision),
        };
        let result = self.env.fence(fence.machine_id.clone(), request).await;
        // A refusal, including the daemon's `conflict` for a superseded
        // operation, never confirms the fence; the newer fence replaces it.
        let (confirmed, detail) = match &result {
            Ok(observation) => (
                true,
                format!("stopped {} unit(s)", observation.stopped.len()),
            ),
            Err(detail) => (false, detail.clone()),
        };
        match self
            .store
            .record_account_fence(fence, confirmed, &detail, now_ms())
            .await
        {
            Ok(_) if confirmed => tracing::info!(
                account = %fence.account_id,
                machine = %fence.machine_id,
                action = ?fence.action,
                "account fence confirmed"
            ),
            Ok(_) => tracing::warn!(
                account = %fence.account_id,
                machine = %fence.machine_id,
                detail,
                "account fence unconfirmed; retrying"
            ),
            Err(error) => tracing::warn!(%error, "recording an account fence failed"),
        }
    }
}
