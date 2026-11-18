//! Account controls against a real SQLite store and a scripted Controller
//! environment: admission at dispatch and revival, parking, the single
//! active turn and its reconciliation, the kill switch ordering, detection
//! and uncertain effects.
use super::reconcile::Observation;
use super::*;
use crate::account_controls::{ReservationState, TurnOutcome};
use crate::remote_runtime::turn_terminal::TurnTerminal;
use crate::runtime_wire::WorkerSnapshot;

const ACCOUNT: &str = "11111111-1111-4111-8111-111111111111";

#[derive(Default)]
struct Fake {
    sessions: parking_lot::Mutex<HashMap<String, (SessionMeta, bool)>>,
    /// The termination ledger's evidence, per session.
    terminations: parking_lot::Mutex<HashMap<String, TargetTermination>>,
    live: parking_lot::Mutex<HashSet<String>>,
    workers: parking_lot::Mutex<HashMap<String, WorkerSnapshot>>,
    connected: parking_lot::Mutex<HashSet<String>>,
    admitted: parking_lot::Mutex<HashSet<String>>,
    settings: parking_lot::Mutex<HashMap<String, Value>>,
    /// Every fence command a Machine received, in order.
    fences: parking_lot::Mutex<Vec<(String, AccountFenceRequest)>>,
    /// Every native logout a Machine received, in order.
    logouts: parking_lot::Mutex<Vec<(String, crate::provider_account::NativeLogoutRequest)>>,
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
    fn native_logout(
        &self,
        machine: String,
        request: crate::provider_account::NativeLogoutRequest,
    ) -> futures::future::BoxFuture<
        'static,
        Result<crate::provider_account::NativeLogoutObservation, String>,
    > {
        self.logouts.lock().push((machine, request.clone()));
        Box::pin(async move {
            Ok(crate::provider_account::NativeLogoutObservation {
                request,
                succeeded: Some(true),
                profile_cleaned: true,
                process_trees_exited: true,
                credential_cleared: Some(true),
                history_retained: Some(true),
            })
        })
    }
    fn termination(
        &self,
        turn: &Reservation,
        _provider: &str,
    ) -> futures::future::BoxFuture<'static, Option<TargetTermination>> {
        let evidence = self.terminations.lock().get(&turn.session_id).cloned();
        Box::pin(std::future::ready(evidence))
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

    fn live(&self, id: &str, epoch: &str) {
        let worker = serde_json::from_value(json!({
            "session_id": id, "worker_epoch": epoch, "generation": "g", "state": "running",
        }))
        .unwrap();
        self.workers.lock().insert(id.into(), worker);
        self.live.lock().insert(id.into());
    }

