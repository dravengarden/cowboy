//! The account-controls reconciler: settles every reservation from the
//! terminal record of exactly its delivery and worker epoch, or from proven
//! termination of its execution target (R1-8, R3-9, R3-10), records
//! uncertain effects the workers report, and delivers pending Machine
//! fences. It runs at Controller start, then every few seconds; every step
//! is idempotent.
use super::*;
use crate::account_controls::{ReservationState, TurnOutcome};
use crate::remote_runtime::turn_terminal::TurnTerminal;

const TICK: std::time::Duration = std::time::Duration::from_secs(3);
/// A dispatched turn with no terminal record, no running turn and no proven
/// unit loss for this long keeps its slot and asks for the owner.
pub(super) const UNSETTLED_AFTER_MS: i64 = 120_000;

pub(in crate::server) fn start(state: &Arc<AppState>) {
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

/// The listener a runtime calls with every terminal record: a governed
/// session's record is stored on the reservation of exactly its delivery
/// and worker epoch, never on another turn's.
pub(in crate::server) fn terminal_listener(
    state: &Arc<AppState>,
) -> crate::remote_runtime::turn_terminal::Listener {
    let weak = Arc::downgrade(state);
    Arc::new(move |terminal: TurnTerminal| {
        let Some(state) = weak.upgrade() else { return };
        let governed = state
            .hub
            .session_info(&terminal.session_id)
            .is_some_and(|info| governed(&info.meta));
        if !governed {
            return;
        }
        let Some(controls) = of(&state) else { return };
        drop(state);
        tokio::spawn(async move { controls.record_terminal(&terminal).await });
    })
}

/// What the Controller can prove about a turn now.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Observation {
    Settle(TurnOutcome),
    Running,
    Attention,
}

