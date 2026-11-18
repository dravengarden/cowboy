//! Account controls against a real SQLite store and a scripted Controller
//! environment: admission at dispatch and revival, parking, the single
//! active turn and its reconciliation, the kill switch ordering, detection
//! and uncertain effects.
use super::reconcile::Observation;
use super::*;
use crate::account_controls::TurnOutcome;
use crate::runtime_wire::WorkerSnapshot;

const ACCOUNT: &str = "11111111-1111-4111-8111-111111111111";

#[derive(Default)]
struct Fake {
    sessions: parking_lot::Mutex<HashMap<String, (SessionMeta, bool)>>,
    ends: parking_lot::Mutex<HashMap<String, String>>,
    live: parking_lot::Mutex<HashSet<String>>,
    workers: parking_lot::Mutex<HashMap<String, WorkerSnapshot>>,
    connected: parking_lot::Mutex<HashSet<String>>,
    admitted: parking_lot::Mutex<HashSet<String>>,
    settings: parking_lot::Mutex<HashMap<String, Value>>,
    /// Every fence command a Machine received, in order.
    fences: parking_lot::Mutex<Vec<(String, AccountFenceRequest)>>,
}

impl Env for Fake {
    fn session(&self, id: &str) -> Option<(SessionMeta, bool)> {
        self.sessions.lock().get(id).cloned()
    }
    fn sessions(&self) -> Vec<SessionMeta> {
        self.sessions
            .lock()
            .values()
            .map(|(meta, _)| meta.clone())
            .collect()
    }
    fn turn_end(&self, session: &str, _after_seq: u64) -> Option<String> {
        self.ends.lock().get(session).cloned()
    }
    fn last_event_seq(&self, _session: &str) -> u64 {
        5
    }
    fn live_worker(&self, session: &str) -> bool {
        self.live.lock().contains(session)
    }
    fn worker(&self, session: &str) -> Option<WorkerSnapshot> {
        self.workers.lock().get(session).cloned()
    }
    fn machine_connected(&self, machine: &str) -> bool {
        self.connected.lock().contains(machine)
    }
    fn admitted(&self, account: &ProviderAccountId) -> bool {
        self.admitted.lock().contains(account.as_str())
    }
    fn setting(&self, key: &str) -> Option<Value> {
        self.settings.lock().get(key).cloned()
    }
    fn set_setting(&self, key: String, value: Value) {
        self.settings.lock().insert(key, value);
    }
    fn fence(
        &self,
        machine: String,
        request: AccountFenceRequest,
    ) -> futures::future::BoxFuture<'static, Result<AccountFenceObservation, String>> {
        self.fences.lock().push((machine, request.clone()));
        Box::pin(async move {
            Ok(AccountFenceObservation {
                fenced: request.action == AccountFenceAction::Stop,
                fence_epoch: Some(1),
                stopped: vec!["cga-k-runtime-1.service".into()],
                request,
            })
        })
    }
}

impl Fake {
    fn add(&self, id: &str, status: Status) {
        let hub = Hub::new();
        hub.create_local_session(
            id.into(),
            "antigravity".into(),
            "/tmp".into(),
            "Account controls fixture".into(),
            crate::core::SessionOrigin::Api,
            false,
        );
        let mut meta = hub.session_info(id).unwrap().meta;
        meta.provider_account_id = ACCOUNT.parse().unwrap();
        meta.machine_id = "ovh".into();
        meta.owner_user_id = Some("owner".into());
        meta.status = status;
        self.sessions.lock().insert(id.into(), (meta, false));
    }

    fn set(&self, id: &str, status: Status, in_flight: bool) {
        let mut sessions = self.sessions.lock();
        let entry = sessions.get_mut(id).unwrap();
        entry.0.status = status;
        entry.1 = in_flight;
    }

    fn uncertain(&self, id: &str, sequence: u64) {
        let worker = serde_json::from_value(json!({
            "session_id": id, "worker_epoch": "epoch-1", "generation": "g", "state": "running",
            "uncertain_effect": {"method": "fs/writeFile", "sequence": sequence},
        }))
        .unwrap();
        self.workers.lock().insert(id.into(), worker);
    }
}

