use super::*;
use crate::core::{Event, Hub, SessionOrigin};
use crate::task_notifications::Occurrence;
use std::path::Path as FsPath;

const SESSION: &str = "parent";
const OWNER: &str = "0123456789abcdef0123456789abcdef";

fn delivery(key: &str, session: &str, category: Category, destination: &str) -> Delivery {
    Delivery {
        intent: Intent {
            dedupe_key: key.into(),
            owner_user_id: "owner".into(),
            session_id: session.into(),
            category,
            destination: destination.into(),
            marker: String::new(),
            expires_at_ms: 1_000_000,
        },
        endpoint_sha256: "device".into(),
        attempts: 0,
    }
}

#[test]
fn requests_stay_independent_and_task_bursts_share_one_notification() {
    let groups = grouped(vec![
        delivery("req:perm:s:1", "s", Category::Input, ""),
        delivery("req:perm:s:2", "s", Category::Input, ""),
        delivery("exec:a:terminal", "s", Category::Failure, "call:a"),
        delivery("exec:b:terminal", "s", Category::Failure, "call:b"),
        delivery("exec:c:terminal", "s", Category::Completion, "call:c"),
        delivery("exec:d:terminal", "other", Category::Failure, "call:d"),
    ]);
    assert_eq!(groups.len(), 5);
    let burst = groups
        .values()
        .find(|group| group.len() == 2)
        .expect("the failures of one session coalesce");
    let (message, tag) = payload(burst, false, "secret title");
    assert_eq!(tag, "cowboy-s-failure");
    assert_eq!(message["title"], "2 tasks failed");
    assert_eq!(message["category"], "error", "a value every worker renders");
    assert!(
        message.get("task").is_none(),
        "an aggregate has no single task"
    );
    assert!(!message.to_string().contains("secret title"));
}

#[test]
fn input_tags_are_per_occurrence_and_a_single_task_links_to_it() {
    let one = delivery("req:perm:s:1", "s", Category::Input, "");
    let two = delivery("req:perm:s:2", "s", Category::Input, "");
    let tags = (payload(&[one], false, "").1, payload(&[two], false, "").1);
    assert_ne!(
        tags.0, tags.1,
        "an offline request is never replaced by the next"
    );
    let (message, _) = payload(
        &[delivery(
            "entity:s:c:9",
            "s",
            Category::Failure,
            "subagent:c",
        )],
        true,
        "Title",
    );
    assert_eq!(message["task"], "subagent:c");
    assert!(message["body"].as_str().unwrap().starts_with("Title"));
}

#[test]
fn a_lease_lasts_until_it_expires_and_a_release_ends_it() {
    let presence = Presence::default();
    presence.renew("user", "phone", "s", 1_000);
    assert_eq!(presence.held_until("user", "s", 2_000), Some(31_000));
    assert_eq!(
        presence.held_until("user", "s", 31_000),
        None,
        "a stale lease delivers"
    );
    assert_eq!(
        presence.held_until("other", "s", 2_000),
        None,
        "per account"
    );
    assert_eq!(presence.held_until("user", "t", 2_000), None, "per session");
    presence.release("user", "phone");
    assert_eq!(presence.held_until("user", "s", 2_000), None);
}

#[test]
fn only_transient_answers_are_retried() {
    for code in [408, 425, 429, 500, 503] {
        assert!(retryable(reqwest::StatusCode::from_u16(code).unwrap()));
    }
    for code in [400, 401, 403, 404, 410, 413] {
        assert!(!retryable(reqwest::StatusCode::from_u16(code).unwrap()));
    }
}

fn hub_with_session() -> Hub {
    let hub = Hub::new();
    hub.create_local_session(
        SESSION.into(),
        "claude-code".into(),
        "/tmp".into(),
        "Notification fixture".into(),
        SessionOrigin::Api,
        false,
    );
    hub
}

async fn store_for(url: &str, root: &FsPath, owner: Option<&str>) -> Store {
    let store = Store::connect(url, root.join("artifacts")).await.unwrap();
    store.migrate().await.unwrap();
    if let Some(owner) = owner {
        store
            .insert_user(&crate::store::ProductUser {
                id: owner.into(),
                username: "reader".into(),
                password_algo: "argon2id".into(),
                password_hash: "unused-hermetic-fixture".into(),
                created_at_ms: 1,
                updated_at_ms: 1,
                disabled_at_ms: None,
            })
            .await
            .unwrap();
    }
    let hub = hub_with_session();
    let mut meta = hub.session_info(SESSION).unwrap().meta;
    meta.machine_id = "hawk".into();
    meta.owner_user_id = owner.map(str::to_owned);
    store.insert_session(&meta).await.unwrap();
    store
}

