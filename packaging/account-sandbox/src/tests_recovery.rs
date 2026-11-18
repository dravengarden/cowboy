//! Fence ordering, fence tombstones, stop retries, unit-end confirmation,
//! generation-checked logout cleanup and registration staging bounds.

use super::*;
use crate::ids;

fn fence(action: &str, key: &str, revision: Option<u64>) -> Request {
    let operation_id = revision.map(|revision| format!("{action}-{revision}"));
    match action {
        "stop" => Request::StopAccount {
            key: key.to_owned(),
            revision,
            operation_id,
        },
        _ => Request::AdmitAccount {
            key: key.to_owned(),
            revision,
            operation_id,
        },
    }
}

fn unit_of(value: &Value) -> String {
    value["unit"].as_str().unwrap().to_owned()
}

#[test]
fn a_delayed_admit_cannot_undo_a_newer_stop() {
    let f = fixture();
    let stopped = f.call(fence("stop", &f.key, Some(5))).unwrap();
    assert_eq!(stopped["fenced"], true);
    assert_eq!(
        code(f.call(fence("admit", &f.key, Some(4)))),
        Code::Conflict
    );
    assert_eq!(code(f.call(fence("admit", &f.key, None))), Code::Conflict);
    assert_eq!(code(f.start(Mode::Probe, 0)), Code::Fenced);
    // The same operation repeats idempotently without a new epoch.
    let replay = f.call(fence("stop", &f.key, Some(5))).unwrap();
    assert_eq!(replay["replayed"], true);
    assert_eq!(replay["fence_epoch"], stopped["fence_epoch"]);
    let meta = f.meta();
    assert_eq!(meta.disposition.unwrap().revision, 5);
    f.call(fence("admit", &f.key, Some(6))).unwrap();
    f.start(Mode::Probe, 0).unwrap();
}

#[test]
fn a_stop_before_the_account_exists_fences_its_first_unit() {
    let f = fixture();
    let key = ids::derive_key("svc", "later");
    let stopped = f.call(fence("stop", &key, Some(1))).unwrap();
    assert_eq!(
        (&stopped["fenced"], &stopped["account_present"]),
        (&Value::Bool(true), &Value::Bool(false))
    );
    let ensured = f
        .call(Request::EnsureAccount {
            service_id: "svc".into(),
            account_id: "later".into(),
        })
        .unwrap();
    assert_eq!(ensured["key"], key.as_str());
    let probe = |key: &str| {
        f.call(Request::Start {
            key: key.to_owned(),
            mode: Mode::Probe,
            credential_generation: 0,
            runtime_sha256: f.sha.clone(),
            session_id: None,
            descriptor_b64: None,
        })
    };
    assert_eq!(code(probe(&key)), Code::Fenced);
    assert!(
        !f.config
            .state_root
            .join("fences")
            .join(format!("{key}.json"))
            .exists()
    );
    // The stale admit of an older revision stays refused after adoption.
    assert_eq!(code(f.call(fence("admit", &key, Some(0)))), Code::Conflict);
    f.call(fence("admit", &key, Some(2))).unwrap();
    probe(&key).unwrap();
}

#[test]
fn a_removed_account_keeps_its_fence() {
    let f = fixture();
    f.call(fence("stop", &f.key, Some(3))).unwrap();
    f.call(Request::RemoveAccount {
        key: f.key.clone(),
        operation_id: None,
    })
    .unwrap();
    f.call(Request::EnsureAccount {
        service_id: "svc".into(),
        account_id: "acct".into(),
    })
    .unwrap();
    assert_eq!(code(f.start(Mode::Probe, 0)), Code::Fenced);
    assert_eq!(f.meta().disposition.unwrap().revision, 3);
}

#[test]
fn a_failed_disconnect_stop_is_retried_until_confirmed() {
    let f = fixture();
    let other = f.daemon.connect(0);
    let unit = unit_of(&f.start_on(&other, Mode::Probe, 0).unwrap());
    f.host.stop_failures.store(1, Ordering::Relaxed);
    f.daemon.disconnect(&other);
    assert!(f.host.unit_status_of(&unit).is_some());
    assert_eq!(f.meta().units.len(), 1, "the failed stop keeps its record");
    f.daemon.sweep_units();
    assert!(f.host.unit_status_of(&unit).is_none());
    assert!(f.meta().units.is_empty());
}

#[test]
fn a_unit_is_settled_only_after_systemd_confirms_it_gone() {
    let f = fixture();
    let unit = unit_of(&f.start(Mode::Probe, 0).unwrap());
    f.host.exit_client_only(&unit, Some(0));
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(f.meta().units.len(), 1);
    let state = f.call(Request::UnitState { unit: unit.clone() }).unwrap();
    assert_eq!(state["alive"], true);
    f.daemon.sweep_units();
    assert_eq!(f.meta().units.len(), 1, "systemd still runs it");
    f.host.end_unit(&unit);
    f.daemon.sweep_units();
    assert!(f.meta().units.is_empty());
}

