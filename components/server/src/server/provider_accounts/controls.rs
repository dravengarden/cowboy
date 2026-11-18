//! Account controls of named-only Providers (Antigravity) on the Controller
//! (`docs/antigravity-real-accounts-design.md`, "Account controls", "Kill
//! switch and logout"). The policy lives in `crate::account_controls`; the
//! durable records in `crate::store::account_controls`. This module applies
//! them at every boundary:
//!
//! - admission (`accounts.antigravity_admitted`) and the kill switch at
//!   login start and completion, session creation ([`super::named_only_admitted`]),
//!   every prompt dispatch ([`Controls::before_dispatch`]) and every worker
//!   revival ([`revival_gate`]);
//! - the account's single active turn: reserved before dispatch, marked
//!   `may_have_dispatched` before the prompt is enqueued ([`Turn::mark`]),
//!   settled by the reconciler from the turn's terminal record;
//! - Machine fences that stop or admit the account's units.
//!
//! Prompts never leave the durable queue while parked: a refused dispatch
//! requeues its prompt with the reason as the session's visible wait.
use super::*;
use crate::account_controls::{
    self as policy, AccountState, Control, Fence, FenceState, Gate, GateInput, Reservation,
    ReservationState,
};
use crate::core::SessionMeta;
use crate::provider_account::{
    AccountFenceAction, AccountFenceObservation, AccountFenceRequest, ProviderAccountId,
};
use crate::store::account_controls::StateChange;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

pub(in crate::server) mod http;
mod logout;
mod reconcile;
pub(in crate::server) use reconcile::start;

/// What the controls observe of the rest of the Controller. [`AppEnv`] is
/// the production binding; tests substitute their own.
pub(in crate::server) trait Env: Send + Sync + 'static {
    /// A session's metadata and whether a prompt of it is in flight.
    fn session(&self, id: &str) -> Option<(SessionMeta, bool)>;
    fn sessions(&self) -> Vec<SessionMeta>;
    /// The stop reason of the first turn end after `after_seq`.
    fn turn_end(&self, session: &str, after_seq: u64) -> Option<String>;
    fn last_event_seq(&self, session: &str) -> u64;
    fn live_worker(&self, session: &str) -> bool;
    fn worker(&self, session: &str) -> Option<crate::runtime_wire::WorkerSnapshot>;
    fn machine_connected(&self, machine: &str) -> bool;
    fn admitted(&self, account: &ProviderAccountId) -> bool;
    fn setting(&self, key: &str) -> Option<Value>;
    fn set_setting(&self, key: String, value: Value);
    fn fence(
        &self,
        machine: String,
        request: AccountFenceRequest,
    ) -> futures::future::BoxFuture<'static, Result<AccountFenceObservation, String>>;
    /// Run one native logout on `machine` (the Machine's existing native
    /// logout command; sandboxed accounts use the daemon's logout unit).
    fn native_logout(
        &self,
        machine: String,
        request: crate::provider_account::NativeLogoutRequest,
    ) -> futures::future::BoxFuture<
        'static,
        Result<crate::provider_account::NativeLogoutObservation, String>,
    >;
}

/// The production [`Env`].
pub(in crate::server) struct AppEnv(Arc<AppState>);

impl Env for AppEnv {
    fn session(&self, id: &str) -> Option<(SessionMeta, bool)> {
        let info = self.0.hub.session_info(id)?;
        Some((info.meta, self.0.hub.session_has_in_flight_prompt(id)))
    }

    fn sessions(&self) -> Vec<SessionMeta> {
        self.0.hub.session_list()
    }

    fn turn_end(&self, session: &str, after_seq: u64) -> Option<String> {
        self.0.hub.managed_turn(session, after_seq)?.stop_reason
    }

    fn last_event_seq(&self, session: &str) -> u64 {
        self.0.hub.last_event_seq(session).unwrap_or(0)
    }

