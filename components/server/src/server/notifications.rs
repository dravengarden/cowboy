//! Task notification delivery and acknowledgement
//! (docs/task-notifications.md). One task turns due intents into frozen
//! deliveries, then sends those deliveries with retries. Everything it needs to
//! resume is in the store; presence alone is process-local, and losing it
//! delivers rather than withholds.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::watch;

use super::{AppState, AuthenticatedProductRequest, session_is_visible};
use crate::config::schema::NOTIFICATIONS_DELIVERY;
use crate::store::Store;
use crate::store::task_notifications::{Delivery, Intent, StoredSubscription};
use crate::task_notifications::{APPROVAL_PREFIX, Category, PERMISSION_PREFIX};
use crate::web_push::{
    PushRequest, SubscriptionKeys, WebPushPreferences, WebPushSubscription, endpoint_sha256,
    push_topic, safe_session_id, validate_subscription,
};

const TICK: Duration = Duration::from_secs(1);
const BATCH: i64 = 200;
const MAX_ATTEMPTS: i64 = 5;
const FIRST_RETRY_MS: i64 = 5_000;
const LEASE_MS: i64 = 30_000;
const RETENTION_MS: i64 = 7 * 24 * 60 * 60 * 1_000;
const PRUNE_EVERY_TICKS: u64 = 3_600;
const MARKER_MAX_BYTES: usize = 64;

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Delivery counters, exported on `/metrics`. "Sent" is push-service
/// acceptance, never device presentation.
#[derive(Debug, Default)]
pub(super) struct Counters {
    sent: AtomicU64,
    retried: AtomicU64,
    failed: AtomicU64,
    deferred: AtomicU64,
    expired: AtomicU64,
}

pub(super) static COUNTERS: Counters = Counters {
    sent: AtomicU64::new(0),
    retried: AtomicU64::new(0),
    failed: AtomicU64::new(0),
    deferred: AtomicU64::new(0),
    expired: AtomicU64::new(0),
};

impl Counters {
    pub(super) fn prometheus(&self) -> String {
        [
            ("sent", &self.sent),
            ("retried", &self.retried),
            ("failed", &self.failed),
            ("deferred", &self.deferred),
            ("expired", &self.expired),
        ]
        .iter()
        .map(|(name, value)| {
            format!(
                "# TYPE cowboy_notification_{name}_total counter\ncowboy_notification_{name}_total {}\n",
                value.load(Ordering::Relaxed)
            )
        })
        .collect()
    }
}

/// Short-lived "this account is looking at that session" leases, per device.
#[derive(Debug, Default)]
pub(super) struct Presence {
    leases: Mutex<HashMap<(String, String), (String, i64)>>,
}

impl Presence {
    fn renew(&self, user: &str, device: &str, session: &str, now: i64) {
        let mut leases = self.leases.lock();
        leases.retain(|_, (_, until)| *until > now);
        leases.insert(
            (user.to_owned(), device.to_owned()),
            (session.to_owned(), now.saturating_add(LEASE_MS)),
        );
    }

    fn release(&self, user: &str, device: &str) {
        self.leases
            .lock()
            .remove(&(user.to_owned(), device.to_owned()));
    }

    /// The latest expiry of the account's live leases on a session.
    fn held_until(&self, user: &str, session: &str, now: i64) -> Option<i64> {
        self.leases
            .lock()
            .iter()
            .filter(|((owner, _), (leased, until))| {
                owner == user && leased == session && *until > now
            })
            .map(|(_, (_, until))| *until)
            .max()
    }
}

