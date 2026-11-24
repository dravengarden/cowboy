//! Owner-scheduled banked resets of named accounts. A schedule is a one-shot,
//! persisted intent to spend the account's nearest reset at a chosen time,
//! through exactly the admission and dispatch of a confirmed reset.
//!
//! - It is bound to the exact sign-in it was created under: the verified
//!   identity, the account's admission revision and the reset route's own
//!   binding (the native login, or the portable sign-in generation). The
//!   timer compares it under the account lock after admission, and the
//!   dispatch rechecks that binding, so any new sign-in, even to the same
//!   identity, withdraws it unspent; a token rotation does not. So does a
//!   disabled or revoking account, a disabled owner, or a fire more than
//!   [`OVERDUE_MS`] late (a Controller that was down or rolled back).
//! - The timer admits first (route, account lock, Provider starts). A short
//!   pacing wait only delays it; a long one (an account backoff) releases
//!   the Provider hold and counts like any other refusal, which sent nothing
//!   and is retried a minute later, [`PREFLIGHT_ATTEMPTS`] times in all.
//! - It spends the nearest available grant of the admitted binding's
//!   successful observation, at most [`FRESH_MS`] old (refreshed through the
//!   ordinary budgeted read first when older than [`REFRESH_AFTER_MS`]).
//! - Only then is the row claimed (deleted under its key), before the audit
//!   row and the dispatch: it fires at most once, even across a restart.
//!   After the claim it is never retried. Until the claim an admitted start
//!   that is dropped (a shutdown) is released unused.
use super::*;
use crate::store::AccountResetSchedule;

/// The earliest a schedule may fire after it is set.
const MIN_LEAD_MS: i64 = 60_000;
/// The latest a schedule may be set for.
const MAX_LEAD_MS: i64 = 366 * 24 * 60 * 60 * 1_000;
/// A due schedule later than this is withdrawn unspent.
const OVERDUE_MS: i64 = 60 * 60 * 1_000;
/// Tries of a due schedule whose admission was refused (not briefly paced).
const PREFLIGHT_ATTEMPTS: i32 = 3;
/// The wait after such a refusal, or for a fresh observation.
const PREFLIGHT_RETRY_MS: i64 = 60_000;
/// The longest pacing wait for which a schedule keeps the Provider held.
const PACED_HOLD_MS: i64 = 2 * 60 * 1_000;
/// An observation older than this is refreshed before admission.
const REFRESH_AFTER_MS: i64 = 2 * 60 * 1_000;
/// The oldest observation a schedule picks its grant from.
const FRESH_MS: i64 = 10 * 60 * 1_000;
/// How often the timer looks for due schedules, and at most how many per look.
const TICK: std::time::Duration = std::time::Duration::from_secs(15);
const BATCH: i64 = 32;

#[derive(Deserialize)]
pub struct ScheduleRequest {
    pub fire_at_ms: i64,
    pub confirm: String,
}

/// The exact sign-in a schedule is bound to: the owner, the account's
/// verified identity and admission revision, and the reset route's binding.
/// None without a verified identity, which no reset may spend from.
fn authority(
    owner: &str,
    account: &crate::store::provider_accounts::Account,
    binding: &Scope,
) -> Option<String> {
    let identity = account.identity_fingerprint.as_deref()?;
    let bound = (
        owner,
        account.id.as_str(),
        identity,
        account.admission_revision,
        binding,
    );
    let bytes = serde_json::to_vec(&bound).ok()?;
    Some(format!("sha256:{}", crate::admin::hex_sha256(&bytes)))
}

/// The owner's account, if it is available for a reset.
async fn available_account(
    store: &crate::store::Store,
    owner: &str,
    id: &ProviderAccountId,
) -> Result<crate::store::provider_accounts::Account, &'static str> {
    let registry = store
        .provider_accounts(owner)
        .await
        .map_err(|_| "unavailable")?;
    registry
        .accounts
        .into_iter()
        .find(|account| &account.id == id)
        .filter(crate::store::provider_accounts::Account::available)
        .ok_or("account_unavailable")
}