async fn fixture() -> (Arc<Fake>, Controls, tempfile::TempDir) {
    let root = tempfile::tempdir().unwrap();
    let store = Store::connect("sqlite::memory:", root.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let fake = Arc::new(Fake::default());
    fake.admitted.lock().insert(ACCOUNT.into());
    fake.add("s1", Status::Running);
    fake.add("s2", Status::Running);
    let controls = Controls::new(fake.clone(), store, Arc::default());
    controls.refresh().await.unwrap();
    (fake, controls, root)
}

fn meta(fake: &Fake, id: &str) -> SessionMeta {
    fake.session(id).unwrap().0
}

/// Let spawned releases and fence deliveries finish.
async fn settle(controls: &Controls) {
    for _ in 0..200 {
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        if controls.cache.dispatching.lock().is_empty() && controls.cache.fencing.lock().is_empty()
        {
            break;
        }
    }
    controls.refresh().await.unwrap();
}

fn parked(result: Result<Option<Turn>, String>, needle: &str) {
    let Err(reason) = result else {
        panic!("expected the dispatch to park on {needle}")
    };
    assert!(
        reason.starts_with(super::super::super::session_maintenance::WAIT_PREFIX)
            && reason.contains(needle),
        "{reason}"
    );
}

async fn control(controls: &Controls) -> Control {
    controls.refresh().await.unwrap();
    controls
        .cache
        .control(ACCOUNT)
        .unwrap_or_else(|| initial(ACCOUNT, "owner", "antigravity"))
}

#[tokio::test]
async fn admission_and_account_state_gate_dispatch_and_revival() {
    let (fake, controls, _root) = fixture().await;
    fake.admitted.lock().clear();
    parked(
        controls.before_dispatch("s1", Some("c1"), true).await,
        "not admitted",
    );
    assert!(controls.revival_gate(&meta(&fake, "s1")).is_err());
    fake.admitted.lock().insert(ACCOUNT.into());
    assert!(controls.revival_gate(&meta(&fake, "s1")).is_ok());
    let turn = controls
        .before_dispatch("s1", Some("c1"), true)
        .await
        .unwrap();
    drop(turn);
    settle(&controls).await;
    let current = control(&controls).await;
    controls
        .enter(
            &current,
            AccountState::VerificationRequired,
            "verification_required",
            None,
            None,
            false,
        )
        .await
        .unwrap();
    parked(
        controls.before_dispatch("s1", Some("c2"), true).await,
        "verify",
    );
    assert!(controls.revival_gate(&meta(&fake, "s1")).is_err());
    assert!(
        controls
            .parked_reason(&meta(&fake, "s2"))
            .unwrap()
            .contains("verify")
    );
    // Other Providers are never governed.
    let mut codex = meta(&fake, "s1");
    codex.provider = "codex".into();
    assert!(controls.parked_reason(&codex).is_none() && controls.revival_gate(&codex).is_ok());
}

#[tokio::test]
async fn one_active_turn_per_account_is_reserved_marked_and_settled() {
    let (fake, controls, _root) = fixture().await;
    let mut first = controls
        .before_dispatch("s1", Some("c1"), true)
        .await
        .unwrap()
        .unwrap();
    // Another session of the account waits, with the reason shown.
    parked(
        controls.before_dispatch("s2", Some("c2"), true).await,
        "session s1",
    );
    assert!(
        controls
            .parked_reason(&meta(&fake, "s2"))
            .unwrap()
            .contains("s1")
    );
    first.mark().await.unwrap();
    let stored = controls.store.account_turns().await.unwrap();
    assert_eq!(stored[0].state, ReservationState::MayHaveDispatched);
    assert_eq!(stored[0].after_seq, 5);
    drop(first);
    settle(&controls).await;
    // Running: the slot stays until the turn's terminal record.
    fake.set("s1", Status::Busy, true);
    controls.tick().await;
    assert_eq!(controls.store.account_turns().await.unwrap().len(), 1);
    fake.ends.lock().insert("s1".into(), "EndTurn".into());
    fake.set("s1", Status::Running, false);
    controls.tick().await;
    assert!(controls.store.account_turns().await.unwrap().is_empty());
    assert_eq!(control(&controls).await.state, AccountState::Ok);
    assert!(
        controls
            .before_dispatch("s2", Some("c2"), true)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn a_dispatch_that_never_enqueued_releases_its_slot() {
    let (_fake, controls, _root) = fixture().await;
    // Dropped before `may_have_dispatched`: not dispatched.
    drop(
        controls
            .before_dispatch("s1", Some("c1"), true)
            .await
            .unwrap(),
    );
    settle(&controls).await;
    assert!(controls.store.account_turns().await.unwrap().is_empty());
    // Marked, then the enqueue refused it synchronously.
    let mut turn = controls
        .before_dispatch("s1", Some("c2"), true)
        .await
        .unwrap()
        .unwrap();
    turn.mark().await.unwrap();
    turn.unsent();
    drop(turn);
    settle(&controls).await;
    assert!(controls.store.account_turns().await.unwrap().is_empty());
}

fn orphan(session: &str, turn: &str, state: ReservationState, updated_at_ms: i64) -> Reservation {
    Reservation {
        account_id: ACCOUNT.into(),
        owner_user_id: "owner".into(),
        session_id: session.into(),
        turn_id: turn.into(),
        worker_epoch: "epoch".into(),
        after_seq: 5,
        state,
        created_at_ms: updated_at_ms,
        updated_at_ms,
    }
}

#[tokio::test]
async fn controller_start_reconciles_reservations_from_evidence_only() {
    let (fake, controls, _root) = fixture().await;
    // An earlier Controller reserved and stopped: not dispatched.
    controls
        .store
        .reserve_account_turn(&orphan("s1", "old", ReservationState::Reserved, 1))
        .await
        .unwrap();
    controls.tick().await;
    assert!(controls.store.account_turns().await.unwrap().is_empty());
    // May have dispatched, Machine offline, nothing proven for long: the
    // slot stays and the account needs the owner.
    let stale = orphan("s1", "t", ReservationState::MayHaveDispatched, 1);
    controls.store.reserve_account_turn(&stale).await.unwrap();
    assert!(
        controls
            .store
            .mark_account_turn_dispatched(ACCOUNT, "t", "e", 1)
            .await
            .unwrap()
    );
    fake.set("s1", Status::Exited, false);
    let turns = controls.store.account_turns().await.unwrap();
    assert_eq!(controls.observe(&turns[0], 1 + 1_000), Observation::Running);
    controls.tick().await;
    assert_eq!(controls.store.account_turns().await.unwrap().len(), 1);
    assert_eq!(control(&controls).await.state, AccountState::NeedsAttention);
    // Proven unit loss (connected Machine reports the worker gone): the
    // slot is released; with no backend evidence it settles unknown_refusal,
    // which never lowers the account's more severe needs_attention.
    fake.connected.lock().insert("ovh".into());
    let turns = controls.store.account_turns().await.unwrap();
    assert_eq!(
        controls.observe(&turns[0], now_ms()),
        Observation::Settle(TurnOutcome::UnitLost {
            evidence_complete: false
        })
    );
    controls.tick().await;
    assert!(controls.store.account_turns().await.unwrap().is_empty());
    assert_eq!(control(&controls).await.state, AccountState::NeedsAttention);
    // From ok, the same loss settles unknown_refusal.
    let attention = control(&controls).await;
    controls
        .enter(
            &attention,
            AccountState::Ok,
            "owner_clear",
            None,
            Some(attention.revision),
            false,
        )
        .await
        .unwrap();
    let lost = orphan("s1", "t2", ReservationState::MayHaveDispatched, now_ms());
    controls.store.reserve_account_turn(&lost).await.unwrap();
    assert!(
        controls
            .store
            .mark_account_turn_dispatched(ACCOUNT, "t2", "e", now_ms())
            .await
            .unwrap()
    );
    controls.tick().await;
    assert!(controls.store.account_turns().await.unwrap().is_empty());
    assert_eq!(control(&controls).await.state, AccountState::UnknownRefusal);
}

#[tokio::test]
async fn refusals_and_violations_park_the_account_and_stop_its_units() {
    let (fake, controls, _root) = fixture().await;
    fake.connected.lock().insert("ovh".into());
    let mut turn = controls
        .before_dispatch("s1", Some("c1"), true)
        .await
        .unwrap()
        .unwrap();
    turn.mark().await.unwrap();
    drop(turn);
    settle(&controls).await;
    fake.ends.lock().insert(
        "s1".into(),
        "error: Internal error: antigravity_backend_refusal:suspended".into(),
    );
    controls.tick().await;
    settle(&controls).await;
    let suspended = control(&controls).await;
    assert_eq!(suspended.state, AccountState::Suspended);
    assert!(controls.store.account_turns().await.unwrap().is_empty());
    assert!(controls.stop_confirmed(ACCOUNT));
    let sent = fake.fences.lock().clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        (sent[0].0.as_str(), sent[0].1.action),
        ("ovh", AccountFenceAction::Stop)
    );
    parked(
        controls.before_dispatch("s2", Some("c2"), true).await,
        "suspended",
    );
    // A later rate limit never lowers the suspension.
    controls
        .settle_state(&suspended, AccountState::RateLimited, "rate_limited", false)
        .await;
    assert_eq!(control(&controls).await.state, AccountState::Suspended);
    // The owner clears it; the units are admitted again.
    controls
        .enter(
            &suspended,
            AccountState::Ok,
            "owner_clear",
            None,
            Some(suspended.revision),
            false,
        )
        .await
        .unwrap();
    settle(&controls).await;
    assert_eq!(
        fake.fences.lock().last().unwrap().1.action,
        AccountFenceAction::Admit
    );
    // A built-in tool that executed: needs attention and every unit stops.
    let mut turn = controls
        .before_dispatch("s1", Some("c3"), true)
        .await
        .unwrap()
        .unwrap();
    turn.mark().await.unwrap();
    drop(turn);
    settle(&controls).await;
    fake.ends.lock().insert(
        "s1".into(),
        "error: antigravity_policy_violation:read_file".into(),
    );
    controls.tick().await;
    settle(&controls).await;
    let flagged = control(&controls).await;
    assert_eq!(
        (flagged.state, flagged.reason.as_str()),
        (AccountState::NeedsAttention, "policy_violation:read_file")
    );
    assert_eq!(
        fake.fences.lock().last().unwrap().1.action,
        AccountFenceAction::Stop
    );
    assert!(controls.stop_confirmed(ACCOUNT));
}

#[tokio::test]
async fn the_kill_switch_fences_durably_before_any_machine_and_gates_force_release() {
    let (fake, controls, _root) = fixture().await;
    let mut turn = controls
        .before_dispatch("s1", Some("c1"), true)
        .await
        .unwrap()
        .unwrap();
    turn.mark().await.unwrap();
    drop(turn);
    settle(&controls).await;
    // Machine offline: the block is durable and parks everything at once.
    let current = control(&controls).await;
    controls
        .enter(
            &current,
            AccountState::Blocked,
            "owner_block",
            None,
            None,
            false,
        )
        .await
        .unwrap();
    settle(&controls).await;
    assert_eq!(control(&controls).await.state, AccountState::Blocked);
    assert!(fake.fences.lock().is_empty());
    let fences = controls.store.account_fences().await.unwrap();
    assert_eq!(fences.len(), 1);
    assert_eq!(
        (fences[0].state, fences[0].action),
        (FenceState::Pending, AccountFenceAction::Stop)
    );
    parked(
        controls.before_dispatch("s2", Some("c2"), true).await,
        "blocked",
    );
    assert!(controls.revival_gate(&meta(&fake, "s2")).is_err());
    // The unsettled turn may be force-released only after a confirmed stop.
    let blocked = control(&controls).await;
    assert_eq!(
        controls.force_release(&blocked, true).await,
        Err("account_stop_unconfirmed")
    );
    fake.connected.lock().insert("ovh".into());
    controls.run_fences();
    settle(&controls).await;
    assert!(controls.stop_confirmed(ACCOUNT));
    assert_eq!(
        controls.force_release(&blocked, false).await,
        Err("confirmation_required")
    );
    controls.force_release(&blocked, true).await.unwrap();
    assert!(controls.store.account_turns().await.unwrap().is_empty());
    // Settlement never replaces the owner's block.
    controls
        .settle_state(
            &blocked,
            AccountState::UnknownRefusal,
            "unknown_refusal",
            false,
        )
        .await;
    assert_eq!(control(&controls).await.state, AccountState::Blocked);
}

#[tokio::test]
async fn an_uncertain_effect_parks_the_session_until_acknowledged() {
    let (fake, controls, _root) = fixture().await;
    fake.uncertain("s1", 2);
    parked(
        controls.before_dispatch("s1", Some("c1"), true).await,
        "fs/writeFile",
    );
    // Other sessions of the account are not parked by it.
    assert!(controls.parked_reason(&meta(&fake, "s2")).is_none());
    // Observed durably: still parked when the worker is gone.
    controls.tick().await;
    fake.workers.lock().clear();
    assert!(
        controls
            .parked_reason(&meta(&fake, "s1"))
            .unwrap()
            .contains("fs/writeFile")
    );
    assert_eq!(
        controls.acknowledge("s1", 1).await.unwrap_err(),
        "uncertain_effect_changed"
    );
    let (key, record) = controls.acknowledge("s1", 2).await.unwrap();
    fake.set_setting(key, record);
    assert!(controls.parked_reason(&meta(&fake, "s1")).is_none());
    assert!(
        controls
            .before_dispatch("s1", Some("c1"), true)
            .await
            .unwrap()
            .is_some()
    );
    // A newer lost reply parks it again.
    fake.uncertain("s1", 3);
    assert!(controls.parked_reason(&meta(&fake, "s1")).is_some());
}

#[tokio::test]
async fn a_rate_limit_resumes_only_on_an_owner_send_after_it_ends() {
    let (_fake, controls, _root) = fixture().await;
    let current = control(&controls).await;
    controls
        .enter(
            &current,
            AccountState::RateLimited,
            "rate_limited",
            Some(now_ms() - 1),
            None,
            false,
        )
        .await
        .unwrap();
    parked(
        controls.before_dispatch("s1", Some("c1"), false).await,
        "send again",
    );
    let turn = controls
        .before_dispatch("s1", Some("c1"), true)
        .await
        .unwrap();
    assert!(turn.is_some());
    assert_eq!(control(&controls).await.state, AccountState::Ok);
}