    fn live_worker(&self, session: &str) -> bool {
        self.0.supervisor.has_live_worker(session)
    }

    fn worker(&self, session: &str) -> Option<crate::runtime_wire::WorkerSnapshot> {
        let machine = self.0.hub.session_info(session)?.meta.machine_id;
        self.0
            .runtime_router
            .runtime(&machine)?
            .worker_snapshot(session)
    }

    fn machine_connected(&self, machine: &str) -> bool {
        self.0.machine_control.operation_connection(machine).is_ok()
    }

    fn admitted(&self, account: &ProviderAccountId) -> bool {
        admitted(&self.0, account)
    }

    fn setting(&self, key: &str) -> Option<Value> {
        self.0.hub.setting(key)
    }

    fn set_setting(&self, key: String, value: Value) {
        self.0.hub.set_setting(key, value);
    }

    fn fence(
        &self,
        machine: String,
        request: AccountFenceRequest,
    ) -> futures::future::BoxFuture<'static, Result<AccountFenceObservation, String>> {
        let state = Arc::clone(&self.0);
        Box::pin(async move {
            state
                .machine_control
                .account_sandbox_fence(&machine, &request)
                .await
        })
    }

    fn native_logout(
        &self,
        machine: String,
        request: crate::provider_account::NativeLogoutRequest,
    ) -> futures::future::BoxFuture<
        'static,
        Result<crate::provider_account::NativeLogoutObservation, String>,
    > {
        let state = Arc::clone(&self.0);
        Box::pin(async move {
            let (connection, _) = state
                .machine_control
                .connection_protocol(&machine)
                .ok_or_else(|| "machine_offline".to_owned())?;
            let pending =
                state
                    .machine_control
                    .begin_native_logout(&connection, &machine, &request)?;
            pending.await
        })
    }
}

/// The Controller's view of the durable records, refreshed after every write
/// and by the reconciler. Synchronous gates read it; dispatch reloads it.
#[derive(Default)]
pub(in crate::server) struct Cache {
    controls: parking_lot::RwLock<HashMap<String, Control>>,
    turns: parking_lot::RwLock<HashMap<String, Reservation>>,
    fences: parking_lot::RwLock<Vec<Fence>>,
    /// Turns whose dispatch runs in this process: their `reserved` rows are
    /// not orphans of an earlier Controller.
    dispatching: parking_lot::Mutex<HashSet<String>>,
    /// Fence operations with a Machine command in flight.
    fencing: parking_lot::Mutex<HashSet<String>>,
    /// Serializes this process's state writers.
    writes: tokio::sync::Mutex<()>,
    loaded: AtomicBool,
}

impl Cache {
    fn control(&self, account: &str) -> Option<Control> {
        self.controls.read().get(account).cloned()
    }

    fn turn(&self, account: &str) -> Option<Reservation> {
        self.turns.read().get(account).cloned()
    }

    fn account_fences(&self, account: &str) -> Vec<Fence> {
        self.fences
            .read()
            .iter()
            .filter(|fence| fence.account_id == account)
            .cloned()
            .collect()
    }
}

fn wait(reason: &str) -> String {
    format!("{}{reason}", super::super::session_maintenance::WAIT_PREFIX)
}

/// Whether the Service admits `account` (`accounts.antigravity_admitted`).
pub(in crate::server) fn admitted(state: &AppState, account: &ProviderAccountId) -> bool {
    !account.is_default()
        && state
            .service_config
            .get(&crate::config::schema::ACCOUNT_ANTIGRAVITY_ADMITTED)
            .iter()
            .any(|id| id == account.as_str())
}

/// The kill switch, checked wherever admission is (login, session creation).
pub(in crate::server) fn blocked(state: &AppState, account: &ProviderAccountId) -> bool {
    state
        .extensions
        .account_controls
        .control(account.as_str())
        .is_some_and(|control| control.state == AccountState::Blocked)
}

fn governed(meta: &SessionMeta) -> bool {
    crate::provider_account::named_only(&meta.provider)
}

