//! The account-controls reconciler: settles every reservation from its
//! turn's terminal record (R1-8, R3-9, R3-10), records uncertain effects the
//! workers report, and delivers pending Machine fences. It runs at Controller
//! start, then every few seconds; every step is idempotent.
use super::*;
use crate::account_controls::TurnOutcome;

const TICK: std::time::Duration = std::time::Duration::from_secs(3);
/// A dispatched turn with no terminal record, no running turn and no proven
/// unit loss for this long keeps its slot and asks for the owner.
pub(super) const UNSETTLED_AFTER_MS: i64 = 120_000;

pub(in crate::server) fn start(state: &Arc<AppState>) {
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

/// What the Controller can prove about a turn now.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Observation {
    Settle(TurnOutcome),
    Running,
    Attention,
}

impl Controls {
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

    /// Observe one reservation. Only the turn's terminal record, or proven
    /// loss of its worker, settles it; anything else waits or asks the owner.
    pub(super) fn observe(&self, turn: &Reservation, now: i64) -> Observation {
        if turn.state == ReservationState::Reserved {
            // Not dispatching in this process: an earlier Controller reserved
            // it and stopped before `may_have_dispatched`.
            return Observation::Settle(TurnOutcome::NotDispatched);
        }
        let Some((meta, in_flight)) = self.env.session(&turn.session_id) else {
            // The session and its worker are gone without a terminal record.
            return Observation::Settle(TurnOutcome::UnitLost {
                evidence_complete: false,
            });
        };
        let after = u64::try_from(turn.after_seq).unwrap_or(0);
        if let Some(stop_reason) = self.env.turn_end(&turn.session_id, after) {
            return Observation::Settle(policy::classify_stop_reason(&stop_reason));
        }
        if in_flight || matches!(meta.status, Status::Busy | Status::Starting) {
            return Observation::Running;
        }
        // The Machine reports a sandboxed worker stopped only after the
        // daemon confirmed its unit gone; no backend evidence survives it.
        if matches!(
            meta.status,
            Status::Exited | Status::Crashed | Status::Interrupted
        ) && !self.env.live_worker(&turn.session_id)
            && self.env.machine_connected(&meta.machine_id)
        {
            return Observation::Settle(TurnOutcome::UnitLost {
                evidence_complete: false,
            });
        }
        if now.saturating_sub(turn.updated_at_ms) > UNSETTLED_AFTER_MS {
            Observation::Attention
        } else {
            Observation::Running
        }
    }

    async fn settle_turn(&self, turn: &Reservation) {
        let template = self.cache.control(&turn.account_id).unwrap_or_else(|| {
            let provider = self
                .env
                .session(&turn.session_id)
                .map_or_else(|| "antigravity".to_owned(), |(meta, _)| meta.provider);
            initial(&turn.account_id, &turn.owner_user_id, &provider)
        });
        let outcome = match self.observe(turn, now_ms()) {
            Observation::Running => return,
            Observation::Attention => {
                if template.state.severity() < AccountState::NeedsAttention.severity() {
                    self.settle_state(
                        &template,
                        AccountState::NeedsAttention,
                        "turn_unsettled",
                        false,
                    )
                    .await;
                }
                return;
            }
            Observation::Settle(outcome) => outcome,
        };
        let settlement = policy::settle(turn.state, &outcome);
        // The account parks before its slot frees, so no queued prompt of
        // the account can slip through between the two writes.
        if let Some((next, reason)) = &settlement.state {
            self.settle_state(&template, *next, reason, settlement.stop_units)
                .await;
        }
        if settlement.release {
            let only =
                (outcome == TurnOutcome::NotDispatched).then_some(ReservationState::Reserved);
            match self
                .store
                .release_account_turn(&turn.account_id, &turn.turn_id, only)
                .await
            {
                Ok(true) => tracing::info!(
                    account = %turn.account_id,
                    session = %turn.session_id,
                    settled = settlement.settled,
                    "account turn settled"
                ),
                Ok(false) => {}
                Err(error) => tracing::warn!(%error, "releasing an account turn failed"),
            }
            let _ = self.refresh().await;
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
        let provider_id = self
            .cache
            .control(&fence.account_id)
            .map_or_else(|| "antigravity".to_owned(), |control| control.provider_id);
        let request = AccountFenceRequest {
            operation_id: fence.operation_id.clone(),
            provider_id,
            account_id,
            action: fence.action,
        };
        let result = self.env.fence(fence.machine_id.clone(), request).await;
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