/// Whether the owner may still act: an enabled user, or the local principal
/// while product authentication is off. Err only for a store failure.
async fn owner_enabled(state: &AppState, owner: &str) -> Result<bool, ()> {
    if !state.product_auth_enabled && owner == "local" {
        return Ok(true);
    }
    let store = state.store.as_ref().ok_or(())?;
    match store.user_by_id(owner).await {
        Ok(user) => Ok(user.is_some_and(|user| user.disabled_at_ms.is_none())),
        Err(_) => Err(()),
    }
}

/// Whether the usage admission ledger failed closed for this route.
fn admission_stopped(state: &AppState, owner: &str, id: &ProviderAccountId, route: &Route) -> bool {
    let key = crate::usage::limiter::Limiter::account_key(owner, id.as_str());
    state
        .usage
        .limiter
        .view(&route.binding.account, &key, now_ms())
        .next_allowed_ms
        == i64::MAX
}

/// The grant a reset would spend in this observation: the collector's
/// canonical nearest ID when it is among the available credits, otherwise
/// the earliest expiry, then grant time, then ID (as the Web dialog picks).
fn nearest_credit(usage: &Value) -> Option<String> {
    let root = usage.pointer("/rate_limits/rateLimitResetCredits")?;
    let available: Vec<&Value> = root
        .get("credits")?
        .as_array()?
        .iter()
        .filter(|credit| {
            credit["status"] == "available" && credit["id"].as_str().is_some_and(valid_credit_id)
        })
        .collect();
    let id = |credit: &&Value| credit["id"].as_str().unwrap_or_default().to_owned();
    if let Some(nearest) = root["nearestCreditId"].as_str()
        && let Some(credit) = available.iter().find(|credit| credit["id"] == nearest)
    {
        return Some(id(credit));
    }
    let order = |value: &Value| value.as_f64().unwrap_or(f64::MAX);
    available
        .into_iter()
        .min_by(|left, right| {
            order(&left["expiresAt"])
                .total_cmp(&order(&right["expiresAt"]))
                .then(order(&left["grantedAt"]).total_cmp(&order(&right["grantedAt"])))
                .then_with(|| id(left).cmp(&id(right)))
        })
        .map(|credit| id(&credit))
}

/// `PUT /api/provider-accounts/{account_id}/usage/reset/schedule`: spend the
/// account's nearest reset once at `fire_at_ms` (typed `confirm`, one minute
/// to one year ahead). Only an account whose reset could be spent now may be
/// scheduled; it replaces the account's earlier schedule.
pub async fn http_schedule(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(id): Path<ProviderAccountId>,
    Json(request): Json<ScheduleRequest>,
) -> Response {
    let owner = auth.principal.user_id.as_str();
    if request.confirm != "confirm" {
        return refusal("reset_confirmation_required");
    }
    let now = now_ms();
    if request.fire_at_ms < now.saturating_add(MIN_LEAD_MS)
        || request.fire_at_ms > now.saturating_add(MAX_LEAD_MS)
    {
        return refusal("reset_schedule_time_invalid");
    }
    let Some(store) = state.store.as_ref() else {
        return refusal("unavailable");
    };
    let account = match available_account(store, owner, &id).await {
        Ok(account) => account,
        Err(code) => return refusal(code),
    };
    // The same eligibility a reset now has (route, release, protocol,
    // admission ledger), bound to exactly the route's sign-in.
    let route = match route(&state, owner, &id).await {
        Ok(route) => route,
        Err(code) => return refusal(code),
    };
    if admission_stopped(&state, owner, &id, &route) {
        return refusal("usage_admission_unavailable");
    }
    let Some(authority) = authority(owner, &account, &route.scope()) else {
        return refusal("portable_reset_identity_required");
    };
    let key = new_reset_idempotency_key();
    if store
        .upsert_account_reset_schedule(owner, id.as_str(), request.fire_at_ms, &key, &authority)
        .await
        .is_err()
    {
        return refusal("unavailable");
    }
    let lane = lane(&id);
    log(
        store,
        (lane.as_str(), key.as_str(), Trigger::Scheduled),
        ("scheduled", "timer"),
        "Reset scheduled",
        None,
    )
    .await;
    state.hub.broadcast_account_usage_changed(owner);
    Json(json!({"reset_schedule":{"fire_at_ms":request.fire_at_ms}})).into_response()
}