/// Whether the owner may still hear about this session and has not seen it.
async fn deliverable(state: &AppState, store: &Store, intent: &Intent) -> anyhow::Result<bool> {
    let Some(user) = store.user_by_id(&intent.owner_user_id).await? else {
        return Ok(false);
    };
    if user.disabled_at_ms.is_some()
        || !session_is_visible(
            &state.hub,
            &super::product_principal(&state.hub, &user),
            &intent.session_id,
        )
    {
        return Ok(false);
    }
    if !intent.marker.is_empty()
        && store
            .task_acknowledged(
                &intent.owner_user_id,
                &intent.session_id,
                &intent.destination,
                &intent.marker,
            )
            .await?
    {
        return Ok(false);
    }
    Ok(input_still_waits(state, intent))
}

fn input_still_waits(state: &AppState, intent: &Intent) -> bool {
    let key = &intent.dedupe_key;
    if let Some(rest) = key.strip_prefix(PERMISSION_PREFIX) {
        return rest
            .rsplit_once(':')
            .and_then(|(_, seq)| seq.parse().ok())
            .is_none_or(|seq| state.hub.permission_pending(&intent.session_id, seq));
    }
    if let Some(request) = key
        .strip_prefix(APPROVAL_PREFIX)
        .and_then(|rest| rest.strip_prefix(&format!("{}:", intent.session_id)))
    {
        return state
            .managed_calls
            .approval_waiting(&intent.session_id, request);
    }
    true
}

fn preferences(subscription: &StoredSubscription) -> Option<WebPushPreferences> {
    serde_json::from_str(&subscription.preferences).ok()
}

/// Turn due intents into deliveries with a frozen audience.
async fn expand(state: &AppState, store: &Store, now: i64) -> anyhow::Result<()> {
    for intent in store.due_intents(now, BATCH).await? {
        if intent.expires_at_ms <= now {
            COUNTERS.expired.fetch_add(1, Ordering::Relaxed);
            store.finish_intent(&intent.dedupe_key, "expired").await?;
            continue;
        }
        if !deliverable(state, store, &intent).await? {
            store.finish_intent(&intent.dedupe_key, "resolved").await?;
            continue;
        }
        // Input ignores presence: a request never depends on a row being noticed.
        if intent.category != Category::Input
            && let Some(until) =
                state
                    .notifications
                    .held_until(&intent.owner_user_id, &intent.session_id, now)
        {
            COUNTERS.deferred.fetch_add(1, Ordering::Relaxed);
            store.defer_intent(&intent.dedupe_key, until).await?;
            continue;
        }
        let audience: Vec<String> = store
            .web_push_subscriptions(&intent.owner_user_id)
            .await?
            .iter()
            .filter(|subscription| {
                preferences(subscription)
                    .is_some_and(|prefs| prefs.wants(intent.category, &intent.session_id))
            })
            .map(|subscription| subscription.endpoint_sha256.clone())
            .collect();
        store
            .freeze_audience(&intent.dedupe_key, &audience, now)
            .await?;
    }
    Ok(())
}

/// The display copy of one send. Agent text stays out unless the person asked
/// for session names.
fn copy(category: Category, count: usize, incident: bool) -> (String, &'static str) {
    let title = match (category, count) {
        (Category::Input, 1) => "Cowboy needs your input".to_owned(),
        (Category::Input, _) => format!("{count} requests need your input"),
        (Category::Failure, 1) if incident => "Cowboy session error".to_owned(),
        (Category::Failure, 1) => "A task failed".to_owned(),
        (Category::Failure, _) => format!("{count} tasks failed"),
        (Category::Completion, 1) => "A task finished".to_owned(),
        (Category::Completion, _) => format!("{count} tasks finished"),
    };
    let body = match category {
        Category::Input => "Review the pending request.",
        Category::Failure | Category::Completion => "Open the session for details.",
    };
    (title, body)
}

