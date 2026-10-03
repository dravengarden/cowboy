//! Opt-in legacy launcher refresh using actual immutable installer bundles.

#![cfg(all(feature = "machine-host", target_os = "linux"))]

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use sha2::{Digest as _, Sha256};

const SERVICE: &str = "svc-0123456789abcdef0123456789abcdef";

fn sha256(path: &Path) -> String {
    let mut file = std::fs::File::open(path).unwrap();
    let mut digest = Sha256::new();
    let mut buffer = [0; 32768];
    loop {
        let size = file.read(&mut buffer).unwrap();
        if size == 0 {
            break;
        }
        digest.update(&buffer[..size]);
    }
    format!("{digest:x}", digest = digest.finalize())
}

fn supplied(variable: &str) -> PathBuf {
    let root = PathBuf::from(std::env::var_os(variable).expect(variable));
    assert_eq!(root.canonicalize().unwrap(), root);
    assert_eq!(root.parent(), Some(Path::new("/nix/store")));
    let source: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("etc/cowboy-release/source.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(source["dirty"], false);
    assert_eq!(source["lane"], "machine");
    assert_eq!(
        source["sessionDeletionJournal"],
        serde_json::json!({"readerSchema": 1, "writerSchema": 0})
    );
    println!(
        "{variable}={} revision={} installer_sha256={}",
        root.display(),
        source["revision"],
        sha256(&root.join("bin/cowboy-machine-install"))
    );
    root
}

fn refresh(
    installer: &Path,
    candidate: &Path,
    home: &Path,
    state: &Path,
    listener: &TcpListener,
) -> Output {
    let listener = listener.try_clone().unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        Instant::now() < deadline,
                        "refresh did not fetch fixture Service identity"
                    );
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("{error}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = [0; 4096];
        let size = stream.read(&mut request).unwrap();
        assert!(
            std::str::from_utf8(&request[..size])
                .unwrap()
                .starts_with("GET /api/machine/service ")
        );
        let body = format!("{{\"service_id\":\"{SERVICE}\"}}");
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    });
    let output = Command::new(installer)
        .env_clear()
        .env("HOME", home)
        .args([
            "--refresh",
            "--no-start",
            "--controller-url",
            &origin,
            "--service-id",
            SERVICE,
            "--workspace",
            "fixture=/no-fixture-workspace",
        ])
        .arg("--state-dir")
        .arg(state)
        .arg("--machine-binary")
        .arg(candidate.join("bin/cowboy-machine"))
        .output()
        .unwrap();
    server.join().unwrap();
    output
}

fn snapshot(state: &Path, launcher: &Path) -> Vec<String> {
    let mut files = vec![launcher.to_path_buf()];
    for name in [
        "identity_ed25519",
        "machine-id",
        "service-origin",
        "enrollment-token",
        "bootstrap/cowboy-machine",
        "bootstrap/cowboy-code-adapter",
        "bootstrap/cowboy-acp-worker",
    ] {
        files.push(state.join(name));
    }
    files.iter().map(|path| sha256(path)).collect()
}

