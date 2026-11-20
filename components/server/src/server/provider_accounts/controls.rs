//! Account controls of named-only Providers (Antigravity) on the Controller
//! (`docs/antigravity-real-accounts-design.md`, "Account controls", "Kill
//! switch and logout" and "Revision 5 simplification"). The policy lives in
//! `crate::account_controls`; the durable records in
//! `crate::store::account_controls`. This module applies them at every
//! boundary:
//!
//! - admission (`accounts.antigravity_admitted`) and the kill switch at
//!   login start and completion, session creation ([`super::named_only_admitted`]),
//!   every prompt dispatch ([`Controls::before_dispatch`]) and every worker
//!   launch or revival ([`revival_gate`]);
//! - concurrent sessions per account ("Revision 6"): every session runs its
//!   own daemon unit over the shared account home, so nothing here gates a
//!   session on another session of the same account;
//! - backend signals: a governed session's turn ending moves its account to
//!   the state the proxy's structured result names; nothing waits for or is
//!   settled by a turn ending;
//! - Machine fences that stop or admit the account's units.
//!
//! Prompts never leave the durable queue while parked: a refused dispatch
//! requeues its prompt with the reason as the session's visible wait.
use super::*;
use crate::account_controls::{
    self as policy, AccountState, Control, Fence, FenceState, Gate, GateInput,
};
use crate::core::SessionMeta;
use crate::provider_account::{
    AccountFenceAction, AccountFenceObservation, AccountFenceRequest, ProviderAccountId,
};
use crate::store::account_controls::StateChange;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

mod dispatch;
pub(in crate::server) mod http;
mod logout;
mod plan;
mod reconcile;
pub(in crate::server) use dispatch::{before_dispatch, handoff};
pub(in crate::server) use reconcile::{start, turn_end_listener};

/// What the controls observe of the rest of the Controller. [`AppEnv`] is
/// the production binding; tests substitute their own.
pub(in crate::server) trait Env: Send + Sync + 'static {
    /// A session's metadata and whether a prompt of it is in flight.
    fn session(&self, id: &str) -> Option<(SessionMeta, bool)>;
    fn sessions(&self) -> Vec<SessionMeta>;
    fn worker(&self, session: &str) -> Option<crate::runtime_wire::WorkerSnapshot>;
    fn machine_connected(&self, machine: &str) -> bool;
    fn admitted(&self, account: &ProviderAccountId) -> bool;
    fn setting(&self, key: &str) -> Option<Value>;
    /// Publish a setting the caller already wrote durably (in memory only:
    /// every writer persists first, under the controls' writer lock).
    fn set_setting(&self, key: String, value: Value);
    fn fence(
        &self,
        machine: String,
        request: AccountFenceRequest,
    ) -> futures::future::BoxFuture<'static, Result<AccountFenceObservation, String>>;
    /// Run one native logout on `machine` (the Machine's existing native
    /// logout command; sandboxed accounts use the daemon's logout unit), only
    /// while the Machine's enrollment is the one that holds the credential
    /// (`recipient_fingerprint`), so a re-enrolled Machine never certifies it.
    fn native_logout(
        &self,
        machine: String,
        recipient_fingerprint: String,
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
        self.0.hub.load_settings(vec![(key, value)]);
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
        recipient_fingerprint: String,
        request: crate::provider_account::NativeLogoutRequest,
    ) -> futures::future::BoxFuture<
        'static,
        Result<crate::provider_account::NativeLogoutObservation, String>,
    > {
        let state = Arc::clone(&self.0);
        Box::pin(async move {
            // Every refusal before the command is enqueued says so
            // (`logout::UNSENT`): an unblock may still cancel the target.
            let unsent = |detail: &str| format!("{}{detail}", logout::UNSENT);
            let store = state.store.as_ref().ok_or_else(|| unsent("unavailable"))?;
            // The connection is captured before the enrollment is checked: a
            // re-enrollment after this point replaces the captured
            // connection (refused below), and one before it changes the key.
            let (connection, _) = state
                .machine_control
                .connection_protocol(&machine)
                .ok_or_else(|| unsent("machine_offline"))?;
            let key = store
                .machine_encryption_public_key(&machine)
                .await
                .map_err(|_| unsent("unavailable"))?
                .ok_or_else(|| unsent("machine_unavailable"))?;
            if crate::admin::hex_sha256(key.as_bytes()) != recipient_fingerprint
                || !state.machine_control.is_current(&connection)
            {
                return Err(unsent("machine_enrollment_changed"));
            }
            let pending = state
                .machine_control
                .begin_native_logout(&connection, &machine, &request)
                .map_err(|detail| unsent(&detail))?;
            let observation = pending.await?;
            // Evidence from a connection replaced meanwhile (a re-enrolled
            // Machine) is never recorded.
            if !state.machine_control.is_current(&connection) {
                return Err("machine_connection_replaced".to_owned());
            }
            Ok(observation)
        })
    }
}

