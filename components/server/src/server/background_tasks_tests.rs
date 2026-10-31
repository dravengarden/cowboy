//! Controller conformance for background-task reconciliation
//! (docs/background-tasks.md §7). Every contract runs on SQLite here and on
//! PostgreSQL through `nix develop -c just test-postgres`.

use super::*;
use serde_json::{Value, json};
use std::time::Duration;

const SESSION: &str = "parent";

fn meta(id: &str) -> crate::core::SessionMeta {
    let hub = Hub::new();
    hub.create_local_session(
        id.into(),
        "claude-code".into(),
        "/tmp".into(),
        "Background tasks fixture".into(),
        SessionOrigin::Api,
        false,
    );
    let mut meta = hub.session_info(id).unwrap().meta;
    meta.machine_id = "hawk".into();
    meta
}

/// One Controller process: a Hub restored from the store with its writer.
struct Lifetime {
    hub: Hub,
    writer: tokio::task::JoinHandle<()>,
}

impl Lifetime {
    async fn start(store: &Store) -> Self {
        let health = Arc::new(PersistenceHealth::default());
        let (sink, rx) = StoreSink::channel(4096, health.clone());
        let hub = Hub::with_store(Some(sink));
        hub.warm_transcript_anchors(
            store
                .recent_transcript_anchors(crate::transcript_anchors::HUB_WARM_ANCHORS)
                .await
                .unwrap(),
        );
        hub.warm_subagent_marks(
            store
                .recent_subagent_snapshots(crate::transcript_anchors::HUB_WARM_ANCHORS)
                .await
                .unwrap(),
        );
        let restored = store
            .load_all()
            .await
            .unwrap()
            .into_iter()
            .map(|ls| RestoredSession {
                meta: ls.meta,
                log: ls.events,
                event_count: ls.event_count,
                reached_start: ls.reached_start,
                next_seq: ls.next_seq,
                queue: ls.queue,
                drafts: ls.drafts,
                config_options: ls.config_options,
                config_preferences: ls.config_preferences,
                mobile_review_state: ls.mobile_review_state,
                folder_id: ls.folder_id,
                last_event_at_ms: ls.last_event_at_ms,
            })
            .collect();
        hub.restore_reconciling_runtime(restored);
        let (_shutdown_tx, shutdown) = watch::channel(false);
        let writer_store = store.clone();
        let writer = tokio::spawn(async move {
            let _keep = _shutdown_tx;
            run_store_writer(writer_store, rx, health, shutdown).await;
        });
        Self { hub, writer }
    }

    /// Stop the process after its writer committed everything it accepted.
    async fn stop(self) {
        drop(self.hub);
        tokio::time::timeout(Duration::from_secs(30), self.writer)
            .await
            .expect("the writer drains once the Hub is gone")
            .unwrap();
    }
}

fn subagent(child: &str, state: &str, revision: u64, extra: &Value) -> Event {
    let mut update = json!({
        "sessionUpdate": "cowboy_subagent",
        "sessionId": child,
        "revision": revision,
        "title": format!("Agent {child}"),
        "state": state,
        "stopReason": if state == "idle" { json!("end_turn") } else { Value::Null },
        "cancellable": state == "running",
    });
    if let (Some(target), Some(extra)) = (update.as_object_mut(), extra.as_object()) {
        target.extend(extra.clone());
    }
    Event::Update { update }
}

fn filler(index: u64) -> Event {
    Event::Update {
        update: json!({
            "sessionUpdate": "tool_call",
            "toolCallId": format!("tool-{index}"),
            "title": "filler",
            "status": "completed",
        }),
    }
}

async fn rows_of(store: &Store, kind: &str, id_field: &str, id: &str) -> Vec<(u64, Value)> {
    rows_in(store, SESSION, kind, id_field, id).await
}

