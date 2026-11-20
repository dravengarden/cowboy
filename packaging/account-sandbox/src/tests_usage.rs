//! `collect_usage` against the fake host: admission like a runtime unit,
//! the fixed command, a bounded run and only the validated projection.

use std::io::Write as _;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{Fixture, code, fixture, fixture_with};
use crate::meta::CredentialState;
use crate::protocol::{Code, Mode, Outcome, Request};

fn collect(f: &Fixture, generation: u64) -> Outcome<Value> {
    f.call(Request::CollectUsage {
        key: f.key.clone(),
        credential_generation: generation,
        runtime_sha256: f.sha.clone(),
    })
}

/// Collect on another thread; the test plays the unit.
fn collect_async(f: &Fixture, generation: u64) -> std::thread::JoinHandle<Outcome<Value>> {
    let (daemon, conn) = (f.daemon.clone(), f.daemon.connect(0));
    let request = Request::CollectUsage {
        key: f.key.clone(),
        credential_generation: generation,
        runtime_sha256: f.sha.clone(),
    };
    std::thread::spawn(move || {
        daemon
            .handle(&conn, request, &mut std::io::empty())
            .map(|reply| reply.value)
    })
}

/// The newest spawned unit of `mode`, once it exists.
fn spawned(f: &Fixture, mode: Mode) -> (String, Vec<String>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let found = f
            .host
            .state()
            .spawned
            .iter()
            .rev()
            .find(|argv| {
                argv.iter().any(|a| {
                    a.starts_with("--unit=") && a.contains(&format!("-{}-", mode.as_str()))
                })
            })
            .cloned();
        if let Some(argv) = found {
            let unit = argv
                .iter()
                .find_map(|a| a.strip_prefix("--unit="))
                .unwrap()
                .to_owned();
            if f.host.unit_status_of(&unit).is_some() {
                return (unit, argv);
            }
        }
        assert!(Instant::now() < deadline, "no {mode:?} unit started");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The unit prints `output` and exits with `exit`.
fn answer(f: &Fixture, unit: &str, output: &[u8], exit: Option<i32>) {
    let stdio = f.host.state().stdio.pop().unwrap();
    let mut stdout = std::fs::File::from(stdio.stdout);
    stdout.write_all(output).unwrap();
    drop(stdout);
    drop(stdio.stderr);
    f.host.finish(unit, exit);
}

fn report() -> Vec<u8> {
    let quota = json!({"groups": [{"name": "Gemini Pro", "buckets": [
        {"name": "Gemini 3 Pro", "remaining_fraction": 0.25, "reset_time": "2026-11-20T13:00:00Z"}]}]});
    let envelope = json!({"status": "SUCCESS", "num_turns": 0,
        "command": {"name": "usage", "data": quota}});
    format!("{envelope}\n").into_bytes()
}

#[test]
fn usage_runs_the_fixed_report_on_the_signed_in_generation() {
    let f = fixture();
    assert_eq!(code(collect(&f, 1)), Code::Precondition);
    f.sign_in(1);
    assert_eq!(code(collect(&f, 0)), Code::Precondition);
    assert_eq!(code(collect(&f, 2)), Code::Precondition);
    let pending = collect_async(&f, 1);
    let (unit, argv) = spawned(&f, Mode::Usage);
    assert!(unit.contains("-usage-"));
    let tail = &argv[argv.len() - 7..];
    assert_eq!(
        tail[1..],
        [
            "-p",
            "/usage",
            "--output-format",
            "json",
            "--log-file",
            "/tmp/agy-usage.log"
        ]
    );
    assert!(tail[0].ends_with("/bin/agy-launch"));
    // No descriptor or execution socket reaches a usage unit.
    assert!(
        !argv
            .iter()
            .any(|a| a.contains("LoadCredential") || a.contains("cowboy-exec.sock"))
    );
    answer(&f, &unit, &report(), Some(0));
    let value = pending.join().unwrap().unwrap();
    assert_eq!(value["outcome"], "available");
    assert_eq!(value["unit"], unit);
    assert_eq!(
        value["quota"],
        json!({"groups": [{"name": "Gemini Pro", "buckets": [
            {"name": "Gemini 3 Pro", "remaining_fraction": 0.25, "reset_at": 1_795_179_600}]}]})
    );
    f.wait_for("usage settled", |meta| meta.units.is_empty());
    // The report never moves the credential.
    assert_eq!(f.meta().credential.state, CredentialState::SignedIn);
    assert_eq!(f.meta().credential.generation, 1);
}

#[test]
fn usage_shares_a_configured_runtime_bound_and_yields_to_authentication() {
    let f = fixture_with(|config| config.max_runtime_units = Some(1));
    f.sign_in(1);
    let runtime = f.start(Mode::Runtime, 1).unwrap()["unit"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(code(collect(&f, 1)), Code::Limit);
    f.host.finish(&runtime, Some(0));
    f.wait_for("runtime settled", |meta| meta.units.is_empty());
    // A logout stops a running report like any runtime unit.
    let pending = collect_async(&f, 1);
    let (unit, _) = spawned(&f, Mode::Usage);
    f.start(Mode::Logout, 1).unwrap();
    assert!(f.host.unit_status_of(&unit).is_none());
    assert!(pending.join().unwrap().is_err());
    // While the authentication unit runs a report is refused.
    assert_eq!(code(collect(&f, 1)), Code::Fenced);
}

#[test]
fn a_fenced_account_reports_nothing() {
    let f = fixture();
    f.sign_in(1);
    f.call(Request::StopAccount {
        key: f.key.clone(),
        revision: None,
        operation_id: None,
    })
    .unwrap();
    assert_eq!(code(collect(&f, 1)), Code::Fenced);
    assert!(
        !f.host
            .state()
            .spawned
            .iter()
            .any(|argv| argv.iter().any(|a| a.contains("-usage-")))
    );
}

#[test]
fn a_failed_or_silent_report_is_stopped_and_refused() {
    let f = fixture();
    f.sign_in(1);
    let pending = collect_async(&f, 1);
    let (unit, _) = spawned(&f, Mode::Usage);
    answer(&f, &unit, b"secret vendor failure\n", Some(1));
    let error = pending.join().unwrap().unwrap_err();
    assert_eq!(error.code, Code::Internal);
    assert!(!error.message.contains("secret"));
    // A report that never ends is stopped at its bound.
    let pending = collect_async(&f, 1);
    let (unit, _) = spawned(&f, Mode::Usage);
    let stdio = f.host.state().stdio.pop().unwrap();
    let error = pending.join().unwrap().unwrap_err();
    drop(stdio);
    assert_eq!(error.code, Code::Internal);
    assert!(f.host.unit_status_of(&unit).is_none());
    f.wait_for("usage settled", |meta| meta.units.is_empty());
}