#[test]
#[ignore = "requires independently supplied old/new Machine releases; optional exact installer release"]
fn legacy_launcher_refresh_preserves_or_upgrades_exact_bundles() {
    let old = supplied("COWBOY_TEST_OLD_MACHINE_RELEASE");
    let guarded = supplied("COWBOY_TEST_NEW_MACHINE_RELEASE");
    assert_ne!(old, guarded);
    let installer = if std::env::var_os("COWBOY_TEST_INSTALLER_RELEASE").is_some() {
        supplied("COWBOY_TEST_INSTALLER_RELEASE").join("bin/cowboy-machine-install")
    } else {
        PathBuf::from(env!("CARGO_BIN_EXE_cowboy-machine-install"))
    };
    println!(
        "tested_installer={} sha256={}",
        installer.display(),
        sha256(&installer)
    );
    for singleton in [false, true] {
        for compatible in [false, true] {
            let home = tempfile::tempdir_in("/tmp").unwrap();
            let state = home.path().join("state");
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let origin = format!("http://{}", listener.local_addr().unwrap());
            let output = Command::new(old.join("bin/cowboy-machine-install"))
                .env_clear()
                .env("HOME", home.path())
                .args([
                    "--no-start",
                    "--controller-url",
                    &origin,
                    "--service-id",
                    SERVICE,
                    "--workspace",
                    "fixture=/no-fixture-workspace",
                    "--enrollment-token",
                    "fixture-only",
                ])
                .arg("--state-dir")
                .arg(&state)
                .arg("--machine-binary")
                .arg(old.join("bin/cowboy-machine"))
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            // Enrollment evidence is synthetic; no real Controller is contacted.
            std::fs::write(state.join("identity_ed25519"), "fixture identity").unwrap();
            std::fs::write(state.join("machine-id"), "legacy-fixture").unwrap();
            let original = home
                .path()
                .join(format!(".local/bin/cowboy-machine-launch-{SERVICE}"));
            let launcher = if singleton {
                let legacy = home.path().join(".local/bin/cowboy-machine-launch");
                std::fs::rename(&original, &legacy).unwrap();
                legacy
            } else {
                original
            };
            let script = std::fs::read_to_string(&launcher).unwrap();
            assert!(!script.contains("--check-portable-session-deletion"));
            let before = snapshot(&state, &launcher);
            let output = refresh(
                &installer,
                if compatible { &guarded } else { &old },
                home.path(),
                &state,
                &listener,
            );
            assert_eq!(
                output.status.success(),
                compatible,
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            if !compatible {
                assert!(
                    String::from_utf8_lossy(&output.stderr)
                        .contains("bootstrap must support the portable Session deletion guard")
                );
                assert_eq!(snapshot(&state, &launcher), before);
                println!(
                    "singleton={singleton} legacy candidate refused; installation bytes retained"
                );
                continue;
            }
            let after = snapshot(&state, &launcher);
            assert_eq!(
                &after[1..5],
                &before[1..5],
                "identity, Machine id, origin or token changed"
            );
            assert_eq!(
                sha256(&state.join("bootstrap/cowboy-machine")),
                sha256(&guarded.join("bin/cowboy-machine"))
            );
            assert!(
                std::fs::read_to_string(&launcher)
                    .unwrap()
                    .contains("--check-portable-session-deletion")
            );
            let journal = state.join("session-deletions");
            std::fs::create_dir(&journal).unwrap();
            let record = b"{\"schema\":1,\"owner\":{\"machine_id\":\"legacy-fixture\",\"service_id\":\"svc-0123456789abcdef0123456789abcdef\"},\"deleted\":[\"fixture-terminal\"]}";
            std::fs::write(journal.join("deletions.json"), record).unwrap();
            let output = Command::new("/bin/sh")
                .env_clear()
                .env("HOME", home.path())
                .env("PATH", "/run/current-system/sw/bin")
                .arg(&launcher)
                .output()
                .unwrap();
            assert!(!output.status.success());
            assert!(
                String::from_utf8_lossy(&output.stderr)
                    .contains("portable Session deletion reader admission")
            );
            assert!(!state.join("provider-usage.sqlite3").exists());
            assert_eq!(
                std::fs::read(journal.join("deletions.json")).unwrap(),
                record
            );
            // Negative control: independently invoked old installers remain an
            // unfenced authority. This fixture never starts their replacement.
            let output = refresh(
                &old.join("bin/cowboy-machine-install"),
                &old,
                home.path(),
                &state,
                &listener,
            );
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                !std::fs::read_to_string(&launcher)
                    .unwrap()
                    .contains("--check-portable-session-deletion")
            );
            assert_eq!(
                std::fs::read(journal.join("deletions.json")).unwrap(),
                record
            );
            println!(
                "singleton={singleton} guarded refresh passed; terminal launch refused; old-installer authority remains outside guard"
            );
        }
    }
    println!(
        "legacy refresh matrix passed: two retained legacy candidates, two guarded upgrades, two independent old-installer negative controls"
    );
}
