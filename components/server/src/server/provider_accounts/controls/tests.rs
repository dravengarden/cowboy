//! Account controls against a real SQLite store and a scripted Controller
//! environment: admission at dispatch and revival, parking, one runtime per
//! account, backend signals, the kill switch ordering and uncertain effects.
use super::*;
use crate::runtime_wire::WorkerSnapshot;

const ACCOUNT: &str = "11111111-1111-4111-8111-111111111111";

#[derive(Default)]
struct Fake {
    sessions: parking_lot::Mutex<HashMap<String, (SessionMeta, bool)>>,
    /// The Machine's current enrollment fingerprint; `None` accepts any.
    enrollment: parking_lot::Mutex<Option<String>>,
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
        recipient_fingerprint: String,
        request: crate::provider_account::NativeLogoutRequest,
    ) -> futures::future::BoxFuture<
        'static,
        Result<crate::provider_account::NativeLogoutObservation, String>,
    > {
        if self
            .enrollment
            .lock()
            .as_ref()
            .is_some_and(|current| *current != recipient_fingerprint)
        {
            return Box::pin(std::future::ready(Err(
                "machine_enrollment_changed".to_owned()
            )));
        }
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

    fn live(&self, id: &str, epoch: &str) {
        let worker = serde_json::from_value(json!({
            "session_id": id, "worker_epoch": epoch, "generation": "g", "state": "running",
        }))
        .unwrap();
        self.workers.lock().insert(id.into(), worker);
        self.live.lock().insert(id.into());
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
        if controls.cache.fencing.lock().is_empty() {
            break;
        }
    }
    controls.refresh().await.unwrap();
}

fn parked(result: Result<(), String>, needle: &str) {
    let Err(reason) = result else {
        panic!("expected the dispatch to park on {needle}")
    };
    assert!(
        reason.starts_with(super::super::super::session_maintenance::WAIT_PREFIX)
            && reason.contains(needle),
        "{reason}"
    );
}

