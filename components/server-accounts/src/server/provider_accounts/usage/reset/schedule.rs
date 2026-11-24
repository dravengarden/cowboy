//! Owner-scheduled banked resets of named accounts. A schedule is a one-shot,
//! persisted intent to spend the account's nearest reset at a chosen time,
//! through exactly the admission and dispatch of a confirmed reset.
//!
//! - It is bound to the sign-in it was created under (the verified identity
//!   and the account's admission revision). A new sign-in, a disabled or
//!   revoked account, or a disabled owner withdraws it unspent.
//! - The timer admits first (route, account lock, Provider starts). A paced
//!   admission waits without counting; any other refusal sent nothing and is
//!   retried a minute later, at most [`PREFLIGHT_ATTEMPTS`] times in all.
//! - Only then is the row claimed (deleted under its key), before the audit
//!   row and the dispatch. A crash after the claim leaves the `started` row
//!   and never fires it again; an unknown outcome is never retried.
//! - The reset spent is the nearest available grant in a usage observation
//!   no older than [`FRESH_MS`], read just before admission.
//! - A schedule overdue by more than [`OVERDUE_MS`] (a Controller that was
//!   down or rolled back) is withdrawn unspent instead of firing late.
use super::*;
use crate::store::AccountResetSchedule;

/// The earliest a schedule may fire after it is set.
const MIN_LEAD_MS: i64 = 60_000;
/// The latest a schedule may be set for.
const MAX_LEAD_MS: i64 = 366 * 24 * 60 * 60 * 1_000;
/// A due schedule later than this is withdrawn unspent.
const OVERDUE_MS: i64 = 60 * 60 * 1_000;
/// Tries of a due schedule whose admission was refused (not paced).
const PREFLIGHT_ATTEMPTS: i32 = 3;
/// The wait after such a refusal.
const PREFLIGHT_RETRY_MS: i64 = 60_000;
/// How old the usage observation a schedule picks its grant from may be.
const FRESH_MS: i64 = 2 * 60 * 1_000;
/// How often the timer looks for due schedules, and at most how many per look.
const TICK: std::time::Duration = std::time::Duration::from_secs(15);
const BATCH: i64 = 32;

#[derive(Deserialize)]
pub struct ScheduleRequest {
    pub fire_at_ms: i64,
    pub confirm: String,
}

/// The sign-in a schedule of this account is bound to: its verified identity
/// and the registry revision of its last authority change. None without a
/// verified identity, which no reset may spend from.
fn authority(owner: &str, account: &crate::store::provider_accounts::Account) -> Option<String> {
    let identity = account.identity_fingerprint.as_deref()?;
    let bound = (
        owner,
        account.id.as_str(),
        identity,
        account.admission_revision,
    );
    let bytes = serde_json::to_vec(&bound).ok()?;
    Some(format!("sha256:{}", crate::admin::hex_sha256(&bytes)))
}

