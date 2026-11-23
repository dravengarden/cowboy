use super::*;
use crate::core::{Event, Hub};
use crate::managed_calls::lifecycle::{Placement, Record, State};
use crate::prompt_origin::{apply_prompt_origin, origin_from_cmid};
use crate::server::agent_tools::AgentTools;

const PARENT: &str = "parent";

struct Fixture {
    hub: Hub,
    store: Store,
    policy: CallsPolicy,
    calls: usize,
    _dir: tempfile::TempDir,
}

impl Fixture {
    async fn new(limit: u16) -> Self {
        let hub = Hub::new();
        hub.create_session(crate::core::SessionRegistration {
            id: PARENT.into(),
            provider: "codex".into(),
            provider_version: String::new(),
            provider_generation_digest: String::new(),
            provider_account_id: crate::provider_account::ProviderAccountId::default(),
            provider_auth_generation: None,
            provider_behavior: None,
            machine_id: "hawk".into(),
            workspace_id: None,
            workspace_name: None,
            workspace_source_path: None,
            execution_binding: None,
            cwd: "/workspace".into(),
            title: "Parent".into(),
            origin: crate::core::SessionOrigin::Web,
            system: false,
            owner_user_id: None,
            owner_username: None,
        });
        let dir = tempfile::tempdir().unwrap();
        let store = Store::connect("sqlite::memory:", dir.path().into())
            .await
            .unwrap();
        store.migrate().await.unwrap();
        store
            .insert_session(&hub.session_info(PARENT).unwrap().meta)
            .await
            .unwrap();
        let mut policy = AgentTools::builtin("codex").calls;
        policy.max_per_turn = limit;
        policy.max_concurrent = 64;
        Self {
            hub,
            store,
            policy,
            calls: 0,
            _dir: dir,
        }
    }

    /// A prompt as the Hub records it: the origin its cmid selects is stamped
    /// on the echoed user-role chunk.
    fn prompt(&self, cmid: Option<&str>) {
        self.prompt_text(cmid, "prompt");
    }

    fn prompt_text(&self, cmid: Option<&str>, text: &str) {
        let mut update = json!({
            "sessionUpdate": "user_message_chunk",
            "content": {"type": "text", "text": text}
        });
        apply_prompt_origin(&mut update, &origin_from_cmid(cmid));
        self.hub
            .push_tagged(PARENT, Event::Update { update }, cmid.map(str::to_owned));
    }

    /// A user-role chunk no send claimed and the Hub never stamped, as an ACP
    /// client's unmatched text arrives.
    fn unclaimed_chunk(&self) {
        self.hub.push(
            PARENT,
            Event::Update {
                update: json!({"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"unmatched"}}),
            },
        );
    }

    fn work(&self) {
        self.hub.push(
            PARENT,
            Event::Update {
                update: json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"working"}}),
            },
        );
    }

    /// One call through the real admission check, recorded in the ledger as
    /// the coordinator records it.
    async fn call(&mut self) -> Result<i64, Value> {
        let turn = admit(&self.hub, Some(&self.store), PARENT, &self.policy).await?;
        self.record(Some(turn)).await;
        Ok(turn)
    }

    async fn record(&mut self, turn: Option<i64>) {
        self.calls += 1;
        let id = format!("call-{}", self.calls);
        let request = crate::managed_calls::Request::parse(
            format!(
                r#"{{"schema":1,"request_id":"{id}","purpose":"review","instruction":"Review","context":{{"scope":"current-worktree"}},"access":"read-only","conversation":{{"mode":"fresh"}}}}"#
            )
            .as_bytes(),
        )
        .unwrap();
        let record = Record {
            schema: 1,
            call_id: id.clone(),
            request_id: id.clone(),
            request_digest: request.digest(),
            provider: "claude-code".into(),
            purpose: request.purpose,
            access: request.access,
            labels: request.labels.clone(),
            placement: Placement {
                service_id: "svc".into(),
                parent_session_id: PARENT.into(),
                parent_revision: "r1".into(),
                machine_id: "hawk".into(),
                workspace_id: "project".into(),
                cwd: "/workspace".into(),
            },
            child_session_id: format!("{id}-child"),
            state: State::Queued,
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
            input_files: Vec::new(),
            cancel_requested_at_ms: None,
            error: None,
            preset: None,
            selection: None,
        };
        self.store
            .admit_managed_call_in_turn(&record, &request, turn)
            .await
            .unwrap();
    }

    async fn spend(&mut self, calls: usize) -> i64 {
        let mut turn = OWNER_TURN_NONE;
        for _ in 0..calls {
            turn = self.call().await.expect("within the turn's budget");
        }
        turn
    }
}

fn refused(result: Result<i64, Value>) -> (String, String) {
    let refusal = result.expect_err("the budget refuses");
    (
        refusal["error"]["code"].as_str().unwrap().to_owned(),
        refusal["error"]["admission"].as_str().unwrap().to_owned(),
    )
}