/// A turn ending of `session`, as the runtime reports it.
async fn end(controls: &Controls, fake: &Fake, session: &str, stop_reason: &str) {
    if let Some(transition) = policy::signal_transition(&policy::classify_stop_reason(stop_reason))
    {
        controls.signal(&meta(fake, session), &transition).await;
    }
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
    controls.before_dispatch("s1", true).await.unwrap();
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
async fn one_runtime_per_account_refuses_another_sessions_start() {
    let (fake, controls, _root) = fixture().await;
    controls
        .cache
        .labels
        .write()
        .insert(ACCOUNT.into(), "Work".into());
    fake.live("s1", "epoch-1");
    // The session holding the runtime may keep (re)starting it.
    assert!(controls.revival_gate(&meta(&fake, "s1")).is_ok());
    let refused = controls.revival_gate(&meta(&fake, "s2")).unwrap_err();
    assert!(
        refused.contains("Antigravity \u{b7} Work is in use by another session (s1)")
            && refused.contains("hibernate it first"),
        "{refused}"
    );
    let account: ProviderAccountId = ACCOUNT.parse().unwrap();
    let value = controls.read_value(&[(account, "antigravity".into())]);
    assert_eq!(value["accounts"][0]["in_use_by"], "s1");
    // Hibernated: the other session may start.
    fake.live.lock().clear();
    assert!(controls.revival_gate(&meta(&fake, "s2")).is_ok());
    let value = controls.read_value(&[(ACCOUNT.parse().unwrap(), "antigravity".into())]);
    assert!(value["accounts"][0]["in_use_by"].is_null());
}

#[tokio::test]
async fn backend_signals_set_the_account_state_and_stop_its_units() {
    let (fake, controls, _root) = fixture().await;
    fake.connected.lock().insert("ovh".into());
    // A normal ending changes nothing.
    end(&controls, &fake, "s1", "EndTurn").await;
    assert_eq!(control(&controls).await.state, AccountState::Ok);
    // Vendor text naming a refusal, or any unclassified error, is
    // unknown_refusal.
    end(
        &controls,
        &fake,
        "s1",
        "error: upstream: antigravity_backend_refusal:ok",
    )
    .await;
    let unknown = control(&controls).await;
    assert_eq!(unknown.state, AccountState::UnknownRefusal);
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
    end(
        &controls,
        &fake,
        "s1",
        &proxy_error(
            "antigravity_backend_refusal:suspended",
            &json!({"antigravity_backend": {"class": "suspended"}}),
        ),
    )
    .await;
    settle(&controls).await;
    let suspended = control(&controls).await;
    assert_eq!(suspended.state, AccountState::Suspended);
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
    end(
        &controls,
        &fake,
        "s1",
        &proxy_error(
            "antigravity_policy_violation:read_file",
            &json!({"policy_violation": "read_file"}),
        ),
    )
    .await;
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
    // A rate limit parks until its end without stopping units.
    let flagged = control(&controls).await;
    controls
        .enter(
            &flagged,
            AccountState::Ok,
            "owner_clear",
            None,
            Some(flagged.revision),
            false,
        )
        .await
        .unwrap();
    end(
        &controls,
        &fake,
        "s1",
        &proxy_error(
            "antigravity_backend_refusal:rate_limited",
            &json!({"antigravity_backend": {"class": "rate_limited"}}),
        ),
    )
    .await;
    let limited = control(&controls).await;
    assert_eq!(limited.state, AccountState::RateLimited);
    assert!(limited.until_ms.is_some_and(|until| until > now_ms()));
    parked(controls.before_dispatch("s1", true).await, "until");
}

#[tokio::test]
async fn a_signal_never_replaces_a_block_written_before_it() {
    let (fake, controls, _root) = fixture().await;
    // The Controller's cached view predates the owner's block.
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
    end(
        &controls,
        &fake,
        "s1",
        &proxy_error(
            "antigravity_backend_refusal:rate_limited",
            &json!({"antigravity_backend": {"class": "rate_limited"}}),
        ),
    )
    .await;
    assert_eq!(control(&controls).await.state, AccountState::Blocked);
}

#[tokio::test]
async fn an_older_refresh_never_reopens_a_blocked_account() {
    let (_fake, controls, _root) = fixture().await;
    // A slow load started before the block was written.
    let stale = (
        controls.store.account_controls().await.unwrap(),
        controls.store.account_fences().await.unwrap(),
    );
    let stale_started = controls.cache.loads_started.fetch_add(1, Ordering::AcqRel) + 1;
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
    controls.apply(stale_started, Ok(stale)).unwrap();
    assert_eq!(
        controls.cache.control(ACCOUNT).unwrap().state,
        AccountState::Blocked
    );
    // Even a later-started load never moves a record to a lower revision.
    let mut older = controls.store.account_controls().await.unwrap();
    older[0].state = AccountState::Ok;
    older[0].revision -= 1;
    let started = controls.cache.loads_started.fetch_add(1, Ordering::AcqRel) + 1;
    controls.apply(started, Ok((older, Vec::new()))).unwrap();
    assert_eq!(
        controls.cache.control(ACCOUNT).unwrap().state,
        AccountState::Blocked
    );
}

#[tokio::test]
async fn the_kill_switch_fences_durably_before_any_machine() {
    let (fake, controls, _root) = fixture().await;
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
    fake.connected.lock().insert("ovh".into());
    controls.run_fences();
    settle(&controls).await;
    assert!(controls.stop_confirmed(ACCOUNT));
    assert_eq!(control(&controls).await.state, AccountState::Blocked);
}

#[tokio::test]
async fn a_fence_without_its_originating_revision_is_never_sent() {
    let (fake, controls, _root) = fixture().await;
    fake.connected.lock().insert("ovh".into());
    let fence = Fence {
        account_id: ACCOUNT.into(),
        machine_id: "ovh".into(),
        action: AccountFenceAction::Admit,
        operation_id: "account-fence-unknown".into(),
        state: FenceState::Pending,
        detail: String::new(),
        updated_at_ms: 0,
    };
    controls.deliver(&fence).await;
    assert!(fake.fences.lock().is_empty());
}

#[tokio::test]
async fn fences_go_only_to_machines_holding_the_account_sandbox() {
    let (_fake, controls, _root) = fixture().await;
    // The sessions' runtime Machine; execution targets are never fenced.
    assert_eq!(
        controls.account_machines(ACCOUNT).await.unwrap(),
        vec!["ovh".to_owned()]
    );
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
    controls.before_dispatch("s1", true).await.unwrap();
    // A newer lost reply parks it again, and acknowledging the older one
    // never erases it.
    fake.uncertain("s1", 3);
    assert!(controls.parked_reason(&meta(&fake, "s1")).is_some());
    controls.tick().await;
    assert_eq!(
        controls.acknowledge("s1", 2).await.unwrap_err(),
        "uncertain_effect_changed"
    );
    assert_eq!(
        fake.setting(&uncertain_key("s1")).unwrap()["observed"],
        json!(3)
    );
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
    controls.before_dispatch("s1", true).await.unwrap();
    assert_eq!(control(&controls).await.state, AccountState::Ok);
}

mod logout_tests;
