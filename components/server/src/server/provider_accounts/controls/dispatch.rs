//! The account gate of one dispatch: admission, account state, uncertain
//! effects and the account's single active turn (R1-8, R3-9).
//!
//! The reservation ID is generated here, never taken from the caller, and
//! every later transition matches account, session and that ID. The final
//! eligibility check (admission, state, uncertain effect, the reservation
//! itself) runs under the control writers' lock, which [`Handoff`] keeps
//! until the prompt was handed to its runtime: no block, unadmission or
//! refusal can be written between the check and the handoff.
use super::*;
use crate::account_controls::ReservationState;

/// The account gate of every dispatch of `session` (user prompts and
/// supervised follow-ups alike). `Ok(None)`: the session is not governed.
pub(in crate::server) async fn before_dispatch(
    state: &Arc<AppState>,
    session: &str,
    explicit: bool,
) -> Result<Option<Turn>, String> {
    let governed = state
        .hub
        .session_info(session)
        .is_some_and(|info| governed(&info.meta));
    if !governed {
        return Ok(None);
    }
    of(state)
        .ok_or_else(|| wait("account controls need the durable store"))?
        .before_dispatch(session, explicit)
        .await
}

/// Hand one prompt of `session` with `delivery` to its runtime through the
/// account gate (supervised follow-ups; the dispatcher interleaves its own
/// preparation between the same steps). `send` runs only after the final
/// check, under the handoff lock; its synchronous refusal releases the slot.
pub(in crate::server) async fn handoff<T>(
    state: &Arc<AppState>,
    session: &str,
    delivery: &str,
    send: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let governed = state
        .hub
        .session_info(session)
        .is_some_and(|info| governed(&info.meta));
    if !governed {
        return send();
    }
    of(state)
        .ok_or_else(|| wait("account controls need the durable store"))?
        .handoff(session, delivery, false, send)
        .await
}

/// Report a dispatch's prompt as never enqueued (see [`Turn::unsent`]).
pub(in crate::server) fn unsent(turn: &mut Option<Turn>) {
    if let Some(turn) = turn.as_mut() {
        turn.unsent();
    }
}

