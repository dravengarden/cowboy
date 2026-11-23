use super::super::current_turn;
use super::*;
use crate::core::Envelope;
use crate::core::{Event, RestoredSession};
use crate::prompt_origin::{apply_prompt_origin, origin_from_cmid};
use serde_json::json;

const CREATOR: &str = "creator";
const CHILD: &str = "child";
const GRANDCHILD: &str = "grandchild";
const PEER: &str = "peer";

struct Fixture {
    hub: Hub,
    store: Store,
    sends: usize,
    _dir: tempfile::TempDir,
}

fn registration(id: &str) -> crate::core::SessionRegistration {
    crate::core::SessionRegistration {
        id: id.into(),
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
        title: id.into(),
        origin: crate::core::SessionOrigin::Web,
        system: false,
        owner_user_id: None,
        owner_username: None,
    }
}

fn chunk(session: &str, seq: u64, cmid: Option<&str>) -> Envelope {
    let mut update =
        json!({"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"go"}});
    apply_prompt_origin(&mut update, &origin_from_cmid(cmid));
    Envelope {
        session_id: session.into(),
        seq,
        event: Event::Update { update },
        cmid: cmid.map(str::to_owned),
    }
}

impl Fixture {
    async fn new() -> Self {
        let hub = Hub::new();
        let dir = tempfile::tempdir().unwrap();
        let store = Store::connect("sqlite::memory:", dir.path().into())
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let fixture = Self {
            hub,
            store,
            sends: 0,
            _dir: dir,
        };
        for id in [CREATOR, CHILD, GRANDCHILD, PEER] {
            fixture.hub.create_session(registration(id));
            fixture
                .store
                .insert_session(&fixture.hub.session_info(id).unwrap().meta)
                .await
                .unwrap();
        }
        fixture
    }

    /// A prompt echoed as the Hub records it, with the origin its cmid selects.
    fn echo(&self, session: &str, cmid: &str) -> u64 {
        let envelope = chunk(session, 0, Some(cmid));
        self.hub.push_tagged(session, envelope.event, envelope.cmid);
        self.hub.last_event_seq(session).unwrap()
    }

    /// An owner prompt and the work it started, so the next one is a new send.
    fn owner_prompt(&mut self, session: &str) -> u64 {
        self.sends += 1;
        let seq = self.echo(session, &format!("cmid-{}", self.sends));
        self.work(session);
        seq
    }

    fn work(&self, session: &str) {
        self.hub.push(
            session,
            Event::Update {
                update: json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"working"}}),
            },
        );
    }

    /// One `queue.submit` from `sender` as the Agent API runs it: read
    /// before the message is sent, granted with its receipt. Returns the
    /// turn a grant would name: the target's newest event after the send.
    async fn send(&mut self, sender: &str, to: &str) -> u64 {
        self.send_with(sender, to, json!({"status":"accepted"}))
            .await
    }

    async fn send_with(&mut self, sender: &str, to: &str, receipt: Value) -> u64 {
        self.sends += 1;
        let cmid = format!("agent-{:064x}", self.sends);
        let grant = prepare(&self.hub, Some(&self.store), sender, to).await;
        if receipt["status"] == "accepted" {
            self.echo(to, &cmid);
            self.work(to);
        }
        if let Some(grant) = grant {
            grant.commit(&self.hub, Some(&self.store), &receipt).await;
        }
        self.hub.last_event_seq(to).unwrap()
    }

    async fn turn(&self, session: &str) -> i64 {
        current_turn(&self.hub, &self.store, session).await.unwrap()
    }
}

fn seq(turn: u64) -> i64 {
    i64::try_from(turn).unwrap()
}

#[tokio::test]
async fn the_creators_first_message_in_each_owner_turn_starts_a_child_turn() {
    let mut fixture = Fixture::new().await;
    record_creator(&fixture.hub, CREATOR, CHILD);
    fixture.owner_prompt(CREATOR);
    assert_eq!(fixture.turn(CHILD).await, OWNER_TURN_NONE);
    let first = fixture.send(CREATOR, CHILD).await;
    assert_eq!(fixture.turn(CHILD).await, seq(first));
    // A second message in the same owner turn of the creator grants nothing,
    // however the two sessions keep messaging each other.
    fixture.send(CREATOR, CHILD).await;
    fixture.send(CHILD, CREATOR).await;
    fixture.send(CREATOR, CHILD).await;
    assert_eq!(fixture.turn(CHILD).await, seq(first));
    // The owner's next prompt to the creator allows one more.
    let creator = fixture.owner_prompt(CREATOR);
    let second = fixture.send(CREATOR, CHILD).await;
    assert!(second > first);
    assert_eq!(fixture.turn(CHILD).await, seq(second));
    fixture.send(CREATOR, CHILD).await;
    assert_eq!(fixture.turn(CHILD).await, seq(second));
    // The child's own owner prompt is still a turn, and the newest wins.
    let owner = fixture.owner_prompt(CHILD);
    assert_eq!(fixture.turn(CHILD).await, seq(owner));
    // The creator never got a turn from its child's messages.
    assert_eq!(fixture.turn(CREATOR).await, seq(creator));
}