async fn rows_in(
    store: &Store,
    session: &str,
    kind: &str,
    id_field: &str,
    id: &str,
) -> Vec<(u64, Value)> {
    let (events, _, _) = store
        .history_page(session, u64::MAX >> 2, 5_000)
        .await
        .unwrap();
    events
        .into_iter()
        .filter_map(|envelope| match envelope.event {
            Event::Update { update }
                if update["sessionUpdate"] == kind && update[id_field] == id =>
            {
                Some((envelope.seq, update))
            }
            _ => None,
        })
        .collect()
}

fn items_by_child(state: &Value) -> std::collections::HashMap<String, Value> {
    state["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["update"]["sessionId"].as_str().unwrap().to_owned(),
                item.clone(),
            )
        })
        .collect()
}

async fn background_tasks_contract(url: &str, root: &FsPath) {
    let store = Store::connect(url, root.join("artifacts")).await.unwrap();
    store.migrate().await.unwrap();
    store.insert_session(&meta(SESSION)).await.unwrap();

    // --- T1: A finishes beyond the reconnect tail while the client is away.
    let first = Lifetime::start(&store).await;
    first.hub.note_worker_incarnation(SESSION, "worker-1");
    first.hub.push(
        SESSION,
        subagent("a", "running", 10, &json!({"incarnation":"worker-1"})),
    );
    let client_seen = first
        .hub
        .subagent_observations(SESSION)
        .unwrap()
        .watermark_seq;
    for index in 0..300 {
        first.hub.push(SESSION, filler(index));
    }
    first.hub.push(
        SESSION,
        subagent("a", "idle", 20, &json!({"incarnation":"worker-1"})),
    );
    let (tail, _) = first.hub.snapshot(SESSION).unwrap();
    assert!(
        tail.iter().all(|envelope| !matches!(&envelope.event,
            Event::Update { update } if update["sessionUpdate"] == "cowboy_subagent")),
        "the reconnect tail no longer carries A's anchor row (the original bug)"
    );
    let state =
        background_tasks::subagent_state(&first.hub, Some(&store), SESSION, Some(client_seen))
            .await
            .unwrap();
    let items = items_by_child(&state);
    assert_eq!(
        items["a"]["update"]["state"], "idle",
        "T1: bootstrap repairs A"
    );
    assert_eq!(items["a"]["anchor_seq"], 0, "A keeps its first row");
    assert_eq!(state["complete"], true);
    assert_eq!(state["incarnation"], "worker-1");
    assert_eq!(state["epoch"], first.hub.boot_epoch());
    assert!(items["a"]["update"]["observedSeq"].as_u64().unwrap() > client_seen);

    // --- T2: 60 more children start and finish before the client returns.
    for index in 0..60 {
        let child = format!("later-{index}");
        first
            .hub
            .push(SESSION, subagent(&child, "running", 1, &json!({})));
        first.hub.push(SESSION, filler(1_000 + index));
        first
            .hub
            .push(SESSION, subagent(&child, "idle", 2, &json!({})));
    }
    let state =
        background_tasks::subagent_state(&first.hub, Some(&store), SESSION, Some(client_seen))
            .await
            .unwrap();
    let items = items_by_child(&state);
    assert_eq!(items.len(), 61, "T2: no newest-N window");
    assert!(items.values().all(|item| item["update"]["state"] == "idle"));
    // Without a client watermark only non-terminal children are listed.
    first
        .hub
        .push(SESSION, subagent("b", "running", 1, &json!({})));
    let state = background_tasks::subagent_state(&first.hub, Some(&store), SESSION, None)
        .await
        .unwrap();
    assert_eq!(items_by_child(&state).keys().collect::<Vec<_>>(), vec!["b"]);

    // --- T5: duplicate, older and conflicting revisions of one incarnation.
    let metrics = &crate::transcript_anchors::METRICS;
    let conflicts = metrics
        .revision_conflicts
        .load(std::sync::atomic::Ordering::Relaxed);
    let before = first
        .hub
        .subagent_observations(SESSION)
        .unwrap()
        .watermark_seq;
    first
        .hub
        .push(SESSION, subagent("b", "running", 1, &json!({})));
    assert_eq!(
        first
            .hub
            .subagent_observations(SESSION)
            .unwrap()
            .watermark_seq,
        before,
        "an exact duplicate is not a new event"
    );
    first
        .hub
        .push(SESSION, subagent("b", "idle", 5, &json!({})));
    first
        .hub
        .push(SESSION, subagent("b", "running", 4, &json!({})));
    let state = background_tasks::subagent_state(&first.hub, Some(&store), SESSION, Some(before))
        .await
        .unwrap();
    assert_eq!(
        items_by_child(&state)["b"]["update"]["state"],
        "idle",
        "an older revision never wins"
    );
    first.hub.push(
        SESSION,
        subagent("b", "idle", 5, &json!({"error":"conflicting payload"})),
    );
    assert_eq!(
        metrics
            .revision_conflicts
            .load(std::sync::atomic::Ordering::Relaxed),
        conflicts + 1
    );
    let state = background_tasks::subagent_state(&first.hub, Some(&store), SESSION, Some(before))
        .await
        .unwrap();
    assert_eq!(
        items_by_child(&state)["b"]["update"]["error"],
        "conflicting payload",
        "the later observation of a conflict is shown"
    );

    // --- T11 (part 1): one Call anchor, repeated pushes replace it.
    let origin = background_tasks::CallOrigin::Mcp {
        action: "calls.start".into(),
        operation: "op-1".into(),
    };
    background_tasks::push_call_anchor(&first.hub, SESSION, "call-1", "op-1", &origin, None);
    background_tasks::push_call_anchor(&first.hub, SESSION, "call-1", "op-1", &origin, None);
    background_tasks::push_call_anchor(
        &first.hub,
        SESSION,
        "call-2",
        "cli-1",
        &background_tasks::CallOrigin::Gateway,
        None,
    );
    first.stop().await;

    let a_rows = rows_of(&store, "cowboy_subagent", "sessionId", "a").await;
    assert_eq!(a_rows.len(), 1);
    assert_eq!(a_rows[0].1["state"], "idle");
    let call_rows = rows_of(&store, "cowboy_call_anchor", "callId", "call-1").await;
    assert_eq!(call_rows.len(), 1, "T11: exactly one anchor per call");
    assert_eq!(call_rows[0].1["origin"]["kind"], "mcp");
    assert_eq!(call_rows[0].1["origin"]["operation"], "op-1");
    let cli_rows = rows_of(&store, "cowboy_call_anchor", "callId", "call-2").await;
    assert_eq!(
        cli_rows[0].1["origin"],
        json!({"kind":"gateway"}),
        "no structured link"
    );

    // --- T3: Controller restart between C's first row and its terminal one.
    let second = Lifetime::start(&store).await;
    second.hub.push(
        SESSION,
        subagent("c", "running", 1, &json!({"incarnation":"worker-1"})),
    );
    second.stop().await;
    let third = Lifetime::start(&store).await;
    // T6: the new worker incarnation announces C again after its restart.
    third.hub.note_worker_incarnation(SESSION, "worker-2");
    let state = background_tasks::subagent_state(&third.hub, Some(&store), SESSION, None)
        .await
        .unwrap();
    let items = items_by_child(&state);
    assert_eq!(state["incarnation"], "worker-2");
    assert_eq!(
        items["c"]["update"]["incarnation"], "worker-1",
        "T6: C was reported by the previous incarnation"
    );
    assert_ne!(state["epoch"], Value::Null);
    third.hub.push(
        SESSION,
        subagent("c", "idle", 2, &json!({"incarnation":"worker-2"})),
    );
    third.hub.push(
        SESSION,
        subagent("a", "idle", 21, &json!({"incarnation":"worker-2"})),
    );
    third.stop().await;
    let c_rows = rows_of(&store, "cowboy_subagent", "sessionId", "c").await;
    assert_eq!(c_rows.len(), 1, "T3: no second anchor row after a restart");
    assert_eq!(c_rows[0].1["state"], "idle");
    assert_eq!(c_rows[0].1["incarnation"], "worker-2");
    assert_eq!(
        rows_of(&store, "cowboy_subagent", "sessionId", "a")
            .await
            .len(),
        1
    );

    // --- T11 (part 2): a call admitted before its anchor is repaired once.
    let fourth = Lifetime::start(&store).await;
    let missing = store.calls_missing_anchor(0, 10).await.unwrap();
    assert!(missing.is_empty(), "anchored calls are not repaired");
    background_tasks::repair_call_anchors(&fourth.hub, &store).await;
    fourth.stop().await;

    // --- T17: a session whose rows predate stamping and the anchor table (no
    // observedSeq, no incarnation, no anchor) still decodes and resolves
    // through the one-time backfill.
    const LEGACY: &str = "legacy-session";
    store.insert_session(&meta(LEGACY)).await.unwrap();
    let legacy = Envelope {
        session_id: LEGACY.into(),
        seq: 7,
        cmid: None,
        event: subagent("legacy", "running", 3, &json!({})),
    };
    store
        .upsert_event_batch(&[legacy], &HashMap::from([(LEGACY.to_owned(), 8)]))
        .await
        .unwrap();
    let fifth = Lifetime::start(&store).await;
    let state = background_tasks::subagent_state(&fifth.hub, Some(&store), LEGACY, None)
        .await
        .unwrap();
    let items = items_by_child(&state);
    assert_eq!(items["legacy"]["anchor_seq"], 7);
    assert!(items["legacy"]["update"].get("observedSeq").is_none());
    fifth
        .hub
        .push(LEGACY, subagent("legacy", "idle", 4, &json!({})));
    fifth.stop().await;
    let legacy_rows = rows_in(&store, LEGACY, "cowboy_subagent", "sessionId", "legacy").await;
    assert_eq!(
        legacy_rows.len(),
        1,
        "the legacy anchor is replaced in place"
    );
    assert_eq!(legacy_rows[0].0, 7);
    assert_eq!(legacy_rows[0].1["state"], "idle");

    // I6: the client's cached non-terminal ids resolve by lookup; an id the
    // Controller never had (or that was cleared) is `not_found`, never Done.
    let third_view = Lifetime::start(&store).await;
    let answer = background_tasks::lookup_items(
        &third_view.hub,
        Some(&store),
        SESSION,
        &["a".to_owned(), "c".to_owned(), "gone".to_owned()],
    )
    .await
    .unwrap()
    .unwrap();
    let statuses: Vec<(String, String)> = answer["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["id"].as_str().unwrap().to_owned(),
                item["status"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        statuses,
        vec![
            ("a".into(), "found".into()),
            ("c".into(), "found".into()),
            ("gone".into(), "not_found".into())
        ]
    );
    assert_eq!(answer["items"][0]["update"]["state"], "idle");
    assert_eq!(answer["items"][1]["anchor_seq"], c_rows[0].0);
    assert!(
        background_tasks::lookup_items(&third_view.hub, Some(&store), "unknown", &["a".to_owned()])
            .await
            .unwrap()
            .is_none()
    );
    third_view.stop().await;

    // Clearing the transcript removes its anchors: a later child is new.
    store.clear_events(SESSION, 0).await.unwrap();
    assert_eq!(
        store
            .transcript_anchor(SESSION, "subagent:a")
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        store.backfill_subagent_anchors(SESSION).await.unwrap(),
        None
    );
}

#[tokio::test]
async fn sqlite_background_tasks_reconciliation_contract() {
    let root = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", root.path().join("fixture.sqlite").display());
    background_tasks_contract(&url, root.path()).await;
}

#[tokio::test]
#[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
async fn postgres_background_tasks_reconciliation_contract() {
    let root = tempfile::tempdir().unwrap();
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL")
        .expect("COWBOY_TEST_POSTGRES_URL must name an isolated empty database");
    background_tasks_contract(&url, root.path()).await;
}

/// Review regressions: rows that predate the table, children past any fixed
/// prefix, a replay after a restart, and a read that fails mid-batch.
async fn anchor_durability_contract(url: &str, root: &FsPath) {
    let store = Store::connect(url, root.join("artifacts")).await.unwrap();
    store.migrate().await.unwrap();
    store.insert_session(&meta(SESSION)).await.unwrap();

    // Legacy duplicates of one child: the anchor keeps the earliest position
    // and takes the newest snapshot.
    let envelope = |seq: u64, event: Event| Envelope {
        session_id: SESSION.into(),
        seq,
        cmid: None,
        event,
    };
    let mut rows = vec![
        envelope(5, subagent("dup", "running", 1, &json!({}))),
        envelope(400, subagent("dup", "idle", 9, &json!({}))),
    ];
    // More children than any page, so the last one sits past the prefix.
    for index in 0..2_100u64 {
        rows.push(envelope(
            1_000 + index,
            subagent(&format!("many-{index}"), "running", 1, &json!({})),
        ));
    }
    store
        .upsert_event_batch(&rows, &HashMap::from([(SESSION.to_owned(), 4_000)]))
        .await
        .unwrap();

    let first = Lifetime::start(&store).await;
    let state = background_tasks::subagent_state(&first.hub, Some(&store), SESSION, None)
        .await
        .unwrap();
    assert_eq!(state["complete"], true);
    let items = items_by_child(&state);
    assert_eq!(items.len(), 2_100, "every non-terminal child, no prefix");
    assert!(!items.contains_key("dup"), "the terminal snapshot won");
    let last = ["dup".to_owned(), "many-2099".to_owned()];
    let found = background_tasks::lookup_items(&first.hub, Some(&store), SESSION, &last)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found["items"][0]["status"], "found");
    assert_eq!(found["items"][0]["anchor_seq"], 5);
    assert_eq!(found["items"][0]["update"]["state"], "idle");
    assert_eq!(found["items"][1]["status"], "found");

    // The durable row is at revision 20 of worker-1; a restart must not let a
    // replay of revision 10 become the newest observation.
    first.hub.push(
        SESSION,
        subagent("fence", "idle", 20, &json!({"incarnation":"worker-1"})),
    );
    first.stop().await;
    let second = Lifetime::start(&store).await;
    second.hub.push(
        SESSION,
        subagent("fence", "running", 10, &json!({"incarnation":"worker-1"})),
    );
    second.stop().await;
    let fenced = rows_of(&store, "cowboy_subagent", "sessionId", "fence").await;
    assert_eq!(fenced.len(), 1);
    assert_eq!(
        fenced[0].1["state"], "idle",
        "a replay never regresses a row"
    );

    // A failed durable read leaves no speculative anchor behind.
    let mut reducer = EventReducer::default();
    reducer.adopt_anchor(SESSION, "subagent:spec", 77);
    reducer.forget_anchor(SESSION, "subagent:spec");
    assert!(!reducer.knows_anchor(SESSION, "subagent:spec"));
}