/// The Controller's view of the durable records, refreshed after every write
/// and by the reconciler. Synchronous gates read it; dispatch reloads it.
#[derive(Default)]
pub(in crate::server) struct Cache {
    controls: parking_lot::RwLock<HashMap<String, Control>>,
    fences: parking_lot::RwLock<Vec<Fence>>,
    /// Fence operations with a Machine command in flight.
    fencing: parking_lot::Mutex<HashSet<String>>,
    /// Backend signals of each account observed but not yet committed: the
    /// account's dispatches and revivals wait for them (fail closed).
    pending_signals: parking_lot::Mutex<HashMap<String, usize>>,
    /// Serializes this process's control writers (state, admission, logout)
    /// with every dispatch's eligibility check and handoff.
    writes: Arc<tokio::sync::Mutex<()>>,
    /// Refresh ordering: a load applies only when no load that started
    /// later already applied, so an older snapshot never replaces a newer
    /// one (a block is never reopened by a slow concurrent refresh).
    loads_started: AtomicU64,
    load_applied: parking_lot::Mutex<u64>,
    loaded: AtomicBool,
}

impl Cache {
    fn control(&self, account: &str) -> Option<Control> {
        self.controls.read().get(account).cloned()
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

const SIGNAL_PENDING: &str = "a backend signal of this account is being recorded";

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

/// The kill switch, checked wherever admission is (login, session creation),
/// read from the store: an unloaded or stale cache is never permission.
pub(in crate::server) async fn blocked(
    state: &AppState,
    account: &ProviderAccountId,
) -> Result<bool, &'static str> {
    state
        .store
        .as_ref()
        .ok_or("unavailable")?
        .account_control(account.as_str())
        .await
        .map(|control| control.is_some_and(|control| control.state == AccountState::Blocked))
        .map_err(|_| "unavailable")
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

/// The final, synchronous recheck of a governed session's dispatch at the
/// prompt handoff (after any asynchronous recovery since
/// [`before_dispatch`]): the session's visible wait when it must park now.
pub(in crate::server) fn handoff_refusal(state: &Arc<AppState>, session: &str) -> Option<String> {
    let meta = state.hub.session_info(session)?.meta;
    parked_reason(state, &meta).map(|reason| wait(&reason))
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

impl Controls {
    #[cfg(test)]
    pub(in crate::server) fn new(env: Arc<dyn Env>, store: Store, cache: Arc<Cache>) -> Self {
        Self { env, store, cache }
    }

    /// Reload every record from the store. A failed reload marks the view
    /// unloaded, so no synchronous gate admits from a stale snapshot. Loads
    /// apply in the order they started, and a control record never moves
    /// back to a lower revision.
    pub(in crate::server) async fn refresh(&self) -> Result<(), String> {
        let unavailable = |_| "account controls are unavailable".to_owned();
        let started = self.cache.loads_started.fetch_add(1, Ordering::AcqRel) + 1;
        let loaded = async {
            Ok::<_, String>((
                self.store.account_controls().await.map_err(unavailable)?,
                self.store.account_fences().await.map_err(unavailable)?,
            ))
        }
        .await;
        self.apply(started, loaded)
    }

    /// Apply one load in start order (see [`Self::refresh`]).
    fn apply(
        &self,
        started: u64,
        loaded: Result<(Vec<Control>, Vec<Fence>), String>,
    ) -> Result<(), String> {
        let mut applied = self.cache.load_applied.lock();
        if *applied > started {
            // A load that started later already applied a newer snapshot.
            return loaded.map(drop);
        }
        // A failed load also advances the watermark: no load that started
        // before it may restore availability, only a later successful one
        // (it may have missed a write the failed one was meant to see).
        *applied = started;
        let (controls, fences) = match loaded {
            Ok(loaded) => loaded,
            Err(error) => {
                self.cache.loaded.store(false, Ordering::Release);
                return Err(error);
            }
        };
        let mut cached = self.cache.controls.write();
        let mut next: HashMap<String, Control> = controls
            .into_iter()
            .map(|control| (control.account_id.clone(), control))
            .collect();
        for (account, current) in cached.iter() {
            if let Some(loaded) = next.get(account)
                && loaded.revision < current.revision
            {
                next.insert(account.clone(), current.clone());
            }
        }
        *cached = next;
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

    /// Whether a backend signal of `account` is still being recorded.
    fn signal_pending(&self, account: &ProviderAccountId) -> bool {
        self.cache
            .pending_signals
            .lock()
            .get(account.as_str())
            .is_some_and(|pending| *pending > 0)
    }

    fn gate(&self, meta: &SessionMeta, explicit: bool) -> Gate {
        if self.signal_pending(&meta.provider_account_id) {
            return Gate::Park(SIGNAL_PENDING.into());
        }
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
        match self.gate(meta, false) {
            Gate::Park(reason) => Some(reason),
            Gate::Proceed { .. } => None,
        }
    }

    pub(in crate::server) fn revival_gate(&self, meta: &SessionMeta) -> Result<(), String> {
        if !governed(meta) {
            return Ok(());
        }
        if !self.cache.loaded.load(Ordering::Acquire) {
            return Err(wait("account controls are loading"));
        }
        if self.signal_pending(&meta.provider_account_id) {
            return Err(wait(SIGNAL_PENDING));
        }
        let control = self.cache.control(meta.provider_account_id.as_str());
        policy::revival_allowed(
            self.env.admitted(&meta.provider_account_id),
            control.as_ref(),
            now_ms(),
        )
        .map_err(|reason| wait(&reason))
    }

    /// The Machines that may run units of `account`: those holding its
    /// sandbox (its native generations' credential Machines) and its
    /// sessions' runtime Machines. Execution targets never run account units
    /// and are not fenced. A failed lookup is an error, never an empty
    /// (unfenced) set.
    async fn account_machines(&self, account: &str) -> Result<Vec<String>, &'static str> {
        let mut machines: BTreeSet<String> = self
            .store
            .account_native_machines(account)
            .await
            .map_err(|_| "unavailable")?
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
        Ok(machines.into_iter().collect())
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
            Some(_) => self.account_machines(&control.account_id).await?,
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
        // Lifting the kill switch cancels every logout target the Machines
        // were never asked, still under the writer lock (logout delivery
        // decides under it too), so no later block can fire it. Only a
        // written unblock does: a refused one leaves a valid logout intact.
        if control.state == AccountState::Blocked && next != AccountState::Blocked {
            self.cancel_undispatched_logouts(&control.account_id)
                .await?;
        }
        // Refreshed before the lock is released: every reader serialized
        // behind it sees this write.
        let _ = self.refresh().await;
        drop(write);
        tracing::info!(
            account = %control.account_id,
            state = next.as_str(),
            reason,
            "account control state changed"
        );
        self.run_fences();
        Ok(written)
    }

    /// Apply one backend signal of `account` atomically: the state change
    /// (compared with the stored state in the same transaction, so it never
    /// lowers a more severe state or replaces the owner's block) and its stop
    /// fences. A failure writes nothing.
    pub(in crate::server) async fn raise(
        &self,
        account: &str,
        owner: &str,
        provider: &str,
        transition: &policy::Transition,
    ) -> Result<bool, &'static str> {
        let until = (transition.state == AccountState::RateLimited)
            .then(|| now_ms().saturating_add(policy::DEFAULT_RATE_LIMIT_MS));
        let write = self.cache.writes.lock().await;
        let machines = if transition.stop_units {
            self.account_machines(account).await?
        } else {
            Vec::new()
        };
        let written = self
            .store
            .raise_account_state(&crate::store::account_controls::Raise {
                account_id: account,
                owner_user_id: owner,
                provider_id: provider,
                state: (transition.state, &transition.reason, until),
                stop_units: transition.stop_units,
                machines: &machines,
                now_ms: now_ms(),
            })
            .await
            .map_err(|_| "outcome_unknown")?;
        let _ = self.refresh().await;
        drop(write);
        if written {
            tracing::info!(
                account,
                state = transition.state.as_str(),
                reason = transition.reason,
                "account control state raised by a backend signal"
            );
        }
        self.run_fences();
        Ok(written)
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
                    "fences": self.cache.account_fences(account).iter().map(|fence| json!({
                        "machine_id": fence.machine_id, "action": fence.action,
                        "state": fence.state, "detail": fence.detail,
                        "operation_id": fence.operation_id, "updated_at_ms": fence.updated_at_ms,
                    })).collect::<Vec<_>>(),
                    "stop_confirmed": self.stop_confirmed(account),
                    "uncertain_effects": uncertain,
                    "logout": self.logout_value(account),
                    "plan": plan::account_plan(&sessions, id),
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
