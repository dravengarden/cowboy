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
        let mut update = json!({
            "sessionUpdate": "user_message_chunk",
            "content": {"type": "text", "text": "prompt"}
        });
        apply_prompt_origin(&mut update, &origin_from_cmid(cmid));
        self.hub
            .push_tagged(PARENT, Event::Update { update }, cmid.map(str::to_owned));
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
    // A prompt without a client id is the owner's too (an ACP client).
    fixture.prompt(None);
    fixture.spend(1).await;
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
async fn concurrency_is_still_checked_before_the_turn() {
    let mut fixture = Fixture::new(16).await;
    fixture.policy.max_concurrent = 1;
    fixture.prompt(Some("cmid-1"));
    fixture.spend(1).await;
    // The first call is still queued, so it is the one allowed to run.
    assert_eq!(
        refused(fixture.call().await),
        ("concurrency_limit".into(), "not_submitted".into())
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

/// A long turn pushes its owner prompt out of the hot log; the budget finds
/// it in durable history rather than treating the session as unprompted.
#[tokio::test]
async fn the_owner_prompt_behind_a_long_turn_is_found_in_durable_history() {
    use crate::core::Envelope;
    for owner_prompt in [true, false] {
        let mut fixture = Fixture::new(2).await;
        let progress = "progress".repeat(1024);
        let mut rows = Vec::new();
        for n in 0..450 {
            let (event, cmid) = if n == 0 && owner_prompt {
                let mut update = json!({"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"task"}});
                apply_prompt_origin(&mut update, &origin_from_cmid(Some("cmid-1")));
                (Event::Update { update }, Some("cmid-1".to_owned()))
            } else if n == 1 {
                // A notice early in the turn is not the owner's prompt.
                let mut update = json!({"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"notice"}});
                apply_prompt_origin(&mut update, &origin_from_cmid(Some("call-notice:call-1")));
                (
                    Event::Update { update },
                    Some("call-notice:call-1".to_owned()),
                )
            } else {
                (
                    Event::Update {
                        update: json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":&progress}}),
                    },
                    None,
                )
            };
            fixture.hub.push_tagged(PARENT, event.clone(), cmid.clone());
            rows.push(Envelope {
                session_id: PARENT.into(),
                seq: fixture.hub.last_event_seq(PARENT).unwrap(),
                event,
                cmid,
            });
        }
        fixture
            .store
            .upsert_event_batch(&rows, &std::collections::HashMap::new())
            .await
            .unwrap();
        assert_eq!(
            newest_owner_prompt(&fixture.hub.snapshot(PARENT).unwrap().0),
            None,
            "the hot tail no longer holds the prompt"
        );
        let turn = fixture.spend(2).await;
        if owner_prompt {
            assert_eq!(turn, i64::try_from(rows[0].seq).unwrap());
        } else {
            assert_eq!(turn, OWNER_TURN_NONE);
        }
        assert_eq!(refused(fixture.call().await).0, "call_limit");
    }
}