async fn storage_contract(url: &str, root: &FsPath) {
    let store = store_for(url, root, Some(OWNER)).await;
    let failure = Occurrence::call(SESSION, "c1", Category::Failure, 500);

    // One occurrence, however often it is recorded.
    store.record_occurrence(&failure, 1_000).await.unwrap();
    store.record_occurrence(&failure, 9_000).await.unwrap();
    assert!(
        store.due_intents(3_999, 10).await.unwrap().is_empty(),
        "the burst window"
    );
    let due = store.due_intents(4_000, 10).await.unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].owner_user_id, OWNER);
    assert_eq!(due[0].expires_at_ms, 1_000 + Category::Failure.ttl_ms());

    // A deferred intent comes back when the lease ends.
    store
        .defer_intent(&due[0].dedupe_key, 40_000)
        .await
        .unwrap();
    assert!(store.due_intents(39_999, 10).await.unwrap().is_empty());

    // An empty audience is a frozen audience: a later enrollment gets nothing.
    store
        .freeze_audience(&due[0].dedupe_key, &[], 40_000)
        .await
        .unwrap();
    assert!(store.due_intents(50_000, 10).await.unwrap().is_empty());
    assert!(store.due_deliveries(50_000, 10).await.unwrap().is_empty());

    // Deliveries resume from the store and retry at their own time.
    let input = Occurrence::permission(SESSION, 7);
    store.record_occurrence(&input, 60_000).await.unwrap();
    let endpoints = ["device-a".to_owned(), "device-b".to_owned()];
    store
        .freeze_audience(&input.dedupe_key, &endpoints, 60_000)
        .await
        .unwrap();
    store
        .freeze_audience(&input.dedupe_key, &endpoints, 60_001)
        .await
        .unwrap();
    let pending = store.due_deliveries(60_000, 10).await.unwrap();
    assert_eq!(pending.len(), 2);
    store
        .finish_delivery(&input.dedupe_key, "device-a", "sent")
        .await
        .unwrap();
    store
        .retry_delivery(&input.dedupe_key, "device-b", 1, 70_000)
        .await
        .unwrap();
    assert!(store.due_deliveries(69_999, 10).await.unwrap().is_empty());
    let retry = store.due_deliveries(70_000, 10).await.unwrap();
    assert_eq!(retry.len(), 1);
    assert_eq!(
        (retry[0].attempts, retry[0].endpoint_sha256.as_str()),
        (1, "device-b")
    );

    // Acknowledgement needs no ordering with its intent and is session scoped.
    assert!(
        !store
            .task_acknowledged(OWNER, SESSION, "call:c1", "500")
            .await
            .unwrap()
    );
    store
        .acknowledge_task(OWNER, SESSION, "call:c1", "500", 1)
        .await
        .unwrap();
    store
        .acknowledge_task(OWNER, SESSION, "call:c1", "500", 2)
        .await
        .unwrap();
    assert!(
        store
            .task_acknowledged(OWNER, SESSION, "call:c1", "500")
            .await
            .unwrap()
    );
    assert!(
        !store
            .task_acknowledged("other", SESSION, "call:c1", "500")
            .await
            .unwrap()
    );
    assert!(
        !store
            .task_acknowledged(OWNER, SESSION, "call:c1", "501")
            .await
            .unwrap()
    );
    assert_eq!(
        store
            .task_acknowledgements(OWNER, SESSION)
            .await
            .unwrap()
            .len(),
        1
    );

    // An endpoint belongs to the account that last enrolled it.
    let row = |owner: &str| StoredSubscription {
        endpoint_sha256: "hash".into(),
        endpoint: "https://web.push.apple.com/x".into(),
        owner_user_id: owner.into(),
        p256dh: "p".into(),
        auth: "a".into(),
        preferences: "{}".into(),
    };
    store
        .upsert_web_push_subscription(&row("alice"), Some("cookie-a"), 1)
        .await
        .unwrap();
    store
        .upsert_web_push_subscription(&row("bob"), Some("cookie-b"), 2)
        .await
        .unwrap();
    assert!(
        store
            .web_push_subscriptions("alice")
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(store.web_push_subscriptions("bob").await.unwrap().len(), 1);
    store
        .remove_web_push_subscriptions_of_session("cookie-a")
        .await
        .unwrap();
    assert_eq!(store.web_push_subscriptions("bob").await.unwrap().len(), 1);
    store
        .remove_web_push_subscriptions_of_session("cookie-b")
        .await
        .unwrap();
    assert!(
        store
            .web_push_subscriptions("bob")
            .await
            .unwrap()
            .is_empty()
    );

    // Retention removes finished work and its deliveries.
    store.prune_notifications(i64::MAX).await.unwrap();
    assert!(store.due_deliveries(i64::MAX, 10).await.unwrap().is_empty());
}