    fn terminated(&self, id: &str, closed_log_class: Option<&str>) {
        self.terminations.lock().insert(
            id.into(),
            TargetTermination {
                terminated: true,
                closed_log_class: closed_log_class.map(str::to_owned),
            },
        );
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

/// The worker's terminal record of `delivery` in `session`.
async fn end(controls: &Controls, session: &str, delivery: &str, epoch: &str, stop_reason: &str) {
    controls
        .record_terminal(&TurnTerminal {
            session_id: session.into(),
            worker_epoch: epoch.into(),
            cmid: Some(delivery.into()),
            stop_reason: stop_reason.into(),
        })
        .await;
}

/// The proxy's structured refusal or violation, as the worker records it.
fn proxy_error(message: &str, cowboy: &Value) -> String {
    format!(
        "error: {message}: {}",
        serde_json::to_string_pretty(&json!({ "cowboy": cowboy })).unwrap()
    )
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
    parked(controls.before_dispatch("s1", true).await, "not admitted");
    assert!(controls.revival_gate(&meta(&fake, "s1")).is_err());
    fake.admitted.lock().insert(ACCOUNT.into());
    assert!(controls.revival_gate(&meta(&fake, "s1")).is_ok());
    let turn = controls.before_dispatch("s1", true).await.unwrap();
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
    parked(controls.before_dispatch("s1", true).await, "verify");
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
    fake.live("s1", "epoch-1");
    let mut first = controls.before_dispatch("s1", true).await.unwrap().unwrap();
    // The reservation ID is the Controller's, never a caller's value.
    assert!(first.id().starts_with("turn-"));
    // Another session of the account waits, with the reason shown.
    parked(controls.before_dispatch("s2", true).await, "session s1");
    assert!(
        controls
            .parked_reason(&meta(&fake, "s2"))
            .unwrap()
            .contains("s1")
    );
    drop(first.mark("c1").await.unwrap());
    let stored = controls.store.account_turns().await.unwrap();
    assert_eq!(stored[0].state, ReservationState::MayHaveDispatched);
    assert_eq!(
        (
            stored[0].delivery_id.as_str(),
            stored[0].worker_epoch.as_str(),
            stored[0].machine_id.as_str()
        ),
        ("c1", "epoch-1", "ovh")
    );
    drop(first);
    settle(&controls).await;
    // Running: the slot stays until the turn's terminal record.
    fake.set("s1", Status::Busy, true);
    controls.tick().await;
    assert_eq!(controls.store.account_turns().await.unwrap().len(), 1);
    // Unrelated turn ends (another delivery, another worker incarnation,
    // another session) never settle it.
    end(&controls, "s1", "other-prompt", "epoch-1", "EndTurn").await;
    end(&controls, "s1", "c1", "epoch-0", "EndTurn").await;
    end(&controls, "s2", "c1", "epoch-1", "EndTurn").await;
    fake.set("s1", Status::Running, false);
    controls.tick().await;
    assert_eq!(controls.store.account_turns().await.unwrap().len(), 1);
    end(&controls, "s1", "c1", "epoch-1", "EndTurn").await;
    controls.tick().await;
    assert!(controls.store.account_turns().await.unwrap().is_empty());
    assert_eq!(control(&controls).await.state, AccountState::Ok);
    assert!(
        controls
            .before_dispatch("s2", true)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn the_final_check_serializes_with_control_writes() {
    let (fake, controls, _root) = fixture().await;
    // Unadmitted between the reservation and the handoff: nothing is sent.
    let mut turn = controls.before_dispatch("s1", true).await.unwrap().unwrap();
    fake.admitted.lock().clear();
    assert!(turn.mark("c1").await.unwrap_err().contains("not admitted"));
    drop(turn);
    settle(&controls).await;
    assert!(controls.store.account_turns().await.unwrap().is_empty());
    // Blocked between the reservation and the handoff: nothing is sent.
    fake.admitted.lock().insert(ACCOUNT.into());
    let mut turn = controls.before_dispatch("s1", true).await.unwrap().unwrap();
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
    assert!(turn.mark("c1").await.unwrap_err().contains("blocked"));
    drop(turn);
    settle(&controls).await;
    assert!(controls.store.account_turns().await.unwrap().is_empty());
    // A control write waits for a handoff in progress.
    let blocked = control(&controls).await;
    controls
        .enter(
            &blocked,
            AccountState::Ok,
            "owner_unblock",
            None,
            Some(blocked.revision),
            false,
        )
        .await
        .unwrap();
    let mut turn = controls.before_dispatch("s1", true).await.unwrap().unwrap();
    let handoff = turn.mark("c2").await.unwrap();
    let writer = controls.clone();
    let current = control(&controls).await;
    let block = tokio::spawn(async move {
        writer
            .enter(
                &current,
                AccountState::Blocked,
                "owner_block",
                None,
                None,
                false,
            )
            .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert!(!block.is_finished());
    drop(handoff);
    block.await.unwrap().unwrap();
}

#[tokio::test]
async fn a_supervised_follow_up_takes_the_same_gate_and_reservation() {
    let (fake, controls, _root) = fixture().await;
    let sent = std::sync::atomic::AtomicUsize::new(0);
    let send = || {
        sent.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    };
    // Parked account: nothing is handed off.
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
    assert!(controls.handoff("s1", "sa-1", false, send).await.is_err());
    let parked_state = control(&controls).await;
    controls
        .enter(
            &parked_state,
            AccountState::Ok,
            "owner_clear",
            None,
            Some(parked_state.revision),
            false,
        )
        .await
        .unwrap();
    // Another session's turn holds the account: nothing is handed off.
    let mut other = controls.before_dispatch("s2", true).await.unwrap().unwrap();
    drop(other.mark("c2").await.unwrap());
    assert!(controls.handoff("s1", "sa-1", false, send).await.is_err());
    assert_eq!(sent.load(std::sync::atomic::Ordering::SeqCst), 0);
    drop(other);
    settle(&controls).await;
    end(&controls, "s2", "c2", "e", "EndTurn").await;
    controls.tick().await;
    // Admitted: reserved and marked with the follow-up's delivery ID.
    controls.handoff("s1", "sa-1", false, send).await.unwrap();
    assert_eq!(sent.load(std::sync::atomic::Ordering::SeqCst), 1);
    settle(&controls).await;
    let stored = controls.store.account_turns().await.unwrap();
    assert_eq!(
        (
            stored[0].session_id.as_str(),
            stored[0].delivery_id.as_str(),
            stored[0].state
        ),
        ("s1", "sa-1", ReservationState::MayHaveDispatched)
    );
    assert!(fake.session("s1").is_some());
    // A synchronous refusal of the send releases the slot.
    end(&controls, "s1", "sa-1", "e", "EndTurn").await;
    controls.tick().await;
    assert!(
        controls
            .handoff("s1", "sa-2", false, || Err::<(), _>("refused".into()))
            .await
            .is_err()
    );
    settle(&controls).await;
    assert!(controls.store.account_turns().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_dispatch_that_never_enqueued_releases_its_slot() {
    let (_fake, controls, _root) = fixture().await;
    // Dropped before `may_have_dispatched`: not dispatched.
    drop(controls.before_dispatch("s1", true).await.unwrap());
    settle(&controls).await;
    assert!(controls.store.account_turns().await.unwrap().is_empty());
    // Marked, then the enqueue refused it synchronously.
    let mut turn = controls.before_dispatch("s1", true).await.unwrap().unwrap();
    drop(turn.mark("c2").await.unwrap());
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
        delivery_id: String::new(),
        worker_epoch: String::new(),
        machine_id: String::new(),
        after_seq: 5,
        state,
        created_at_ms: updated_at_ms,
        updated_at_ms,
        terminal: None,
    }
}

async fn dispatched(controls: &Controls, turn: &str, at: i64) {
    let key = crate::store::account_controls::TurnKey {
        session_id: "s1",
        turn_id: turn,
        worker_epoch: "e",
        only: None,
    };
    controls
        .store
        .reserve_account_turn(&orphan("s1", turn, ReservationState::Reserved, at))
        .await
        .unwrap();
    assert!(
        controls
            .store
            .mark_account_turn_dispatched(ACCOUNT, &key, ("c", "ovh"), at)
            .await
            .unwrap()
    );
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
    // May have dispatched, its worker gone, nothing proven for long: the
    // slot stays and the account needs the owner. A stopped worker on a
    // connected Machine is not termination evidence.
    dispatched(&controls, "t", 1).await;
    fake.set("s1", Status::Exited, false);
    fake.connected.lock().insert("ovh".into());
    let turns = controls.store.account_turns().await.unwrap();
    assert_eq!(
        controls.observe(&turns[0], 1 + 1_000).await,
        Observation::Running
    );
    controls.tick().await;
    assert_eq!(controls.store.account_turns().await.unwrap().len(), 1);
    assert_eq!(control(&controls).await.state, AccountState::NeedsAttention);
    // The ledger proves the execution target terminated, with no closed-log
    // evidence: released as unknown_refusal, which never lowers the more
    // severe needs_attention.
    fake.terminated("s1", None);
    let turns = controls.store.account_turns().await.unwrap();
    assert_eq!(
        controls.observe(&turns[0], now_ms()).await,
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
    dispatched(&controls, "t2", now_ms()).await;
    controls.tick().await;
    assert!(controls.store.account_turns().await.unwrap().is_empty());
    assert_eq!(control(&controls).await.state, AccountState::UnknownRefusal);
    // A closed log that settled ok makes the loss an interruption.
    let refused = control(&controls).await;
    controls
        .enter(
            &refused,
            AccountState::Ok,
            "owner_clear",
            None,
            Some(refused.revision),
            false,
        )
        .await
        .unwrap();
    fake.terminated("s1", Some("ok"));
    dispatched(&controls, "t3", now_ms()).await;
    controls.tick().await;
    assert!(controls.store.account_turns().await.unwrap().is_empty());
    assert_eq!(control(&controls).await.state, AccountState::Ok);
}

#[tokio::test]
async fn refusals_and_violations_park_the_account_and_stop_its_units() {
    let (fake, controls, _root) = fixture().await;
    fake.connected.lock().insert("ovh".into());
    let mut turn = controls.before_dispatch("s1", true).await.unwrap().unwrap();
    drop(turn.mark("c1").await.unwrap());
    drop(turn);
    settle(&controls).await;
    // Vendor text naming a refusal settles nothing but unknown_refusal.
    end(
        &controls,
        "s1",
        "c1",
        "e",
        "error: upstream: antigravity_backend_refusal:ok",
    )
    .await;
    controls.tick().await;
    assert_eq!(control(&controls).await.state, AccountState::UnknownRefusal);
    let unknown = control(&controls).await;
    controls
        .enter(
            &unknown,
            AccountState::Ok,
            "owner_clear",
            None,
            Some(unknown.revision),
            false,
        )
        .await
        .unwrap();
    let mut turn = controls.before_dispatch("s1", true).await.unwrap().unwrap();
    drop(turn.mark("c2").await.unwrap());
    drop(turn);
    settle(&controls).await;
    end(
        &controls,
        "s1",
        "c2",
        "e",
        &proxy_error(
            "antigravity_backend_refusal:suspended",
            &json!({"antigravity_backend": {"class": "suspended"}}),
        ),
    )
    .await;
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
    // The daemon orders fences by the revision that wrote them.
    assert_eq!(sent[0].1.revision, u64::try_from(suspended.revision).ok());
    parked(controls.before_dispatch("s2", true).await, "suspended");
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
    let admit = fake.fences.lock().last().unwrap().1.clone();
    assert_eq!(admit.action, AccountFenceAction::Admit);
    assert_eq!(admit.revision, u64::try_from(suspended.revision + 1).ok());
    // A built-in tool that executed: needs attention and every unit stops.
    let mut turn = controls.before_dispatch("s1", true).await.unwrap().unwrap();
    drop(turn.mark("c3").await.unwrap());
    drop(turn);
    settle(&controls).await;
    end(
        &controls,
        "s1",
        "c3",
        "e",
        &proxy_error(
            "antigravity_policy_violation:read_file",
            &json!({"policy_violation": "read_file"}),
        ),
    )
    .await;
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
async fn settlement_never_replaces_a_block_written_after_its_observation() {
    let (fake, controls, _root) = fixture().await;
    fake.connected.lock().insert("ovh".into());
    let mut turn = controls.before_dispatch("s1", true).await.unwrap().unwrap();
    drop(turn.mark("c1").await.unwrap());
    drop(turn);
    settle(&controls).await;
    end(
        &controls,
        "s1",
        "c1",
        "e",
        &proxy_error(
            "antigravity_backend_refusal:rate_limited",
            &json!({"antigravity_backend": {"class": "rate_limited"}}),
        ),
    )
    .await;
    // The reconciler's view predates the owner's block.
    let turns = controls.store.account_turns().await.unwrap();
    controls
        .store
        .set_account_state(&StateChange {
            account_id: ACCOUNT,
            owner_user_id: "owner",
            provider_id: "antigravity",
            state: AccountState::Blocked,
            reason: "owner_block",
            until_ms: None,
            expected_revision: Some(0),
            fence: None,
            now_ms: now_ms(),
        })
        .await
        .unwrap()
        .unwrap();
    assert!(controls.cache.control(ACCOUNT).is_none());
    controls.settle_turn(&turns[0]).await;
    assert_eq!(control(&controls).await.state, AccountState::Blocked);
    assert!(controls.store.account_turns().await.unwrap().is_empty());
}

#[tokio::test]
async fn the_kill_switch_fences_durably_before_any_machine_and_gates_force_release() {
    let (fake, controls, _root) = fixture().await;
    let mut turn = controls.before_dispatch("s1", true).await.unwrap().unwrap();
    drop(turn.mark("c1").await.unwrap());
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
    parked(controls.before_dispatch("s2", true).await, "blocked");
    assert!(controls.revival_gate(&meta(&fake, "s2")).is_err());
    // The unsettled turn may be force-released only after a confirmed stop
    // and the ledger's proof that its execution target terminated.
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
    assert_eq!(
        controls.force_release(&blocked, true).await,
        Err("turn_termination_unconfirmed")
    );
    assert_eq!(controls.store.account_turns().await.unwrap().len(), 1);
    fake.terminated("s1", None);
    controls.force_release(&blocked, true).await.unwrap();
    assert!(controls.store.account_turns().await.unwrap().is_empty());
    // Settlement never replaces the owner's block.
    assert_eq!(control(&controls).await.state, AccountState::Blocked);
}

#[tokio::test]
async fn a_failed_machine_lookup_never_writes_an_unfenced_block() {
    let root = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        root.path().join("controls.db").display()
    );
    let store = Store::connect(&url, root.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let fake = Arc::new(Fake::default());
    fake.add("s1", Status::Running);
    let controls = Controls::new(fake, store, Arc::default());
    controls.refresh().await.unwrap();
    // The native-generation lookup fails: the block is refused, not written
    // without the Machines it must stop.
    let mut connection = <sqlx::SqliteConnection as sqlx::Connection>::connect(&url)
        .await
        .unwrap();
    sqlx::query("DROP TABLE provider_native_generations")
        .execute(&mut connection)
        .await
        .unwrap();
    let current = control(&controls).await;
    assert_eq!(
        controls
            .enter(
                &current,
                AccountState::Blocked,
                "owner_block",
                None,
                None,
                false
            )
            .await,
        Err("unavailable")
    );
    assert_eq!(control(&controls).await.state, AccountState::Ok);
}

#[tokio::test]
async fn an_uncertain_effect_parks_the_session_until_acknowledged() {
    let (fake, controls, _root) = fixture().await;
    fake.uncertain("s1", 2);
    parked(controls.before_dispatch("s1", true).await, "fs/writeFile");
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
            .before_dispatch("s1", true)
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
    parked(controls.before_dispatch("s1", false).await, "send again");
    let turn = controls.before_dispatch("s1", true).await.unwrap();
    assert!(turn.is_some());
    assert_eq!(control(&controls).await.state, AccountState::Ok);
}

mod logout_tests;