/// The notification one group of deliveries becomes. `category` stays a value
/// every shipped service worker renders.
fn payload(group: &[Delivery], show_names: bool, session_title: &str) -> (Value, String) {
    let first = &group[0].intent;
    let (tag, wire) = match first.category {
        Category::Input => (
            format!("cowboy-input-{}", &endpoint_sha256(&first.dedupe_key)[..16]),
            "permission",
        ),
        Category::Failure => (format!("cowboy-{}-failure", first.session_id), "error"),
        Category::Completion => (
            format!("cowboy-{}-completion", first.session_id),
            "completed",
        ),
    };
    let incident = first.dedupe_key.starts_with("incident:");
    let (title, generic) = copy(first.category, group.len(), incident);
    let body = if show_names {
        format!(
            "{} — {generic}",
            session_title.chars().take(120).collect::<String>()
        )
    } else {
        generic.to_owned()
    };
    let mut message = json!({
        "version": 1,
        "category": wire,
        "sessionId": first.session_id,
        "title": title,
        "body": body,
        "tag": tag,
    });
    if group.len() == 1 && !first.destination.is_empty() {
        message["task"] = json!(first.destination);
    }
    (message, tag)
}

async fn settle(store: &Store, group: &[Delivery], state: &str) -> anyhow::Result<()> {
    for delivery in group {
        store
            .finish_delivery(
                &delivery.intent.dedupe_key,
                &delivery.endpoint_sha256,
                state,
            )
            .await?;
    }
    Ok(())
}

/// Later attempt, or the end of the road for this group.
async fn retry(
    store: &Store,
    group: &[Delivery],
    retry_after: Option<Duration>,
    now: i64,
) -> anyhow::Result<()> {
    for delivery in group {
        let attempts = delivery.attempts + 1;
        if attempts >= MAX_ATTEMPTS {
            COUNTERS.failed.fetch_add(1, Ordering::Relaxed);
            store
                .finish_delivery(
                    &delivery.intent.dedupe_key,
                    &delivery.endpoint_sha256,
                    "failed",
                )
                .await?;
            continue;
        }
        COUNTERS.retried.fetch_add(1, Ordering::Relaxed);
        let backoff = FIRST_RETRY_MS.saturating_mul(1 << (attempts - 1));
        let requested = retry_after
            .and_then(|delay| i64::try_from(delay.as_millis()).ok())
            .unwrap_or(0);
        store
            .retry_delivery(
                &delivery.intent.dedupe_key,
                &delivery.endpoint_sha256,
                attempts,
                now.saturating_add(backoff.max(requested)),
            )
            .await?;
    }
    Ok(())
}

fn retryable(status: reqwest::StatusCode) -> bool {
    status.is_server_error() || matches!(status.as_u16(), 408 | 425 | 429)
}

async fn send_group(
    state: &AppState,
    store: &Store,
    endpoint: &str,
    group: Vec<Delivery>,
    now: i64,
) -> anyhow::Result<()> {
    let Some(subscription) = store.web_push_subscription(endpoint).await? else {
        return settle(store, &group, "dropped").await;
    };
    let Some(prefs) = preferences(&subscription) else {
        return settle(store, &group, "dropped").await;
    };
    let mut live = Vec::with_capacity(group.len());
    let mut stale = Vec::new();
    for delivery in group {
        let intent = &delivery.intent;
        if intent.expires_at_ms > now
            && subscription.owner_user_id == intent.owner_user_id
            && prefs.wants(intent.category, &intent.session_id)
            && deliverable(state, store, intent).await?
        {
            live.push(delivery);
        } else {
            stale.push(delivery);
        }
    }
    settle(store, &stale, "dropped").await?;
    let Some(first) = live.first() else {
        return Ok(());
    };
    let title = state
        .hub
        .session_info(&first.intent.session_id)
        .map_or_else(|| "Session".to_owned(), |info| info.meta.title);
    let (message, tag) = payload(&live, prefs.show_session_names, &title);
    let body = message.to_string();
    let ttl = live
        .iter()
        .map(|delivery| delivery.intent.expires_at_ms - now)
        .max()
        .and_then(|ms| u64::try_from(ms).ok())
        .map_or(Duration::ZERO, Duration::from_millis);
    let keys = SubscriptionKeys {
        p256dh: subscription.p256dh.clone(),
        auth: subscription.auth.clone(),
    };
    let request = PushRequest {
        payload: body.as_bytes(),
        urgent: first.intent.category == Category::Input,
        ttl,
        topic: &push_topic(&tag),
    };
    match state
        .web_push
        .send(&subscription.endpoint, &keys, &request)
        .await
    {
        Ok(response) if response.status.is_success() => {
            COUNTERS
                .sent
                .fetch_add(live.len() as u64, Ordering::Relaxed);
            settle(store, &live, "sent").await
        }
        Ok(response) if matches!(response.status.as_u16(), 404 | 410) => {
            store
                .remove_web_push_subscription(&subscription.endpoint_sha256)
                .await?;
            settle(store, &live, "dropped").await
        }
        Ok(response) if retryable(response.status) => {
            tracing::warn!(status = %response.status, "Web Push service asked to retry");
            retry(store, &live, response.retry_after, now).await
        }
        Ok(response) => {
            tracing::warn!(status = %response.status, "Web Push service rejected notification");
            COUNTERS
                .failed
                .fetch_add(live.len() as u64, Ordering::Relaxed);
            settle(store, &live, "failed").await
        }
        Err(error) => {
            tracing::warn!(%error, "Web Push delivery failed");
            retry(store, &live, None, now).await
        }
    }
}