impl Controls {
    /// Store one terminal record on its reservation (see [`terminal_listener`]).
    pub(super) async fn record_terminal(&self, terminal: &TurnTerminal) {
        let Some(delivery) = terminal.cmid.as_deref() else {
            return;
        };
        match self
            .store
            .record_account_turn_terminal(
                &terminal.session_id,
                delivery,
                &terminal.worker_epoch,
                &terminal.stop_reason,
                now_ms(),
            )
            .await
        {
            Ok(true) => {
                let _ = self.refresh().await;
            }
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(%error, "recording an account turn's terminal record failed")
            }
        }
    }

    /// One reconciliation pass; at Controller start it also settles every
    /// reservation an earlier Controller left `reserved`.
    pub(in crate::server) async fn tick(&self) {
        if self.refresh().await.is_err() {
            return;
        }
        self.observe_uncertain_effects();
        let turns: Vec<Reservation> = self.cache.turns.read().values().cloned().collect();
        for turn in turns {
            if self.cache.dispatching.lock().contains(&turn.turn_id) {
                continue;
            }
            self.settle_turn(&turn).await;
        }
        self.run_fences();
        self.run_logouts();
    }

    /// Observe one reservation. Only the terminal record of exactly its
    /// delivery and worker epoch, or proven termination of its execution
    /// target, settles it; anything else waits or asks the owner.
    pub(super) async fn observe(&self, turn: &Reservation, now: i64) -> Observation {
        let provider = self
            .cache
            .control(&turn.account_id)
            .map_or_else(|| "antigravity".to_owned(), |control| control.provider_id);
        if turn.state == ReservationState::Reserved {
            // Not dispatching in this process: an earlier Controller reserved
            // it and stopped before `may_have_dispatched`.
            return Observation::Settle(TurnOutcome::NotDispatched);
        }
        if let Some(terminal) = &turn.terminal
            && (turn.worker_epoch.is_empty() || terminal.worker_epoch == turn.worker_epoch)
        {
            return Observation::Settle(policy::classify_stop_reason(&terminal.stop_reason));
        }
        let lost = match self.env.session(&turn.session_id) {
            // The session is gone without the turn's terminal record.
            None => true,
            Some((meta, in_flight)) => {
                if in_flight || matches!(meta.status, Status::Busy | Status::Starting) {
                    return Observation::Running;
                }
                // A replaced worker incarnation can never end this turn.
                !self.env.live_worker(&turn.session_id)
                    || (!turn.worker_epoch.is_empty()
                        && self
                            .env
                            .worker(&turn.session_id)
                            .is_some_and(|worker| worker.worker_epoch != turn.worker_epoch))
            }
        };
        if lost {
            // A stopped or replaced worker proves nothing about its
            // execution target: only the ledger's termination record of the
            // original binding settles the turn, and only its closed log
            // decides how.
            if let Some(outcome) =
                policy::unit_loss_outcome(self.env.termination(turn, &provider).await.as_ref())
            {
                return Observation::Settle(outcome);
            }
        }
        if now.saturating_sub(turn.updated_at_ms) > UNSETTLED_AFTER_MS {
            Observation::Attention
        } else {
            Observation::Running
        }
    }

    pub(super) async fn settle_turn(&self, turn: &Reservation) {
        let provider = self.cache.control(&turn.account_id).map_or_else(
            || {
                self.env
                    .session(&turn.session_id)
                    .map_or_else(|| "antigravity".to_owned(), |(meta, _)| meta.provider)
            },
            |control| control.provider_id,
        );
        let mut key = TurnKey {
            session_id: &turn.session_id,
            turn_id: &turn.turn_id,
            worker_epoch: &turn.worker_epoch,
            only: Some(turn.state),
        };
        let (outcome, settlement) = match self.observe(turn, now_ms()).await {
            Observation::Running => return,
            Observation::Attention => {
                let current = self
                    .cache
                    .control(&turn.account_id)
                    .map_or(AccountState::Ok, |control| control.state);
                if current.severity() >= AccountState::NeedsAttention.severity() {
                    return;
                }
                let attention = policy::Settlement {
                    release: false,
                    settled: "unsettled",
                    state: Some((AccountState::NeedsAttention, "turn_unsettled".into())),
                    stop_units: false,
                };
                (None, attention)
            }
            Observation::Settle(outcome) => {
                let settlement = policy::settle(turn.state, &outcome);
                (Some(outcome), settlement)
            }
        };
        if outcome == Some(TurnOutcome::NotDispatched) {
            key.only = Some(ReservationState::Reserved);
        }
        match self.settle(turn, key, &provider, &settlement).await {
            Ok(crate::store::account_controls::Settled::Applied { released: true, .. }) => {
                tracing::info!(
                    account = %turn.account_id,
                    session = %turn.session_id,
                    settled = settlement.settled,
                    "account turn settled"
                );
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(
                account = %turn.account_id,
                error,
                "account turn settlement failed; the slot stays held"
            ),
        }
    }

    /// Record each governed session's newest uncertain effect durably, so a
    /// dormant session stays parked until the owner acknowledges it.
    fn observe_uncertain_effects(&self) {
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
            self.env.set_setting(key, record);
        }
    }

    /// Deliver every pending fence whose Machine is connected, one command
    /// per operation at a time. The same operation ID is retried until
    /// confirmed.
    pub(in crate::server) fn run_fences(&self) {
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

    pub(super) async fn deliver(&self, fence: &Fence) {
        let Ok(account_id) = fence.account_id.parse::<ProviderAccountId>() else {
            return;
        };
        let control = self.cache.control(&fence.account_id);
        // The daemon orders fences by the control revision that wrote them
        // and refuses an older one (`conflict`): a delayed admit can never
        // undo a newer confirmed stop.
        let revision = control
            .as_ref()
            .and_then(|control| policy::fence_revision(fence, control.revision));
        let provider_id =
            control.map_or_else(|| "antigravity".to_owned(), |control| control.provider_id);
        let request = AccountFenceRequest {
            operation_id: fence.operation_id.clone(),
            provider_id,
            account_id,
            action: fence.action,
            revision,
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