#[tokio::test]
async fn a_peer_that_did_not_create_the_session_grants_nothing() {
    let mut fixture = Fixture::new().await;
    record_creator(&fixture.hub, CREATOR, CHILD);
    fixture.owner_prompt(PEER);
    fixture.send(PEER, CHILD).await;
    assert_eq!(fixture.turn(CHILD).await, OWNER_TURN_NONE);
    assert_eq!(fixture.hub.setting(&grants_key(CHILD)), None);
}

#[tokio::test]
async fn without_a_creator_link_the_budget_is_unchanged() {
    let mut fixture = Fixture::new().await;
    fixture.owner_prompt(CREATOR);
    fixture.send(CREATOR, CHILD).await;
    assert_eq!(fixture.turn(CHILD).await, OWNER_TURN_NONE);
    // A session filed beside its creator before the creator was recorded
    // keeps that link.
    fixture.hub.set_setting(
        format!("{SESSION_SPAWNED_FROM_PREFIX}{CHILD}"),
        json!(CREATOR),
    );
    let granted = fixture.send(CREATOR, CHILD).await;
    assert_eq!(fixture.turn(CHILD).await, seq(granted));
}

#[tokio::test]
async fn an_unreadable_record_grants_nothing_and_is_kept() {
    let mut fixture = Fixture::new().await;
    record_creator(&fixture.hub, CREATOR, CHILD);
    let garbled = json!({"creator_turn": 1, "turn": 2, "future": true});
    fixture.hub.set_setting(grants_key(CHILD), garbled.clone());
    fixture.owner_prompt(CREATOR);
    fixture.send(CREATOR, CHILD).await;
    assert_eq!(fixture.turn(CHILD).await, OWNER_TURN_NONE);
    assert_eq!(fixture.hub.setting(&grants_key(CHILD)), Some(garbled));
}

#[tokio::test]
async fn a_creator_no_person_prompted_grants_nothing_and_grants_recurse() {
    let mut fixture = Fixture::new().await;
    record_creator(&fixture.hub, CREATOR, CHILD);
    record_creator(&fixture.hub, CHILD, GRANDCHILD);
    // Only automation drove the creator: it has no turn to grant from, and
    // neither has its child.
    fixture.send(PEER, CREATOR).await;
    fixture.send(CREATOR, CHILD).await;
    fixture.send(CHILD, GRANDCHILD).await;
    assert_eq!(fixture.turn(CHILD).await, OWNER_TURN_NONE);
    assert_eq!(fixture.turn(GRANDCHILD).await, OWNER_TURN_NONE);
    // Once the owner prompts the root, the grant passes down the chain, once
    // per turn at each level.
    fixture.owner_prompt(CREATOR);
    let child = fixture.send(CREATOR, CHILD).await;
    let grandchild = fixture.send(CHILD, GRANDCHILD).await;
    assert_eq!(fixture.turn(GRANDCHILD).await, seq(grandchild));
    fixture.send(CHILD, GRANDCHILD).await;
    fixture.send(CREATOR, CHILD).await;
    fixture.send(CHILD, GRANDCHILD).await;
    assert_eq!(fixture.turn(CHILD).await, seq(child));
    assert_eq!(fixture.turn(GRANDCHILD).await, seq(grandchild));
}