#[tokio::test]
async fn a_turn_admits_its_limit_and_refuses_the_next_call_unsubmitted() {
    let mut fixture = Fixture::new(3).await;
    fixture.prompt(Some("cmid-1"));
    fixture.work();
    // A call from before the turn column belongs to no turn and costs none.
    fixture.record(None).await;
    let turn = fixture.spend(3).await;
    assert_eq!(
        refused(fixture.call().await),
        ("call_limit".into(), "not_submitted".into())
    );
    // The refusal recorded nothing: the boundary is the limit exactly.
    assert_eq!(
        fixture
            .store
            .managed_call_turn_count(PARENT, turn)
            .await
            .unwrap(),
        3
    );
}

#[tokio::test]
async fn notices_peer_messages_and_wakeups_never_start_a_turn() {
    let mut fixture = Fixture::new(2).await;
    fixture.prompt(Some("cmid-1"));
    let turn = fixture.spend(2).await;
    let peer = format!("agent-{}", "b".repeat(64));
    for cmid in [
        "call-notice:call-1",
        "managed-call:call-1:round-1",
        "session-notice:peer-1",
        "__wake__parent-1",
        "__sched__parent-draft",
        peer.as_str(),
    ] {
        fixture.prompt(Some(cmid));
        fixture.work();
        assert_eq!(
            refused(fixture.call().await).0,
            "call_limit",
            "{cmid} must not hand out a new budget"
        );
    }
    assert_eq!(
        fixture
            .store
            .managed_call_turn_count(PARENT, turn)
            .await
            .unwrap(),
        2
    );
}

#[tokio::test]
async fn an_owner_prompt_starts_a_fresh_budget_and_keeps_the_old_turn_counted() {
    let mut fixture = Fixture::new(2).await;
    fixture.prompt(Some("cmid-1"));
    let first = fixture.spend(2).await;
    assert_eq!(refused(fixture.call().await).0, "call_limit");
    fixture.prompt(Some("call-notice:call-2"));
    assert_eq!(refused(fixture.call().await).0, "call_limit");
    // The person writes again, as the web composer or an API client does.
    fixture.prompt(Some("cmid-2"));
    let second = fixture.spend(2).await;
    assert_ne!(first, second, "a new owner prompt is a new turn");
    assert_eq!(refused(fixture.call().await).0, "call_limit");
    for turn in [first, second] {
        assert_eq!(
            fixture
                .store
                .managed_call_turn_count(PARENT, turn)
                .await
                .unwrap(),
            2
        );
    }
    // A chunk no send claimed (an ACP client's unmatched text) is no turn.
    fixture.unclaimed_chunk();
    assert_eq!(refused(fixture.call().await).0, "call_limit");
}

#[tokio::test]
async fn a_send_with_a_context_preamble_is_the_owners_turn() {
    let mut fixture = Fixture::new(1).await;
    fixture.prompt(Some("cmid-1"));
    fixture.work();
    let first = fixture.spend(1).await;
    assert_eq!(refused(fixture.call().await).0, "call_limit");
    // Linked context rides ahead of the owner's text: the id and a runtime
    // looking block first, the owner's words without an id second.
    fixture.prompt_text(
        Some("cmid-2"),
        "<system-reminder>linked context</system-reminder>",
    );
    fixture.prompt_text(None, "Continue");
    let second = fixture.spend(1).await;
    assert_ne!(first, second, "the owner's send reset the budget");
    assert_eq!(refused(fixture.call().await).0, "call_limit");
}

#[tokio::test]
async fn a_direct_prompt_without_an_id_is_the_owners_turn() {
    // The authenticated WebSocket `prompt` command sends no client id.
    let mut fixture = Fixture::new(1).await;
    fixture.prompt(Some("cmid-1"));
    fixture.work();
    fixture.spend(1).await;
    assert_eq!(refused(fixture.call().await).0, "call_limit");
    fixture.prompt(None);
    fixture.spend(1).await;
}