pub(in crate::server) fn uncertain_key(session: &str) -> String {
    format!(
        "{}{session}",
        crate::core::settings_keys::SESSION_UNCERTAIN_EFFECT_ACK_PREFIX
    )
}

/// The account controls bound to one Controller.
#[derive(Clone)]
pub(in crate::server) struct Controls {
    env: Arc<dyn Env>,
    store: Store,
    cache: Arc<Cache>,
}

/// The controls of `state`; `None` without a durable store.
pub(in crate::server) fn of(state: &Arc<AppState>) -> Option<Controls> {
    Some(Controls {
        env: Arc::new(AppEnv(Arc::clone(state))),
        store: state.store.clone()?,
        cache: Arc::clone(&state.extensions.account_controls),
    })
}

/// Why a governed session's queue stays parked now (cached view).
pub(in crate::server) fn parked_reason(
    state: &Arc<AppState>,
    meta: &SessionMeta,
) -> Option<String> {
    if !governed(meta) {
        return None;
    }
    match of(state) {
        Some(controls) => controls.parked_reason(meta),
        None => Some("account controls need the durable store".into()),
    }
}

/// Every worker revival of a governed session: admitted and `ok` (a rate
/// limit after its end counts as resumable).
pub(in crate::server) fn revival_gate(
    state: &Arc<AppState>,
    meta: &SessionMeta,
) -> Result<(), String> {
    if !governed(meta) {
        return Ok(());
    }
    of(state)
        .ok_or_else(|| wait("account controls need the durable store"))?
        .revival_gate(meta)
}

/// The account gate of one dispatch (see [`Controls::before_dispatch`]).
pub(in crate::server) async fn before_dispatch(
    state: &Arc<AppState>,
    session: &str,
    cmid: Option<&str>,
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
        .before_dispatch(session, cmid, explicit)
        .await
}

/// Report a dispatch's prompt as never enqueued (see [`Turn::unsent`]).
pub(in crate::server) fn unsent(turn: &mut Option<Turn>) {
    if let Some(turn) = turn.as_mut() {
        turn.unsent();
    }
}

/// A reserved turn of this process's dispatch. Dropping it releases the
/// reservation unless [`Turn::mark`] made it `may_have_dispatched` and the
/// prompt was not reported [`Turn::unsent`].
pub(in crate::server) struct Turn {
    controls: Controls,
    account: String,
    session: String,
    turn_id: String,
    marked: bool,
    unsent: bool,
}