#[tokio::test]
async fn only_a_message_that_was_taken_grants() {
    let mut fixture = Fixture::new().await;
    record_creator(&fixture.hub, CREATOR, CHILD);
    fixture.owner_prompt(CREATOR);
    for receipt in [
        json!({"status":"rejected"}),
        json!({"status":"unknown","error":"invalid_argument"}),
        json!({"status":"unknown","error":"outcome_unknown"}),
        json!({"status":"pending","http_status":202,"result":{"status":"pending"}}),
        json!({"status":"accepted","http_status":200,"result":{"status":"not_delivered"}}),
        json!({"status":"accepted","http_status":202,"result":{"status":"pending"}}),
    ] {
        fixture.send_with(CREATOR, CHILD, receipt).await;
        assert_eq!(fixture.turn(CHILD).await, OWNER_TURN_NONE);
    }
    // An interrupt counts once its delivery says it was submitted.
    let granted = fixture
        .send_with(
            CREATOR,
            CHILD,
            json!({"status":"accepted","http_status":200,"result":{"status":"submitted"}}),
        )
        .await;
    assert_eq!(fixture.turn(CHILD).await, seq(granted));
    // The grant and the creator link it relies on are in the store before it
    // counts, so a restart keeps both.
    let durable = fixture.store.load_settings().await.unwrap();
    assert!(
        durable
            .iter()
            .any(|(key, value)| *key == grants_key(CHILD) && value["turn"] == seq(granted))
    );
    assert!(durable.iter().any(|(key, value)| {
        *key == format!("{SESSION_CREATED_BY_PREFIX}{CHILD}") && value == CREATOR
    }));
}

/// A send that read its creator turn before a newer grant was written (a
/// delayed send) never grants: only a creator turn newer than the recorded
/// one does.
#[tokio::test]
async fn a_send_from_an_older_creator_turn_never_grants_after_a_newer_one() {
    let mut fixture = Fixture::new().await;
    record_creator(&fixture.hub, CREATOR, CHILD);
    fixture.owner_prompt(CREATOR);
    let newer = json!({"creator_turn": 1_000_000, "turn": 2});
    fixture.hub.set_setting(grants_key(CHILD), newer.clone());
    fixture.send(CREATOR, CHILD).await;
    assert_eq!(fixture.hub.setting(&grants_key(CHILD)), Some(newer));
    assert_eq!(fixture.turn(CHILD).await, 2);
}

#[tokio::test]
async fn a_granted_turn_survives_a_cleared_transcript_before_any_call() {
    let mut fixture = Fixture::new().await;
    record_creator(&fixture.hub, CREATOR, CHILD);
    fixture.owner_prompt(CREATOR);
    let granted = fixture.send(CREATOR, CHILD).await;
    fixture.hub.clear_transcript(CHILD);
    assert_eq!(fixture.turn(CHILD).await, seq(granted));
}

fn restore(fixture: &Fixture, grant: Value, log: Vec<Envelope>) -> Hub {
    let restarted = Hub::new();
    restarted.load_settings(vec![(grants_key(CHILD), grant)]);
    restarted.restore_reconciling_runtime(vec![RestoredSession {
        meta: fixture.hub.session_info(CHILD).unwrap().meta,
        log,
        event_count: 500,
        reached_start: false,
        next_seq: 501,
        queue: Vec::new(),
        drafts: Vec::new(),
        config_options: None,
        config_preferences: Default::default(),
        mobile_review_state: serde_json::Value::Null,
        folder_id: None,
        last_event_at_ms: None,
    }]);
    restarted
}

/// After a restart the grant comes back with the settings; an owner prompt
/// newer than it that only durable history still holds is found there.
#[tokio::test]
async fn after_a_restart_the_grant_and_durable_owner_prompts_are_compared() {
    let fixture = Fixture::new().await;
    let rows: Vec<Envelope> = (1..=500_u64)
        .map(|seq| match seq {
            300 | 301 => chunk(CHILD, seq, Some("cmid-1")),
            _ => Envelope {
                session_id: CHILD.into(),
                seq,
                event: Event::Update {
                    update: json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"working"}}),
                },
                cmid: None,
            },
        })
        .collect();
    fixture
        .store
        .upsert_event_batch(&rows, &std::collections::HashMap::new())
        .await
        .unwrap();
    let tail = rows[480..].to_vec();
    // A grant older than the owner prompt: the prompt, two pages back, wins.
    let restarted = restore(
        &fixture,
        json!({"creator_turn": 3, "turn": 100}),
        tail.clone(),
    );
    assert_eq!(
        current_turn(&restarted, &fixture.store, CHILD).await,
        Ok(300)
    );
    // A send the restored tail cut is found whole in durable history, and
    // kept once found, through a cleared transcript.
    let cut = restore(
        &fixture,
        json!({"creator_turn": 3, "turn": 100}),
        rows[300..].to_vec(),
    );
    assert_eq!(current_turn(&cut, &fixture.store, CHILD).await, Ok(300));
    cut.clear_transcript(CHILD);
    assert_eq!(current_turn(&cut, &fixture.store, CHILD).await, Ok(300));
    // A grant newer than all durable history needs no durable read.
    let restarted = restore(&fixture, json!({"creator_turn": 3, "turn": 490}), tail);
    let unreadable = Store::connect("sqlite::memory:", fixture._dir.path().into())
        .await
        .unwrap();
    assert_eq!(current_turn(&restarted, &unreadable, CHILD).await, Ok(490));
}