/// The control writers' lock, held from a turn's final eligibility check
/// until its prompt was handed to the runtime (or refused synchronously).
#[derive(Debug)]
pub(in crate::server) struct Handoff(#[allow(dead_code)] tokio::sync::OwnedMutexGuard<()>);

/// A reserved turn of this process's dispatch. Dropping it releases the
/// reservation unless [`Turn::mark`] made it `may_have_dispatched` and the
/// prompt was not reported [`Turn::unsent`].
pub(in crate::server) struct Turn {
    controls: Controls,
    account: String,
    session: String,
    turn_id: String,
    explicit: bool,
    marked: bool,
    unsent: bool,
}

impl Turn {
    pub(in crate::server) fn id(&self) -> &str {
        &self.turn_id
    }

    /// The final eligibility check, then `may_have_dispatched` with the
    /// prompt's `delivery_id`, before the prompt is enqueued. The returned
    /// [`Handoff`] must be held until the prompt was handed to the runtime.
    pub(in crate::server) async fn mark(&mut self, delivery_id: &str) -> Result<Handoff, String> {
        let controls = self.controls.clone();
        let handoff = Arc::clone(&controls.cache.writes).lock_owned().await;
        controls.refresh().await.map_err(|reason| wait(&reason))?;
        let (meta, _) = controls
            .env
            .session(&self.session)
            .ok_or_else(|| wait("the session no longer exists"))?;
        if meta.provider_account_id.as_str() != self.account {
            return Err(wait("the session's account changed"));
        }
        match controls.gate(&meta, self.explicit) {
            Gate::Park(reason) => return Err(wait(&reason)),
            // The dispatch already resumed the account; a rate limit again
            // now is a newer refusal.
            Gate::Proceed {
                clears_rate_limit: true,
            } => return Err(wait("the account's state changed; send again")),
            Gate::Proceed { .. } => {}
        }
        let held = controls.cache.turn(&self.account).is_some_and(|turn| {
            turn.session_id == self.session
                && turn.turn_id == self.turn_id
                && turn.state == ReservationState::Reserved
        });
        if !held {
            return Err(wait("the account's turn reservation changed"));
        }
        let epoch = controls
            .env
            .worker(&self.session)
            .map(|worker| worker.worker_epoch)
            .unwrap_or_default();
        let key = TurnKey {
            session_id: &self.session,
            turn_id: &self.turn_id,
            worker_epoch: &epoch,
            only: Some(ReservationState::Reserved),
        };
        match controls
            .store
            .mark_account_turn_dispatched(
                &self.account,
                &key,
                (delivery_id, &meta.machine_id),
                now_ms(),
            )
            .await
        {
            Ok(true) => {
                self.marked = true;
                let _ = controls.refresh().await;
                Ok(Handoff(handoff))
            }
            Ok(false) => Err(wait("the account's turn reservation changed")),
            Err(_) => Err(wait("account controls are unavailable")),
        }
    }

    /// This dispatch never enqueued the prompt: the enqueue refused it
    /// synchronously, or a path before it requeued the prompt. That is the
    /// Controller's positive, in-process proof that nothing left it, so the
    /// slot is released like a `reserved` one when the guard drops.
    pub(in crate::server) fn unsent(&mut self) {
        self.unsent = true;
    }
}

impl Drop for Turn {
    fn drop(&mut self) {
        let only = if self.unsent {
            None
        } else if self.marked {
            self.controls.cache.dispatching.lock().remove(&self.turn_id);
            return;
        } else {
            Some(ReservationState::Reserved)
        };
        // The reconciler skips this turn until the release is durable.
        let controls = self.controls.clone();
        let (account, session, turn) = (
            self.account.clone(),
            self.session.clone(),
            self.turn_id.clone(),
        );
        tokio::spawn(async move {
            let key = TurnKey {
                session_id: &session,
                turn_id: &turn,
                worker_epoch: "",
                only,
            };
            let _ = controls.store.release_account_turn(&account, &key).await;
            let _ = controls.refresh().await;
            controls.cache.dispatching.lock().remove(&turn);
        });
    }
}

impl Controls {
    /// See [`handoff`].
    pub(in crate::server) async fn handoff<T>(
        &self,
        session: &str,
        delivery: &str,
        explicit: bool,
        send: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let mut turn = self.before_dispatch(session, explicit).await?;
        let _handoff = match turn.as_mut() {
            Some(turn) => Some(turn.mark(delivery).await?),
            None => None,
        };
        let sent = send();
        if sent.is_err() {
            unsent(&mut turn);
        }
        sent
    }

    /// The account gate of one dispatch: admission, account state, uncertain
    /// effects and the account's single active turn. `Err` is the session's
    /// visible wait; the caller requeues the prompt unchanged.
    pub(in crate::server) async fn before_dispatch(
        &self,
        session: &str,
        explicit: bool,
    ) -> Result<Option<Turn>, String> {
        let Some((meta, _)) = self.env.session(session) else {
            return Ok(None);
        };
        if !governed(&meta) {
            return Ok(None);
        }
        if meta.provider_account_id.is_default() {
            return Err(wait("a named Antigravity account is required"));
        }
        self.refresh().await.map_err(|reason| wait(&reason))?;
        let account = meta.provider_account_id.as_str().to_owned();
        match self.gate(&meta, explicit) {
            Gate::Park(reason) => return Err(wait(&reason)),
            Gate::Proceed {
                clears_rate_limit: true,
            } => {
                let control = self
                    .cache
                    .control(&account)
                    .ok_or_else(|| wait("the account's state changed; send again"))?;
                self.enter(
                    &control,
                    AccountState::Ok,
                    "owner_send_after_rate_limit",
                    None,
                    Some(control.revision),
                    false,
                )
                .await
                .map_err(|_| wait("the account's state changed; send again"))?;
            }
            Gate::Proceed { .. } => {}
        }
        // Never a caller's value: two sessions can never name one slot.
        let turn_id = format!("turn-{}", uuid::Uuid::new_v4());
        let now = now_ms();
        // Registered before the row exists, so the reconciler never takes
        // this reservation for an orphan of an earlier Controller.
        self.cache.dispatching.lock().insert(turn_id.clone());
        let wanted = Reservation {
            account_id: account.clone(),
            owner_user_id: meta.owner_user_id.clone().unwrap_or_default(),
            session_id: session.to_owned(),
            turn_id: turn_id.clone(),
            delivery_id: String::new(),
            worker_epoch: String::new(),
            machine_id: String::new(),
            after_seq: i64::try_from(self.env.last_event_seq(session)).unwrap_or(i64::MAX),
            state: ReservationState::Reserved,
            created_at_ms: now,
            updated_at_ms: now,
            terminal: None,
        };
        let holder = self.store.reserve_account_turn(&wanted).await;
        let _ = self.refresh().await;
        let refused = match &holder {
            Ok(holder)
                if holder.turn_id == turn_id
                    && holder.session_id == session
                    && holder.state == ReservationState::Reserved =>
            {
                None
            }
            Ok(holder) => Some(policy::slot_reason(holder, session).unwrap_or_else(|| {
                "the previous turn of this session is still being settled".into()
            })),
            Err(_) => Some("account controls are unavailable".into()),
        };
        if let Some(reason) = refused {
            self.cache.dispatching.lock().remove(&turn_id);
            return Err(wait(&reason));
        }
        Ok(Some(Turn {
            controls: self.clone(),
            account,
            session: session.to_owned(),
            turn_id,
            explicit,
            marked: false,
            unsent: false,
        }))
    }
}