impl Turn {
    /// Persist `may_have_dispatched` before the prompt is enqueued.
    pub(in crate::server) async fn mark(&mut self) -> Result<(), String> {
        let epoch = self
            .controls
            .env
            .worker(&self.session)
            .map(|worker| worker.worker_epoch)
            .unwrap_or_default();
        match self
            .controls
            .store
            .mark_account_turn_dispatched(&self.account, &self.turn_id, &epoch, now_ms())
            .await
        {
            Ok(true) => {
                self.marked = true;
                let _ = self.controls.refresh().await;
                Ok(())
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
        let release = if self.unsent {
            None
        } else if self.marked {
            self.controls.cache.dispatching.lock().remove(&self.turn_id);
            return;
        } else {
            Some(ReservationState::Reserved)
        };
        // The reconciler skips this turn until the release is durable.
        let controls = self.controls.clone();
        let (account, turn) = (self.account.clone(), self.turn_id.clone());
        tokio::spawn(async move {
            let _ = controls
                .store
                .release_account_turn(&account, &turn, release)
                .await;
            let _ = controls.refresh().await;
            controls.cache.dispatching.lock().remove(&turn);
        });
    }
}

impl Controls {
    #[cfg(test)]
    pub(in crate::server) fn new(env: Arc<dyn Env>, store: Store, cache: Arc<Cache>) -> Self {
        Self { env, store, cache }
    }

    /// Reload every record from the store.
    pub(in crate::server) async fn refresh(&self) -> Result<(), String> {
        let unavailable = |_| "account controls are unavailable".to_owned();
        let controls = self.store.account_controls().await.map_err(unavailable)?;
        let turns = self.store.account_turns().await.map_err(unavailable)?;
        let fences = self.store.account_fences().await.map_err(unavailable)?;
        *self.cache.controls.write() = controls
            .into_iter()
            .map(|control| (control.account_id.clone(), control))
            .collect();
        *self.cache.turns.write() = turns
            .into_iter()
            .map(|turn| (turn.account_id.clone(), turn))
            .collect();
        *self.cache.fences.write() = fences;
        self.cache.loaded.store(true, Ordering::Release);
        Ok(())
    }

    /// The session's unacknowledged uncertain effect: the live worker
    /// snapshot or the Controller's durable observation of it, beyond the
    /// owner's acknowledged sequence.
    pub(in crate::server) fn uncertain(&self, session: &str) -> Option<(String, u64)> {
        let record = self.env.setting(&uncertain_key(session));
        let field = |name: &str| {
            record
                .as_ref()
                .and_then(|value| value.get(name))
                .and_then(Value::as_u64)
        };
        let acknowledged = field("acknowledged").unwrap_or(0);
        let live = self
            .env
            .worker(session)
            .and_then(|worker| worker.uncertain_effect)
            .map(|effect| (effect.method, effect.sequence.max(1)));
        let stored = field("observed").map(|observed| {
            let method = record
                .as_ref()
                .and_then(|value| value["method"].as_str())
                .unwrap_or("request")
                .to_owned();
            (method, observed)
        });
        [live, stored]
            .into_iter()
            .flatten()
            .max_by_key(|(_, sequence)| *sequence)
            .filter(|(_, sequence)| *sequence > acknowledged)
    }

    fn gate(&self, meta: &SessionMeta, explicit: bool) -> Gate {
        let control = self.cache.control(meta.provider_account_id.as_str());
        let uncertain = self.uncertain(&meta.id);
        policy::gate(&GateInput {
            admitted: self.env.admitted(&meta.provider_account_id),
            control: control.as_ref(),
            uncertain: uncertain
                .as_ref()
                .map(|(method, sequence)| (method.as_str(), *sequence)),
            explicit,
            now_ms: now_ms(),
        })
    }

    /// Why a governed session's queue stays parked now, from the cache. The
    /// dispatcher rechecks against the store before anything is sent.
    pub(in crate::server) fn parked_reason(&self, meta: &SessionMeta) -> Option<String> {
        if !governed(meta) {
            return None;
        }
        if !self.cache.loaded.load(Ordering::Acquire) {
            return Some("account controls are loading".into());
        }
        if let Gate::Park(reason) = self.gate(meta, false) {
            return Some(reason);
        }
        self.cache
            .turn(meta.provider_account_id.as_str())
            .and_then(|turn| policy::slot_reason(&turn, &meta.id))
    }

    pub(in crate::server) fn revival_gate(&self, meta: &SessionMeta) -> Result<(), String> {
        if !governed(meta) {
            return Ok(());
        }
        if !self.cache.loaded.load(Ordering::Acquire) {
            return Err(wait("account controls are loading"));
        }
        let control = self.cache.control(meta.provider_account_id.as_str());
        policy::revival_allowed(
            self.env.admitted(&meta.provider_account_id),
            control.as_ref(),
            now_ms(),
        )
        .map_err(|reason| wait(&reason))
    }

    /// The account gate of one dispatch: admission, account state, uncertain
    /// effects and the account's single active turn. `Err` is the session's
    /// visible wait; the caller requeues the prompt unchanged.
    pub(in crate::server) async fn before_dispatch(
        &self,
        session: &str,
        cmid: Option<&str>,
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
        let turn_id = cmid.map_or_else(|| format!("turn-{}", uuid::Uuid::new_v4()), str::to_owned);
        let now = now_ms();
        // Registered before the row exists, so the reconciler never takes
        // this reservation for an orphan of an earlier Controller.
        self.cache.dispatching.lock().insert(turn_id.clone());
        let wanted = Reservation {
            account_id: account.clone(),
            owner_user_id: meta.owner_user_id.clone().unwrap_or_default(),
            session_id: session.to_owned(),
            turn_id: turn_id.clone(),
            worker_epoch: String::new(),
            after_seq: i64::try_from(self.env.last_event_seq(session)).unwrap_or(i64::MAX),
            state: ReservationState::Reserved,
            created_at_ms: now,
            updated_at_ms: now,
        };
        let holder = self.store.reserve_account_turn(&wanted).await;
        let _ = self.refresh().await;
        let refused = match &holder {
            Ok(holder)
                if holder.turn_id == turn_id && holder.state == ReservationState::Reserved =>
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
            marked: false,
            unsent: false,
        }))
    }