#[tokio::test]
async fn sqlite_anchor_durability_contract() {
    let root = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", root.path().join("fixture.sqlite").display());
    anchor_durability_contract(&url, root.path()).await;
}

#[tokio::test]
#[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
async fn postgres_anchor_durability_contract() {
    let root = tempfile::tempdir().unwrap();
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL")
        .expect("COWBOY_TEST_POSTGRES_URL must name an isolated empty database");
    anchor_durability_contract(&url, root.path()).await;
}

/// T11 (part 3): a call admitted (ledger committed) before the Controller
/// crashed without its anchor gets exactly one anchor at the next start.
async fn call_anchor_repair_contract(url: &str, root: &FsPath) {
    use crate::managed_calls::lifecycle::{Placement, Record, State as CallState};
    let store = Store::connect(url, root.join("artifacts")).await.unwrap();
    store.migrate().await.unwrap();
    store.insert_session(&meta(SESSION)).await.unwrap();
    let request = crate::managed_calls::Request::parse(br#"{"schema":1,"request_id":"op-9","purpose":"review","instruction":"Review","context":{"scope":"current-worktree"},"access":"read-only","conversation":{"mode":"fresh"}}"#).unwrap();
    let record = Record {
        schema: 1,
        call_id: "call-9".into(),
        request_id: "op-9".into(),
        request_digest: request.digest(),
        provider: "codex".into(),
        purpose: request.purpose,
        access: request.access,
        labels: request.labels.clone(),
        placement: Placement {
            service_id: "svc".into(),
            parent_session_id: SESSION.into(),
            parent_revision: "r1".into(),
            machine_id: "hawk".into(),
            workspace_id: "project".into(),
            cwd: "/workspace".into(),
        },
        child_session_id: "child-9".into(),
        state: CallState::Queued,
        revision: 1,
        created_at_ms: 1,
        updated_at_ms: 1,
        started_at_ms: None,
        finished_at_ms: None,
        configuration: None,
        provider_account_id: None,
        input_revision: None,
        result: None,
        runtime_machine_id: None,
        provider_version: None,
        provider_generation_digest: None,
        child_cursor: None,
        cancel_requested_at_ms: None,
        error: None,
        preset: None,
        selection: None,
    };
    store.admit_managed_call(&record, &request).await.unwrap();
    assert_eq!(
        store.calls_missing_anchor(0, 10).await.unwrap(),
        vec![(SESSION.to_owned(), "call-9".to_owned())]
    );
    let lifetime = Lifetime::start(&store).await;
    background_tasks::repair_call_anchors(&lifetime.hub, &store).await;
    lifetime.stop().await;
    assert!(store.calls_missing_anchor(0, 10).await.unwrap().is_empty());
    let lifetime = Lifetime::start(&store).await;
    background_tasks::repair_call_anchors(&lifetime.hub, &store).await;
    lifetime.stop().await;
    let rows = rows_of(&store, "cowboy_call_anchor", "callId", "call-9").await;
    assert_eq!(rows.len(), 1, "repair is idempotent");
    assert_eq!(rows[0].1["requestId"], "op-9");
    assert_eq!(rows[0].1["origin"]["kind"], "repaired");

    // Eligibility is decided before the limit: ended Calls older than the
    // boundary never crowd out a newer one that lost its anchor.
    let admit = |index: u64| {
        let mut next = record.clone();
        next.call_id = format!("call-old-{index}");
        next.child_session_id = format!("child-old-{index}");
        next
    };
    let boundary = store.managed_call_admission_high_water().await.unwrap();
    for index in 0..3 {
        let mut body: Value = serde_json::from_str(r#"{"schema":1,"purpose":"review","instruction":"Review","context":{"scope":"current-worktree"},"access":"read-only","conversation":{"mode":"fresh"}}"#).unwrap();
        body["request_id"] = json!(format!("op-old-{index}"));
        let request = crate::managed_calls::Request::parse(body.to_string().as_bytes()).unwrap();
        let mut queued = admit(index);
        queued.request_id = format!("op-old-{index}");
        queued.request_digest = request.digest();
        store.admit_managed_call(&queued, &request).await.unwrap();
        let mut ended = queued.clone();
        ended.state = CallState::Cancelled;
        ended.revision = 2;
        ended.updated_at_ms = 2;
        ended.finished_at_ms = Some(2);
        assert!(store.advance_managed_call(&queued, &ended).await.unwrap());
    }
    let high_water = store.managed_call_admission_high_water().await.unwrap();
    assert_eq!(
        store.calls_missing_anchor(high_water, 2).await.unwrap(),
        Vec::new(),
        "ended Calls from before the boundary are not eligible"
    );
    assert_eq!(
        store.calls_missing_anchor(boundary, 2).await.unwrap().len(),
        2,
        "ended Calls after the boundary are"
    );
}

#[tokio::test]
async fn sqlite_call_anchor_repair_contract() {
    let root = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", root.path().join("fixture.sqlite").display());
    call_anchor_repair_contract(&url, root.path()).await;
}

#[tokio::test]
#[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
async fn postgres_call_anchor_repair_contract() {
    let root = tempfile::tempdir().unwrap();
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL")
        .expect("COWBOY_TEST_POSTGRES_URL must name an isolated empty database");
    call_anchor_repair_contract(&url, root.path()).await;
}