async fn unowned_contract(url: &str, root: &FsPath) {
    let store = store_for(url, root, None).await;
    store
        .record_occurrence(&Occurrence::permission(SESSION, 1), 1)
        .await
        .unwrap();
    assert!(store.due_intents(i64::MAX, 10).await.unwrap().is_empty());
}

#[tokio::test]
async fn sqlite_notification_storage_contract() {
    let root = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", root.path().join("fixture.sqlite").display());
    storage_contract(&url, root.path()).await;
    let other = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", other.path().join("fixture.sqlite").display());
    unowned_contract(&url, other.path()).await;
}

#[tokio::test]
#[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
async fn postgres_notification_storage_contract() {
    let root = tempfile::tempdir().unwrap();
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL")
        .expect("COWBOY_TEST_POSTGRES_URL must name an isolated empty database");
    storage_contract(&url, root.path()).await;
}

fn snapshot(state: &str, revision: u64, extra: &Value) -> Event {
    let mut update = json!({
        "sessionUpdate": "cowboy_subagent",
        "sessionId": "c",
        "revision": revision,
        "incarnation": "w1",
        "title": "Agent",
        "state": state,
        "stopReason": if state == "idle" { json!("error") } else { Value::Null },
    });
    if let (Some(target), Some(extra)) = (update.as_object_mut(), extra.as_object()) {
        target.extend(extra.clone());
    }
    Event::Update { update }
}

/// The Hub writes one occurrence per terminal transition: metadata updates keep
/// its identity, and a resumed child's next ending is a new one.
#[tokio::test]
async fn subagent_endings_are_occurrences_not_snapshots() {
    let health = Arc::new(crate::core::PersistenceHealth::default());
    let (sink, mut rx) = crate::core::StoreSink::channel(64, health);
    let hub = Hub::with_store(Some(sink));
    hub.create_local_session(
        SESSION.into(),
        "claude-code".into(),
        "/tmp".into(),
        "Fixture".into(),
        SessionOrigin::Api,
        false,
    );
    hub.push(SESSION, snapshot("running", 1, &json!({})));
    hub.push(SESSION, snapshot("idle", 2, &json!({})));
    hub.push(
        SESSION,
        snapshot("idle", 3, &json!({"result":"more metadata"})),
    );
    hub.push(SESSION, snapshot("running", 4, &json!({})));
    hub.push(SESSION, snapshot("idle", 5, &json!({})));
    hub.push(SESSION, snapshot("idle", 2, &json!({})));
    drop(hub);
    let mut occurrences = Vec::new();
    let mut terminal = Vec::new();
    while let Some(write) = rx.recv().await {
        match write {
            crate::core::StoreWrite::RecordOccurrence(occurrence) => occurrences.push(occurrence),
            crate::core::StoreWrite::AppendEvent(envelope) => {
                if let Event::Update { update } = envelope.event
                    && update["sessionUpdate"] == "cowboy_subagent"
                    && update["state"] == "idle"
                {
                    terminal.push(update["terminalSeq"].as_u64().unwrap());
                }
            }
            _ => {}
        }
    }
    assert_eq!(occurrences.len(), 2, "{occurrences:?}");
    assert_eq!(terminal[0], terminal[1], "metadata keeps the identity");
    assert_ne!(
        terminal[1], terminal[2],
        "a resumed child's ending is a new one"
    );
    assert_eq!(occurrences[0].category, Category::Failure);
    assert_eq!(occurrences[0].marker, terminal[0].to_string());
}