/// Group due deliveries per installation: failures and completions of one
/// session become one notification; every request stays its own.
fn grouped(deliveries: Vec<Delivery>) -> BTreeMap<(String, String), Vec<Delivery>> {
    let mut groups: BTreeMap<(String, String), Vec<Delivery>> = BTreeMap::new();
    for delivery in deliveries {
        let intent = &delivery.intent;
        let group = match intent.category {
            Category::Input => intent.dedupe_key.clone(),
            category => format!("{}\n{}", intent.session_id, category.code()),
        };
        groups
            .entry((delivery.endpoint_sha256.clone(), group))
            .or_default()
            .push(delivery);
    }
    groups
}

async fn tick(state: &AppState, store: &Store, now: i64) -> anyhow::Result<()> {
    expand(state, store, now).await?;
    let deliveries = store.due_deliveries(now, BATCH).await?;
    for ((endpoint, _), group) in grouped(deliveries) {
        send_group(state, store, &endpoint, group, now).await?;
    }
    Ok(())
}

pub(super) async fn run(state: Arc<AppState>, mut shutdown: watch::Receiver<bool>) {
    let Some(store) = state.store.clone() else {
        return;
    };
    let mut interval = tokio::time::interval(TICK);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut ticks: u64 = 0;
    loop {
        tokio::select! {
            _ = interval.tick() => {}
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() { return; }
                continue;
            }
        }
        ticks = ticks.wrapping_add(1);
        let now = now_ms();
        if state.service_config.get(&NOTIFICATIONS_DELIVERY)
            && let Err(error) = tick(&state, &store, now).await
        {
            tracing::warn!(%error, "task notification tick failed");
        }
        if ticks % PRUNE_EVERY_TICKS == 0
            && let Err(error) = store.prune_notifications(now - RETENTION_MS).await
        {
            tracing::warn!(%error, "pruning task notifications failed");
        }
    }
}

// --- Routes ------------------------------------------------------------------