    /// The Machines that may run units of `account`: its native generations
    /// and the runtime Machines of its sessions.
    async fn account_machines(&self, account: &str) -> Vec<String> {
        let mut machines: BTreeSet<String> = self
            .store
            .account_native_machines(account)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect();
        machines.extend(
            self.env
                .sessions()
                .into_iter()
                .filter(|meta| governed(meta) && meta.provider_account_id.as_str() == account)
                .map(|meta| meta.machine_id),
        );
        machines.remove("local");
        machines.into_iter().collect()
    }

    /// Move `control`'s account to `next`. Entering a unit-stopping state (or
    /// `force_stop`) queues a stop fence on every Machine of the account in
    /// the same transaction; returning to `ok` after a stop queues an admit
    /// fence. The state is durable before any Machine is asked anything.
    pub(in crate::server) async fn enter(
        &self,
        control: &Control,
        next: AccountState,
        reason: &str,
        until_ms: Option<i64>,
        expected_revision: Option<i64>,
        force_stop: bool,
    ) -> Result<Control, &'static str> {
        let write = self.cache.writes.lock().await;
        let stopped = self
            .cache
            .account_fences(&control.account_id)
            .iter()
            .any(|fence| fence.action == AccountFenceAction::Stop);
        let action = if next.stops_units() || force_stop {
            Some(AccountFenceAction::Stop)
        } else if next == AccountState::Ok && stopped {
            Some(AccountFenceAction::Admit)
        } else {
            None
        };
        let machines = match action {
            Some(_) => self.account_machines(&control.account_id).await,
            None => Vec::new(),
        };
        let written = self
            .store
            .set_account_state(&StateChange {
                account_id: &control.account_id,
                owner_user_id: &control.owner_user_id,
                provider_id: &control.provider_id,
                state: next,
                reason,
                until_ms,
                expected_revision,
                fence: action.map(|action| (action, machines.as_slice())),
                now_ms: now_ms(),
            })
            .await
            .map_err(|_| "outcome_unknown")?
            .ok_or("revision_conflict")?;
        drop(write);
        let _ = self.refresh().await;
        tracing::info!(
            account = %control.account_id,
            state = next.as_str(),
            reason,
            "account control state changed"
        );
        self.run_fences();
        Ok(written)
    }

    /// Apply a settlement's automatic state change: never lowers a more
    /// severe state and never replaces the owner's block.
    async fn settle_state(
        &self,
        template: &Control,
        next: AccountState,
        reason: &str,
        stop_units: bool,
    ) {
        let current = self.cache.control(&template.account_id);
        let current_state = current.as_ref().map_or(AccountState::Ok, |c| c.state);
        let raise =
            next.severity() >= current_state.severity() && current_state != AccountState::Blocked;
        if !raise && !stop_units {
            return;
        }
        let kept = current
            .as_ref()
            .map_or_else(String::new, |c| c.reason.clone());
        let (next, reason) = if raise {
            (next, reason)
        } else {
            (current_state, kept.as_str())
        };
        let until = (next == AccountState::RateLimited)
            .then(|| now_ms().saturating_add(policy::DEFAULT_RATE_LIMIT_MS));
        let base = current.unwrap_or_else(|| template.clone());
        if let Err(error) = self
            .enter(&base, next, reason, until, None, stop_units)
            .await
        {
            tracing::warn!(account = %template.account_id, error, "account settlement state write failed");
        }
    }

    /// Every unit of the account is confirmed stopped on every Machine.
    pub(in crate::server) fn stop_confirmed(&self, account: &str) -> bool {
        let fences = self.cache.account_fences(account);
        !fences.is_empty()
            && fences.iter().all(|fence| {
                fence.action == AccountFenceAction::Stop && fence.state == FenceState::Confirmed
            })
    }

    /// The account controls of the owner's named-only accounts, for the
    /// provider-accounts read model.
    pub(in crate::server) fn read_value(&self, accounts: &[(ProviderAccountId, String)]) -> Value {
        let sessions = self.env.sessions();
        let accounts = accounts
            .iter()
            .filter(|(_, provider)| crate::provider_account::named_only(provider))
            .map(|(id, provider)| {
                let account = id.as_str();
                let control = self.cache.control(account);
                let uncertain: Vec<Value> = sessions
                    .iter()
                    .filter(|meta| governed(meta) && meta.provider_account_id == *id)
                    .filter_map(|meta| {
                        let (method, sequence) = self.uncertain(&meta.id)?;
                        Some(json!({"session_id":meta.id,"method":method,"sequence":sequence}))
                    })
                    .collect();
                json!({
                    "account_id": account,
                    "provider_id": provider,
                    "admitted": self.env.admitted(id),
                    "state": control.as_ref().map_or("ok", |c| c.state.as_str()),
                    "reason": control.as_ref().map(|c| c.reason.clone()),
                    "until_ms": control.as_ref().and_then(|c| c.until_ms),
                    "updated_at_ms": control.as_ref().map(|c| c.updated_at_ms),
                    "revision": control.as_ref().map_or(0, |c| c.revision),
                    "turn": self.cache.turn(account).map(|turn| json!({
                        "session_id": turn.session_id, "turn_id": turn.turn_id,
                        "state": turn.state.as_str(), "created_at_ms": turn.created_at_ms,
                    })),
                    "fences": self.cache.account_fences(account).iter().map(|fence| json!({
                        "machine_id": fence.machine_id, "action": fence.action,
                        "state": fence.state, "detail": fence.detail,
                        "operation_id": fence.operation_id, "updated_at_ms": fence.updated_at_ms,
                    })).collect::<Vec<_>>(),
                    "stop_confirmed": self.stop_confirmed(account),
                    "uncertain_effects": uncertain,
                    "logout": self.logout_value(account),
                })
            })
            .collect::<Vec<_>>();
        json!({"loaded": self.cache.loaded.load(Ordering::Acquire), "accounts": accounts})
    }
}

/// A control record for an account that has none yet.
fn initial(account: &str, owner: &str, provider: &str) -> Control {
    Control {
        account_id: account.to_owned(),
        owner_user_id: owner.to_owned(),
        provider_id: provider.to_owned(),
        state: AccountState::Ok,
        reason: String::new(),
        until_ms: None,
        updated_at_ms: 0,
        revision: 0,
    }
}

/// The `account_controls` member of the provider-accounts read model.
pub(in crate::server) fn read_value(
    state: &Arc<AppState>,
    registry: &crate::store::provider_accounts::Registry,
) -> Value {
    let accounts: Vec<(ProviderAccountId, String)> = registry
        .accounts
        .iter()
        .map(|account| (account.id.clone(), account.provider_id.clone()))
        .collect();
    of(state).map_or_else(
        || json!({"loaded": false, "accounts": []}),
        |controls| controls.read_value(&accounts),
    )
}

#[cfg(test)]
mod tests;