#[test]
fn a_live_unrecorded_unit_is_alive_and_needs_reconciliation() {
    let f = fixture();
    let unit = unit_of(&f.start(Mode::Probe, 0).unwrap());
    let dir = safefs::open_root(&f.account_dir()).unwrap();
    let mut meta = f.meta();
    meta.units.clear();
    meta::save(dir.as_fd(), &meta).unwrap();
    let state = f.call(Request::UnitState { unit: unit.clone() }).unwrap();
    assert_eq!(
        (
            &state["recorded"],
            &state["alive"],
            &state["requires_reconciliation"]
        ),
        (&Value::Bool(false), &Value::Bool(true), &Value::Bool(true))
    );
    f.call(Request::StopUnit { unit: unit.clone() }).unwrap();
    assert!(f.host.unit_status_of(&unit).is_none());
}

#[test]
fn a_delayed_logout_cleanup_never_removes_a_newer_credential() {
    let f = fixture();
    f.sign_in(1);
    let logout = unit_of(&f.start(Mode::Logout, 1).unwrap());
    f.host.finish(&logout, Some(0));
    f.wait_for("logout pending", |meta| {
        meta.credential.state == CredentialState::LogoutPending
    });
    let cleanup = |generation| {
        f.call(Request::LogoutCleanup {
            key: f.key.clone(),
            credential_generation: generation,
            operation_id: Some("logout-op".into()),
        })
    };
    assert_eq!(cleanup(1).unwrap()["token_removed"], true);
    assert_eq!(cleanup(1).unwrap()["token_removed"], false, "idempotent");
    f.sign_in(2);
    let delayed = cleanup(1).unwrap();
    assert_eq!(delayed["superseded"], true);
    assert!(f.token().exists(), "generation 2 keeps its token");
    assert_eq!(f.meta().credential.state, CredentialState::SignedIn);
    assert_eq!(code(cleanup(3)), Code::Precondition);
}

struct Unread;

impl std::io::Read for Unread {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        panic!("the body was read")
    }
}

#[test]
fn registrations_reserve_staging_space_before_their_body() {
    let f = fixture();
    let reserve = f.config.free_space_reserve_bytes;
    f.host.free.store(reserve + 300, Ordering::Relaxed);
    let first = f.daemon.reserve_staging(100).unwrap();
    // A second archive of 100 bytes needs 200 more bytes than are free.
    assert_eq!(
        code(f.daemon.reserve_staging(100).map(drop)),
        Code::InsufficientSpace
    );
    drop(first);
    f.host.free.store(u64::MAX, Ordering::Relaxed);
    let held: Vec<_> = (0..crate::daemon::MAX_CONCURRENT_REGISTRATIONS)
        .map(|_| f.daemon.reserve_staging(1).unwrap())
        .collect();
    assert_eq!(code(f.daemon.reserve_staging(1).map(drop)), Code::Limit);
    drop(held);
    // A refused registration never reads its body.
    f.host.free.store(reserve, Ordering::Relaxed);
    let request = Request::RegisterRuntime {
        sha256: "ab".repeat(32),
        size: 10,
    };
    let refused = f.daemon.handle(&f.conn, request, &mut Unread);
    assert_eq!(
        code(refused.map(|reply| reply.value)),
        Code::InsufficientSpace
    );
}

#[test]
fn runtimes_are_probed_by_the_daemon_owned_identity() {
    let f = fixture();
    let host = f.host.clone();
    // The fake probe unit exits 0 as soon as it is started.
    let finisher = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let unit = host
                .state()
                .units
                .keys()
                .find(|unit| unit.contains("-probe-"))
                .cloned();
            if let Some(unit) = unit {
                host.finish(&unit, Some(0));
                return unit;
            }
            assert!(Instant::now() < deadline, "no probe unit started");
            std::thread::sleep(Duration::from_millis(2));
        }
    });
    let probe = f
        .call(Request::ProbeRuntime {
            runtime_sha256: f.sha.clone(),
        })
        .unwrap();
    let unit = finisher.join().unwrap();
    assert_eq!(
        (&probe["passed"], &probe["exit_code"]),
        (&Value::Bool(true), &Value::from(0))
    );
    let probe_key = ids::derive_key(crate::probe::PROBE_SERVICE, "runtime-probe");
    assert!(unit.starts_with(&format!("cga-{probe_key}-probe-")));
    assert!(f.host.unit_status_of(&unit).is_none());
    // The probe identity is reserved; no caller can ensure or start in it.
    let reserved = crate::protocol::parse_request(
        br#"{"op":"ensure_account","service_id":"cowboy-account-sandbox","account_id":"x"}"#,
    );
    assert_eq!(code(reserved), Code::InvalidRequest);
    // An unknown runtime is refused before any unit starts.
    assert_eq!(
        code(f.call(Request::ProbeRuntime {
            runtime_sha256: "0".repeat(64),
        })),
        Code::NotFound
    );
}