pub(super) async fn subscribe(
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    Json(mut subscription): Json<WebPushSubscription>,
) -> Response {
    subscription.preferences = subscription.preferences.migrated();
    if let Err(error) = validate_subscription(&subscription) {
        tracing::warn!(%error, "rejecting Web Push subscription");
        return (StatusCode::BAD_REQUEST, "invalid Web Push subscription").into_response();
    }
    let Some(store) = &state.store else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let Ok(preferences) = serde_json::to_string(&subscription.preferences) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let row = StoredSubscription {
        endpoint_sha256: endpoint_sha256(&subscription.endpoint),
        endpoint: subscription.endpoint,
        owner_user_id: authenticated.principal.user_id.clone(),
        p256dh: subscription.keys.p256dh,
        auth: subscription.keys.auth,
        preferences,
    };
    let enrolled = authenticated
        .cookie_session
        .as_ref()
        .map(|session| session.session_id.as_str());
    match store
        .upsert_web_push_subscription(&row, enrolled, now_ms())
        .await
    {
        Ok(()) => Json(json!({ "subscribed": true })).into_response(),
        Err(error) => {
            tracing::warn!(%error, "storing Web Push subscription");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Unsubscribe {
    endpoint: String,
}

pub(super) async fn unsubscribe(
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    Json(request): Json<Unsubscribe>,
) -> Response {
    let Some(store) = &state.store else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let key = endpoint_sha256(&request.endpoint);
    // Knowing an endpoint does not let another account remove it.
    let owned = store
        .web_push_subscription(&key)
        .await
        .map(|row| row.is_some_and(|row| row.owner_user_id == authenticated.principal.user_id));
    let removed = match owned {
        Ok(true) => store.remove_web_push_subscription(&key).await,
        Ok(false) => Ok(()),
        Err(error) => Err(error),
    };
    match removed {
        Ok(()) => Json(json!({ "subscribed": false })).into_response(),
        Err(error) => {
            tracing::warn!(%error, "removing Web Push subscription");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PresenceRenewal {
    device_id: String,
    session_id: String,
}

fn valid_device(device: &str) -> bool {
    safe_session_id(device)
}

pub(super) async fn renew_presence(
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    Json(body): Json<PresenceRenewal>,
) -> Response {
    if !valid_device(&body.device_id)
        || !session_is_visible(&state.hub, &authenticated.principal, &body.session_id)
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    state.notifications.renew(
        &authenticated.principal.user_id,
        &body.device_id,
        &body.session_id,
        now_ms(),
    );
    StatusCode::NO_CONTENT.into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PresenceRelease {
    device_id: String,
}

pub(super) async fn release_presence(
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    Json(body): Json<PresenceRelease>,
) -> Response {
    state
        .notifications
        .release(&authenticated.principal.user_id, &body.device_id);
    StatusCode::NO_CONTENT.into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Acknowledge {
    task_key: String,
    marker: String,
}

/// A task key of this session's shape; the row needs no further proof, since
/// it only ever affects its own account's view of a session it can see.
fn valid_task_key(key: &str) -> bool {
    key.split_once(':').is_some_and(|(kind, id)| {
        matches!(kind, "subagent" | "call") && crate::transcript_anchors::valid_id(id)
    })
}

pub(super) async fn acknowledge(
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    Path(session_id): Path<String>,
    Json(body): Json<Acknowledge>,
) -> Response {
    if !session_is_visible(&state.hub, &authenticated.principal, &session_id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !valid_task_key(&body.task_key)
        || body.marker.is_empty()
        || body.marker.len() > MARKER_MAX_BYTES
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let Some(store) = &state.store else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match store
        .acknowledge_task(
            &authenticated.principal.user_id,
            &session_id,
            &body.task_key,
            &body.marker,
            now_ms(),
        )
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => {
            tracing::warn!(%error, "storing task acknowledgement");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}

/// The account's acknowledgements for the session bootstrap.
pub(super) async fn acknowledgements(
    store: Option<&Store>,
    owner: &str,
    session_id: &str,
) -> Option<Value> {
    let rows = store?.task_acknowledgements(owner, session_id).await.ok()?;
    Some(Value::Array(
        rows.into_iter()
            .map(|(task_key, marker)| json!({"task_key": task_key, "marker": marker}))
            .collect(),
    ))
}

#[cfg(test)]
#[path = "notifications_tests.rs"]
mod tests;