/// `DELETE /api/provider-accounts/{account_id}/usage/reset/schedule`: cancel
/// the account's schedule. Never stops a reset the timer already claimed.
pub async fn http_cancel(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(id): Path<ProviderAccountId>,
) -> Response {
    let owner = auth.principal.user_id.as_str();
    let Some(store) = state.store.as_ref() else {
        return refusal("unavailable");
    };
    if let Err(code) = owned(store, owner, &id).await {
        return refusal(code);
    }
    match store
        .delete_account_reset_schedule(owner, id.as_str())
        .await
    {
        // The audit names exactly the row this request removed.
        Ok(Some(key)) => {
            let lane = lane(&id);
            log(
                store,
                (lane.as_str(), key.as_str(), Trigger::Scheduled),
                ("cancelled", "timer"),
                "Scheduled reset cancelled",
                None,
            )
            .await;
            state.hub.broadcast_account_usage_changed(owner);
            StatusCode::NO_CONTENT.into_response()
        }
        Ok(None) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => refusal("unavailable"),
    }
}

/// Fire due schedules until the server stops. Schedules live only in the
/// store, so a restart resumes them and each fires at most once. A claimed
/// spend runs in its own task, so a slow one never delays other accounts.
pub async fn run(state: Arc<AppState>) {
    loop {
        if let Some(store) = state.store.as_ref()
            && let Ok(due) = store.due_account_reset_schedules(now_ms(), BATCH).await
        {
            for schedule in due {
                fire(&state, store, &schedule).await;
            }
        }
        tokio::time::sleep(TICK).await;
    }
}

/// An admitted reset not yet claimed: dropped (an early return, a shutdown)
/// it releases its Provider starts unused, since nothing was sent.
struct Unclaimed(Option<Admitted>);

impl Unclaimed {
    fn route(&self) -> &Route {
        &self.0.as_ref().expect("admitted until claimed").route
    }

    /// Hand the admission to the spend once the schedule is claimed.
    fn claimed(mut self) -> Admitted {
        self.0.take().expect("admitted until claimed")
    }
}

impl Drop for Unclaimed {
    fn drop(&mut self) {
        if let Some(admitted) = self.0.take() {
            admitted
                .permit
                .finish(crate::usage::limiter::Outcome::Unused, now_ms());
        }
    }
}

/// One due schedule and the account it names.
struct Due<'a> {
    state: &'a Arc<AppState>,
    store: &'a crate::store::Store,
    schedule: &'a AccountResetSchedule,
    id: ProviderAccountId,
    lane: String,
}

impl Due<'_> {
    fn owner(&self) -> &str {
        &self.schedule.owner_id
    }

    fn key(&self) -> &str {
        &self.schedule.idempotency_key
    }

    fn overdue(&self) -> bool {
        now_ms().saturating_sub(self.schedule.fire_at_ms) > OVERDUE_MS
    }

    async fn log(&self, phase: (&str, &str), message: &str) {
        let row = (self.lane.as_str(), self.key(), Trigger::Scheduled);
        log(self.store, row, phase, message, None).await;
    }

    /// Remove exactly this schedule; true only for the caller that did.
    async fn claim(&self) -> bool {
        let claimed = self
            .store
            .claim_account_reset_schedule(self.owner(), self.id.as_str(), self.key())
            .await
            .unwrap_or(false);
        if claimed {
            self.state.hub.broadcast_account_usage_changed(self.owner());
        }
        claimed
    }

    /// Withdraw it unspent, with the reason.
    async fn withdraw(&self, message: &str) {
        if self.claim().await {
            self.log(("cancelled", "preflight"), message).await;
        }
    }

    /// Move its next attempt; a counted one is a refusal that sent nothing.
    async fn defer(&self, at_ms: i64, counted: bool) -> bool {
        self.store
            .defer_account_reset_schedule(
                self.owner(),
                self.id.as_str(),
                self.key(),
                at_ms,
                counted,
            )
            .await
            .unwrap_or(false)
    }

    /// An admission refusal sent nothing: retry a minute later, a bounded
    /// number of times, then give up.
    async fn refused(&self, code: &'static str) {
        if self.schedule.attempt_count.saturating_add(1) >= PREFLIGHT_ATTEMPTS {
            if self.claim().await {
                self.log(("failed", "preflight"), error_message(code)).await;
            }
        } else if self
            .defer(now_ms().saturating_add(PREFLIGHT_RETRY_MS), true)
            .await
        {
            self.log(("retrying", "preflight"), error_message(code))
                .await;
        }
    }

    /// When the account's current usage observation last succeeded.
    async fn observed_at(&self) -> Option<i64> {
        let resolved = target(self.state, self.owner(), &self.id).await.ok()?;
        let slot = self
            .state
            .usage
            .native
            .entry(self.owner(), self.id.as_str());
        let entry = slot.data.lock().snapshot(&resolved.scope());
        entry.usage?["observed_at_ms"].as_i64()
    }

    /// The admitted binding's successful observation, if it is fresh.
    fn fresh_observation(&self, binding: &Scope) -> Option<Value> {
        let slot = self
            .state
            .usage
            .native
            .entry(self.owner(), self.id.as_str());
        let usage = slot.data.lock().snapshot(binding).usage?;
        let observed = usage["observed_at_ms"].as_i64()?;
        (now_ms().saturating_sub(observed) <= FRESH_MS).then_some(usage)
    }
}