/// The owner's available account and its current schedule authority.
async fn current_authority(
    store: &crate::store::Store,
    owner: &str,
    id: &ProviderAccountId,
) -> Result<String, &'static str> {
    let registry = store
        .provider_accounts(owner)
        .await
        .map_err(|_| "unavailable")?;
    let account = registry
        .accounts
        .iter()
        .find(|account| &account.id == id)
        .ok_or("account_unavailable")?;
    if !account.available() {
        return Err("account_unavailable");
    }
    authority(owner, account).ok_or("portable_reset_identity_required")
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
/// account's nearest reset once at `fire_at_ms` (typed `confirm`, at least a
/// minute ahead). Only an account whose reset could be spent now may be
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
    let authority = match current_authority(store, owner, &id).await {
        Ok(authority) => authority,
        Err(code) => return refusal(code),
    };
    // The same eligibility a reset now has (route, release, protocol).
    if let Err(code) = route(&state, owner, &id).await {
        return refusal(code);
    }
    let key = new_reset_idempotency_key();
    if store
        .upsert_account_reset_schedule(owner, id.as_str(), request.fire_at_ms, &key, &authority)
        .await
        .is_err()
    {
        return refusal("unavailable");
    }
    let row = (lane(&id), key);
    log(
        store,
        (row.0.as_str(), row.1.as_str(), Trigger::Scheduled),
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
    let existing = match store.account_reset_schedule(owner, id.as_str()).await {
        Ok(existing) => existing,
        Err(_) => return refusal("unavailable"),
    };
    match store
        .delete_account_reset_schedule(owner, id.as_str())
        .await
    {
        Ok(true) => {
            let lane = lane(&id);
            let key = existing
                .map(|schedule| schedule.idempotency_key)
                .unwrap_or_default();
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
        Ok(false) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => refusal("unavailable"),
    }
}

/// Fire due schedules until the server stops. Schedules live only in the
/// store, so a restart resumes them and each fires at most once.
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

    async fn log(&self, phase: (&str, &str), message: &str, credit_id: Option<&str>) {
        let row = (self.lane.as_str(), self.key(), Trigger::Scheduled);
        log(self.store, row, phase, message, credit_id).await;
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
            self.log(("cancelled", "preflight"), message, None).await;
        }
    }

    /// An admission refusal sent nothing: retry a minute later, a bounded
    /// number of times, then give up.
    async fn refused(&self, code: &'static str) {
        if self.schedule.attempt_count.saturating_add(1) >= PREFLIGHT_ATTEMPTS {
            if self.claim().await {
                self.log(("failed", "preflight"), error_message(code), None)
                    .await;
            }
            return;
        }
        let next = now_ms().saturating_add(PREFLIGHT_RETRY_MS);
        if self
            .store
            .defer_account_reset_schedule(self.owner(), self.id.as_str(), self.key(), next, true)
            .await
            .unwrap_or(false)
        {
            self.log(("retrying", "preflight"), error_message(code), None)
                .await;
        }
    }

    /// The account's current usage observation and when it was attempted.
    async fn observation(&self) -> Option<(i64, Value)> {
        let resolved = target(self.state, self.owner(), &self.id).await.ok()?;
        let slot = self
            .state
            .usage
            .native
            .entry(self.owner(), self.id.as_str());
        let entry = slot.data.lock().snapshot(&resolved.scope());
        entry.usage.map(|usage| (entry.last_attempt_at_ms, usage))
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
    if now_ms().saturating_sub(schedule.fire_at_ms) > OVERDUE_MS {
        return due
            .withdraw("The scheduled reset was more than an hour overdue; no reset was sent.")
            .await;
    }
    let owner_enabled = match store.user_by_id(owner).await {
        Ok(user) => user.is_some_and(|user| user.disabled_at_ms.is_none()),
        // A transient store failure decides nothing; try again next tick.
        Err(_) => return,
    };
    match current_authority(store, owner, &due.id).await {
        Ok(current) if owner_enabled && current == schedule.authority => {}
        Err("unavailable") => return,
        _ => {
            return due
                .withdraw(
                    "The account's sign-in changed or it is no longer available; no reset was sent.",
                )
                .await;
        }
    }
    // Pick the grant from a recent observation; the refresh is budgeted like
    // any read, and the admission below then waits for the Provider spacing.
    let observed_at = due.observation().await.map_or(0, |(at, _)| at);
    if now_ms().saturating_sub(observed_at) > FRESH_MS {
        let _ = refresh_one(state, owner, &due.id).await;
    }
    let admitted = match admit(state, owner, &due.id).await {
        Ok(admitted) => admitted,
        Err(Refused {
            code: "usage_reset_paced",
            retry_at_ms: Some(at),
        }) => {
            // Pacing only delays it; the limiter holds the Provider meanwhile.
            let _ = store
                .defer_account_reset_schedule(owner, due.id.as_str(), due.key(), at, false)
                .await;
            return;
        }
        Err(Refused { code, .. }) => return due.refused(code).await,
    };
    let release = |admitted: Admitted| {
        admitted
            .permit
            .finish(crate::usage::limiter::Outcome::Unused, now_ms());
    };
    let Some(credit) = due
        .observation()
        .await
        .and_then(|(_, usage)| nearest_credit(&usage))
    else {
        release(admitted);
        return due.refused("reset_unavailable").await;
    };
    // The one-shot claim: only the caller that removes the row spends it.
    if !due.claim().await {
        return release(admitted);
    }
    let (state, store) = (Arc::clone(state), store.clone());
    let (owner, id, key) = (owner.to_owned(), due.id.clone(), due.key().to_owned());
    // The claimed spend runs to completion even if this tick is dropped.
    let task = tokio::spawn(async move {
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
    let _ = task.await;
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
    fn a_schedule_is_bound_to_the_verified_sign_in() {
        let account: crate::store::provider_accounts::Account = serde_json::from_value(json!({
            "id":"11111111-1111-4111-8111-111111111111","provider_id":"codex",
            "label":"Pika","enabled":true,"identity_fingerprint":"abc","admission_revision":4
        }))
        .unwrap();
        let bound = authority("owner", &account).unwrap();
        assert!(bound.starts_with("sha256:"));
        assert_ne!(authority("other", &account), Some(bound.clone()));
        let mut signed_in_again = account.clone();
        signed_in_again.admission_revision = Some(5);
        assert_ne!(authority("owner", &signed_in_again), Some(bound.clone()));
        let mut unverified = account;
        unverified.identity_fingerprint = None;
        assert_eq!(authority("owner", &unverified), None);
    }
}