#[tokio::test]
async fn one_send_is_one_turn_however_many_blocks_it_has() {
    let mut fixture = Fixture::new(1).await;
    fixture.prompt(Some("cmid-1"));
    let turn = fixture.spend(1).await;
    // The send's second block carries no message id; it is stamped like the
    // first but must not open another budget.
    fixture.prompt(None);
    assert_eq!(refused(fixture.call().await).0, "call_limit");
    // Nor may a peer's steered later block that an older worker left unstamped.
    let peer = format!("agent-{}", "a".repeat(64));
    fixture.prompt(Some(peer.as_str()));
    fixture.hub.push(
        PARENT,
        Event::Update {
            update: json!({
                "sessionUpdate":"user_message_chunk","content":{"type":"text","text":"second block"},
                "_meta":{"cowboyDelivery":"current_turn"},
            }),
        },
    );
    assert_eq!(refused(fixture.call().await).0, "call_limit");
    assert_eq!(
        fixture
            .store
            .managed_call_turn_count(PARENT, turn)
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn clearing_the_transcript_keeps_the_turn() {
    let mut fixture = Fixture::new(2).await;
    fixture.prompt(Some("cmid-1"));
    let turn = fixture.spend(2).await;
    fixture.hub.clear_transcript(PARENT);
    assert_eq!(
        newest_owner_prompt(&fixture.hub.snapshot(PARENT).unwrap().0),
        None,
        "the prompt left the log"
    );
    assert_eq!(refused(fixture.call().await).0, "call_limit");
    assert_eq!(
        fixture
            .store
            .managed_call_turn_count(PARENT, turn)
            .await
            .unwrap(),
        2
    );
}

#[tokio::test]
async fn a_session_no_person_prompted_is_one_turn() {
    let mut fixture = Fixture::new(2).await;
    let peer = format!("agent-{}", "c".repeat(64));
    fixture.prompt(Some(peer.as_str()));
    fixture.work();
    assert_eq!(fixture.spend(2).await, OWNER_TURN_NONE);
    // Nothing a notice or wakeup says reopens it.
    fixture.prompt(Some("__wake__parent-1"));
    assert_eq!(refused(fixture.call().await).0, "call_limit");
}

#[tokio::test]
async fn concurrency_is_checked_before_the_turn_budget() {
    // Both limits are spent by one queued call: the refusal says which one a
    // caller can wait out (a running call ends; a turn only ends with a prompt).
    let mut fixture = Fixture::new(1).await;
    fixture.policy.max_concurrent = 1;
    fixture.prompt(Some("cmid-1"));
    fixture.spend(1).await;
    assert_eq!(
        refused(fixture.call().await),
        ("concurrency_limit".into(), "not_submitted".into())
    );
}

#[tokio::test]
async fn the_budget_follows_a_prompt_the_client_bootstrap_window_no_longer_shows() {
    let mut fixture = Fixture::new(2).await;
    fixture.prompt(Some("cmid-1"));
    let turn = fixture.spend(2).await;
    // One large event pushes the prompt out of the 128 KiB bootstrap tail, as
    // a long tool output does; the Hub still retains it, written or not.
    fixture.hub.push(
        PARENT,
        Event::Update {
            update: json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"x".repeat(140 * 1024)}}),
        },
    );
    assert_eq!(
        newest_owner_prompt(&fixture.hub.snapshot(PARENT).unwrap().0),
        None,
        "the bootstrap tail no longer holds the prompt"
    );
    assert_eq!(refused(fixture.call().await).0, "call_limit");
    assert_eq!(
        fixture
            .store
            .managed_call_turn_count(PARENT, turn)
            .await
            .unwrap(),
        2
    );
}

#[tokio::test]
async fn an_unreadable_turn_refuses_instead_of_guessing() {
    let mut fixture = Fixture::new(2).await;
    let refusal = admit(&fixture.hub, None, PARENT, &fixture.policy)
        .await
        .unwrap_err();
    assert_eq!(refusal["error"]["code"], "capacity");
    let refusal = admit(
        &fixture.hub,
        Some(&fixture.store),
        "missing",
        &fixture.policy,
    )
    .await
    .unwrap_err();
    assert_eq!(refusal["error"]["code"], "capacity");
    assert_eq!(refusal["error"]["reason"], "session_unavailable");
    assert_eq!(refusal["error"]["retryable"], true);
    assert_eq!(fixture.spend(0).await, OWNER_TURN_NONE);
}

/// Durable history is read newest first, a page at a time, until the newest
/// owner prompt: a notice or a peer message among newer rows never stops it.
#[tokio::test]
async fn durable_history_is_paged_until_the_newest_owner_prompt() {
    use crate::core::Envelope;
    let peer = format!("agent-{}", "e".repeat(64));
    for owner_at in [Some(5_u64), None] {
        let fixture = Fixture::new(2).await;
        let rows: Vec<Envelope> = (0..450_u64)
            .map(|seq| {
                let cmid = match seq {
                    5 if owner_at.is_some() => Some("cmid-1"),
                    6 => Some("call-notice:call-1"),
                    400 => Some(peer.as_str()),
                    _ => None,
                };
                let update = match cmid {
                    Some(cmid) => {
                        let mut update = json!({"sessionUpdate":"user_message_chunk","content":{"type":"text","text":format!("prompt {seq}")}});
                        apply_prompt_origin(&mut update, &origin_from_cmid(Some(cmid)));
                        update
                    }
                    None => json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":format!("progress {seq}")}}),
                };
                Envelope {
                    session_id: PARENT.into(),
                    seq,
                    event: Event::Update { update },
                    cmid: cmid.map(str::to_owned),
                }
            })
            .collect();
        fixture
            .store
            .upsert_event_batch(&rows, &std::collections::HashMap::new())
            .await
            .unwrap();
        let end = i64::MAX as u64;
        let expected = owner_at.map_or(OWNER_TURN_NONE, |seq| i64::try_from(seq).unwrap());
        // 450 events are three pages of 200: the prompt is on the last one.
        assert_eq!(
            durable_turn(&fixture.store, PARENT, end).await,
            Ok(expected)
        );
        // Reading only what precedes it finds an earlier owner prompt or none.
        assert_eq!(
            durable_turn(&fixture.store, PARENT, 5).await,
            Ok(OWNER_TURN_NONE)
        );
    }
}