/// One due schedule's next step: withdraw it, wait, or claim and spend it.
async fn fire(state: &Arc<AppState>, store: &crate::store::Store, schedule: &AccountResetSchedule) {
    let Ok(id) = schedule.account_id.parse::<ProviderAccountId>() else {
        // Not an account ID: nothing can ever spend it.
        let _ = store
            .claim_account_reset_schedule(
                &schedule.owner_id,
                &schedule.account_id,
                &schedule.idempotency_key,
            )
            .await;
        return;
    };
    let due = Due {
        state,
        store,
        schedule,
        lane: lane(&id),
        id,
    };
    let owner = due.owner();
    const GONE: &str =
        "The account's sign-in changed or it is no longer available; no reset was sent.";
    const LATE: &str = "The scheduled reset was more than an hour overdue; no reset was sent.";
    if due.overdue() {
        return due.withdraw(LATE).await;
    }
    // Cheap withdrawals first; the binding is compared after admission.
    match (
        owner_enabled(state, owner).await,
        available_account(store, owner, &due.id).await,
    ) {
        (Err(()), _) | (_, Err("unavailable")) => return,
        (Ok(true), Ok(_)) => {}
        _ => return due.withdraw(GONE).await,
    }
    // Pick the grant from a recent observation; the refresh is budgeted like
    // any read, and the admission below then waits for the Provider spacing.
    let observed_at = due.observed_at().await.unwrap_or(0);
    if now_ms().saturating_sub(observed_at) > REFRESH_AFTER_MS {
        let _ = refresh_one(state, owner, &due.id).await;
    }
    let admitted = match admit(state, owner, &due.id).await {
        Ok(admitted) => Unclaimed(Some(admitted)),
        Err(Refused {
            code: "usage_reset_paced",
            retry_at_ms: Some(at),
            held,
        }) => {
            if at.saturating_sub(now_ms()) <= PACED_HOLD_MS {
                // A short wait only delays it; the Provider stays held.
                due.defer(at, false).await;
            } else {
                // An account backoff: never hold other accounts' polls for it.
                if let Some(provider) = held {
                    let key = crate::usage::limiter::Limiter::account_key(owner, due.id.as_str());
                    state.usage.limiter.release_priority(&provider, &key);
                }
                due.refused("usage_reset_paced").await;
            }
            return;
        }
        Err(Refused { code, .. }) => return due.refused(code).await,
    };
    // Under the account lock: exactly the sign-in it was scheduled under,
    // which the dispatch then rechecks for this route's binding.
    let binding = admitted.route().scope();
    let account = match available_account(store, owner, &due.id).await {
        Ok(account) => account,
        Err("unavailable") => return,
        Err(_) => return due.withdraw(GONE).await,
    };
    if authority(owner, &account, &binding).as_deref() != Some(schedule.authority.as_str()) {
        return due.withdraw(GONE).await;
    }
    let Some(credit) = due
        .fresh_observation(&binding)
        .as_ref()
        .and_then(nearest_credit)
    else {
        drop(admitted);
        if due.fresh_observation(&binding).is_some() {
            return due.refused("reset_unavailable").await;
        }
        // No fresh observation yet: wait without counting (the cutoff bounds it).
        due.defer(now_ms().saturating_add(PREFLIGHT_RETRY_MS), false)
            .await;
        return;
    };
    // The last checks before the one-shot claim.
    if due.overdue() {
        drop(admitted);
        return due.withdraw(LATE).await;
    }
    match owner_enabled(state, owner).await {
        Ok(true) => {}
        Ok(false) => {
            drop(admitted);
            return due.withdraw(GONE).await;
        }
        Err(()) => return,
    }
    // Only the caller that removes the row spends it.
    if !due.claim().await {
        return;
    }
    let admitted = admitted.claimed();
    let (state, store) = (Arc::clone(state), store.clone());
    let (owner, id, key) = (owner.to_owned(), due.id.clone(), due.key().to_owned());
    // The claimed spend runs to completion in its own task.
    tokio::spawn(async move {
        let spent = spend(
            &state,
            &store,
            (&owner, &id),
            admitted,
            (&key, Trigger::Scheduled),
            &credit,
        )
        .await;
        if matches!(spent, Spent::NotAudited) {
            let lane = lane(&id);
            let row = (lane.as_str(), key.as_str(), Trigger::Scheduled);
            log(
                &store,
                row,
                ("failed", "preflight"),
                error_message("unavailable"),
                None,
            )
            .await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_nearest_credit_matches_the_dialog() {
        let usage = |credits: Value, nearest: Value| json!({"rate_limits":{"rateLimitResetCredits":{"credits":credits,"nearestCreditId":nearest}}});
        let credits = json!([
            {"id":"late","status":"available","expiresAt":300,"grantedAt":1},
            {"id":"used","status":"used","expiresAt":100},
            {"id":"soon-b","status":"available","expiresAt":200,"grantedAt":2},
            {"id":"soon-a","status":"available","expiresAt":200,"grantedAt":2},
            {"id":"../bad","status":"available","expiresAt":1},
        ]);
        // The canonical ID wins when it is available.
        assert_eq!(
            nearest_credit(&usage(credits.clone(), json!("late"))).as_deref(),
            Some("late")
        );
        // Otherwise (absent, used or unknown) the earliest expiry, then ID.
        for nearest in [Value::Null, json!("used"), json!("gone")] {
            assert_eq!(
                nearest_credit(&usage(credits.clone(), nearest)).as_deref(),
                Some("soon-a")
            );
        }
        assert_eq!(nearest_credit(&usage(json!([]), Value::Null)), None);
        assert_eq!(nearest_credit(&json!({})), None);
    }

    #[test]
    fn a_schedule_is_bound_to_the_exact_sign_in() {
        let account: crate::store::provider_accounts::Account = serde_json::from_value(json!({
            "id":"11111111-1111-4111-8111-111111111111","provider_id":"codex",
            "label":"Pika","enabled":true,"identity_fingerprint":"abc","admission_revision":4
        }))
        .unwrap();
        let portable = |sign_in_generation: Option<u64>| {
            Scope::Portable(crate::usage::native::PortableScope {
                portable_provider: "codex".into(),
                usage_provider: "openai".into(),
                identity_fingerprint: Some("abc".into()),
                admission_revision: Some(4),
                credential_generation: None,
                sign_in_generation,
            })
        };
        let binding = portable(Some(1));
        let bound = authority("owner", &account, &binding).unwrap();
        assert!(bound.starts_with("sha256:"));
        // The same inputs bind the same authority (a token rotation keeps it).
        assert_eq!(
            authority("owner", &account, &portable(Some(1))),
            Some(bound.clone())
        );
        assert_ne!(authority("other", &account, &binding), Some(bound.clone()));
        // A new sign-in to the same identity changes the binding.
        assert_ne!(
            authority("owner", &account, &portable(Some(2))),
            Some(bound.clone())
        );
        let mut signed_in_again = account.clone();
        signed_in_again.admission_revision = Some(5);
        assert_ne!(
            authority("owner", &signed_in_again, &binding),
            Some(bound.clone())
        );
        let mut unverified = account;
        unverified.identity_fingerprint = None;
        assert_eq!(authority("owner", &unverified, &binding), None);
    }
}