#[tokio::test]
async fn a_grant_never_outlives_its_deleted_target() {
    let mut fixture = Fixture::new().await;
    record_creator(&fixture.hub, CREATOR, CHILD);
    fixture.owner_prompt(CREATOR);
    let grant = prepare(&fixture.hub, Some(&fixture.store), CREATOR, CHILD)
        .await
        .unwrap();
    fixture.send(PEER, CHILD).await;
    assert!(fixture.hub.delete_session(CHILD));
    grant
        .commit(
            &fixture.hub,
            Some(&fixture.store),
            &json!({"status":"accepted"}),
        )
        .await;
    assert_eq!(fixture.hub.setting(&grants_key(CHILD)), None);
}

#[tokio::test]
async fn the_creator_link_is_written_to_the_store_directly() {
    let fixture = Fixture::new().await;
    persist_creator(Some(&fixture.store), CREATOR, CHILD).await;
    let durable = fixture.store.load_settings().await.unwrap();
    assert!(durable.iter().any(|(key, value)| {
        *key == format!("{SESSION_CREATED_BY_PREFIX}{CHILD}") && value == CREATOR
    }));
}

/// A restarted session cleared before its owner turn was found in durable
/// history refuses calls rather than fall back to an older turn, until its
/// creator grants it a new one.
#[tokio::test]
async fn clearing_an_unresolved_owner_turn_refuses_until_a_new_turn() {
    let mut fixture = Fixture::new().await;
    let rows: Vec<Envelope> = (1..=20_u64)
        .map(|seq| match seq {
            10 | 11 => chunk(CHILD, seq, Some("cmid-1")),
            _ => Envelope {
                session_id: CHILD.into(),
                seq,
                event: Event::Update {
                    update: json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"working"}}),
                },
                cmid: None,
            },
        })
        .collect();
    let restarted = restore(
        &fixture,
        json!({"creator_turn": 3, "turn": 5}),
        rows[10..].to_vec(),
    );
    restarted.clear_transcript(CHILD);
    assert_eq!(
        current_turn(&restarted, &fixture.store, CHILD).await,
        Err("owner_turn_history_cleared")
    );
    // On the live Hub, a grant newer than the clear names a turn again.
    fixture.hub.clear_transcript(CHILD);
    fixture.hub.set_setting(
        format!(
            "{}{CHILD}",
            crate::core::settings_keys::SESSION_OWNER_TURN_LOST_PREFIX
        ),
        json!(fixture.hub.last_event_seq(CHILD).unwrap() + 1),
    );
    assert_eq!(
        current_turn(&fixture.hub, &fixture.store, CHILD).await,
        Err("owner_turn_history_cleared")
    );
    record_creator(&fixture.hub, CREATOR, CHILD);
    fixture.owner_prompt(CREATOR);
    fixture.send(PEER, CHILD).await;
    fixture.work(CHILD);
    let granted = fixture.send(CREATOR, CHILD).await;
    assert_eq!(fixture.turn(CHILD).await, seq(granted));
}

/// History already read to its start without an owner turn loses nothing to
/// a Clear: automation alone keeps driving the session.
#[tokio::test]
async fn clearing_history_confirmed_without_an_owner_turn_keeps_the_session_usable() {
    let fixture = Fixture::new().await;
    let rows: Vec<Envelope> = (1..=20_u64)
        .map(|seq| Envelope {
            session_id: CHILD.into(),
            seq,
            event: Event::Update {
                update: json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"working"}}),
            },
            cmid: None,
        })
        .collect();
    fixture
        .store
        .upsert_event_batch(&rows, &std::collections::HashMap::new())
        .await
        .unwrap();
    let restarted = restore(&fixture, Value::Null, rows[10..].to_vec());
    assert_eq!(
        current_turn(&restarted, &fixture.store, CHILD).await,
        Ok(OWNER_TURN_NONE)
    );
    restarted.clear_transcript(CHILD);
    assert_eq!(
        current_turn(&restarted, &fixture.store, CHILD).await,
        Ok(OWNER_TURN_NONE)
    );
}
