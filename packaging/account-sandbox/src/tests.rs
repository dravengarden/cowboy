//! State-machine tests of the daemon against the fake host.

use std::os::fd::AsFd as _;
use std::os::unix::fs::{MetadataExt as _, symlink};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::config::Config;
use crate::daemon::{Conn, Daemon};
use crate::fake::FakeHost;
use crate::meta::{self, AccountMeta, CredentialState};
use crate::protocol::{Code, Mode, Outcome, Request};
use crate::safefs;
use crate::store::tests::{AGY, runtime_archive, test_config};

#[path = "tests_recovery.rs"]
mod recovery;

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    config: Config,
    host: Arc<FakeHost>,
    daemon: Arc<Daemon>,
    conn: Conn,
    key: String,
    sha: String,
}

fn fixture() -> Fixture {
    fixture_with(|_| {})
}

/// A fixture whose configuration `adjust` changes first.
fn fixture_with(adjust: impl FnOnce(&mut Config)) -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().to_owned();
    let mut config = test_config(&root);
    config.stop_timeout_secs = 2;
    adjust(&mut config);
    let host = FakeHost::new();
    let daemon = Daemon::new(config.clone(), host.clone(), false).unwrap();
    daemon.reconcile().unwrap();
    let conn = daemon.connect(rustix::process::getuid().as_raw());
    let reply = call(
        &daemon,
        &conn,
        Request::EnsureAccount {
            service_id: "svc".into(),
            account_id: "acct".into(),
        },
    )
    .unwrap();
    let key = reply["key"].as_str().unwrap().to_owned();
    let (bytes, sha) = runtime_archive(AGY);
    let request = Request::RegisterRuntime {
        sha256: sha.clone(),
        size: bytes.len() as u64,
    };
    daemon
        .handle(&conn, request, &mut &bytes[..])
        .map_err(|f| f.to_string())
        .unwrap();
    Fixture {
        _temp: temp,
        root,
        config,
        host,
        daemon,
        conn,
        key,
        sha,
    }
}

fn call(daemon: &Arc<Daemon>, conn: &Conn, request: Request) -> Outcome<Value> {
    daemon
        .handle(conn, request, &mut std::io::empty())
        .map(|reply| reply.value)
}

impl Fixture {
    fn account_dir(&self) -> PathBuf {
        self.config.state_root.join("accounts").join(&self.key)
    }

    fn home(&self) -> PathBuf {
        self.account_dir().join("home")
    }

    fn token(&self) -> PathBuf {
        self.home()
            .join(".gemini/antigravity-cli/antigravity-oauth-token")
    }

    fn meta(&self) -> AccountMeta {
        let dir = safefs::open_root(&self.account_dir()).unwrap();
        meta::load(dir.as_fd()).unwrap().unwrap()
    }

    fn start(&self, mode: Mode, generation: u64) -> Outcome<Value> {
        self.start_on(&self.conn, mode, generation)
    }

    fn start_on(&self, conn: &Conn, mode: Mode, generation: u64) -> Outcome<Value> {
        let request = Request::Start {
            key: self.key.clone(),
            mode,
            credential_generation: generation,
            runtime_sha256: self.sha.clone(),
            session_id: Some("session-1".into()),
            descriptor_b64: Some("eyJ4IjoxfQ==".into()),
        };
        self.daemon
            .handle(conn, request, &mut std::io::empty())
            .map(|reply| {
                assert!(reply.fds.len() >= 3);
                reply.value
            })
    }

    fn call(&self, request: Request) -> Outcome<Value> {
        call(&self.daemon, &self.conn, request)
    }

    fn wait_for(&self, what: &str, condition: impl Fn(&AccountMeta) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !condition(&self.meta()) {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Sign in at `generation`: a login unit that writes the token and exits 0.
    fn sign_in(&self, generation: u64) {
        let unit = self.start(Mode::Login, generation).unwrap()["unit"]
            .as_str()
            .unwrap()
            .to_owned();
        std::fs::write(self.token(), b"token").unwrap();
        self.host.finish(&unit, Some(0));
        self.wait_for("sign-in", |meta| {
            meta.credential.state == CredentialState::SignedIn
        });
    }
}

fn code<T: std::fmt::Debug>(outcome: Outcome<T>) -> Code {
    outcome.unwrap_err().code
}

fn mode_of(path: &Path) -> u32 {
    std::fs::symlink_metadata(path).unwrap().mode() & 0o7777
}

#[test]
fn ensure_account_builds_the_layout_idempotently() {
    let f = fixture();
    assert_eq!(f.key, crate::ids::derive_key("svc", "acct"));
    let again = f
        .call(Request::EnsureAccount {
            service_id: "svc".into(),
            account_id: "acct".into(),
        })
        .unwrap();
    assert_eq!(again["user"], format!("cga-{}", f.key));
    assert_eq!(mode_of(&f.config.state_root), 0o711);
    assert_eq!(mode_of(&f.config.state_root.join("accounts")), 0o711);
    assert_eq!(mode_of(&f.account_dir()), 0o711);
    assert_eq!(mode_of(&f.account_dir().join("meta.json")), 0o600);
    assert_eq!(mode_of(&f.account_dir().join("home.img")), 0o600);
    assert_eq!(
        std::fs::metadata(f.account_dir().join("home.img"))
            .unwrap()
            .len(),
        f.config.home_image_bytes
    );
    assert_eq!(mode_of(&f.home()), 0o700);
    assert_eq!(mode_of(&f.home().join(".gemini/antigravity-cli")), 0o700);
    assert!(
        f.home()
            .join(".gemini/antigravity-cli/settings.json")
            .is_file()
    );
    assert_eq!(mode_of(&f.account_dir().join("gemini")), 0o755);
    assert!(f.account_dir().join("gemini/config/hooks.json").is_file());
    assert!(f.host.state().mounted.contains(&f.home()));
    let meta = f.meta();
    assert!(meta.image_ready);
    assert_eq!(meta.credential.state, CredentialState::SignedOut);
}

#[test]
fn runtime_needs_the_current_signed_in_generation() {
    let f = fixture();
    assert_eq!(code(f.start(Mode::Runtime, 0)), Code::Precondition);
    f.sign_in(1);
    let reply = f.start(Mode::Runtime, 1).unwrap();
    assert_eq!(reply["boot_id"], "boot-1");
    assert!(reply["invocation_id"].is_string());
    assert_eq!(
        reply["fds"],
        serde_json::json!(["stdin", "stdout", "stderr", "exec_listener"])
    );
    assert_eq!(code(f.start(Mode::Runtime, 0)), Code::Precondition);
    assert_eq!(code(f.start(Mode::Runtime, 2)), Code::Precondition);
    let record = f
        .meta()
        .units
        .into_iter()
        .find(|r| r.mode == Mode::Runtime)
        .unwrap();
    assert_eq!(
        record.invocation_id.as_deref(),
        reply["invocation_id"].as_str()
    );
    assert_eq!(record.connection, f.conn.id);
    assert_eq!(record.credential_generation, 1);
    let unit_dir = f.config.run_dir.join("units").join(&record.nonce);
    assert_eq!(
        std::fs::read(unit_dir.join("descriptor")).unwrap(),
        br#"{"x":1}"#
    );
    assert_eq!(mode_of(&unit_dir.join("descriptor")), 0o600);
    assert_eq!(mode_of(&unit_dir.join("exec.sock")), 0o660);
}

#[test]
fn one_runtime_per_account_by_default() {
    let f = fixture();
    assert_eq!(f.config.max_runtime_units, 1);
    f.sign_in(1);
    f.start(Mode::Runtime, 1).unwrap();
    // A second session of the account is refused: `limit`.
    assert_eq!(code(f.start(Mode::Runtime, 1)), Code::Limit);
    assert_eq!(code(f.start(Mode::Probe, 1)), Code::Limit);
}

#[test]
fn login_requires_an_absent_token_and_a_new_generation() {
    let f = fixture();
    assert_eq!(code(f.start(Mode::Login, 0)), Code::Precondition);
    f.sign_in(1);
    assert_eq!(
        code(f.start(Mode::Login, 2)),
        Code::Precondition,
        "token present"
    );
    std::fs::remove_file(f.token()).unwrap();
    assert_eq!(
        code(f.start(Mode::Login, 1)),
        Code::Precondition,
        "reused generation"
    );
    let unit = f.start(Mode::Login, 2).unwrap()["unit"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        code(f.start(Mode::Login, 3)),
        Code::Limit,
        "one authentication unit"
    );
    assert_eq!(
        code(f.start(Mode::Runtime, 2)),
        Code::Fenced,
        "authentication holds the account"
    );
    f.host.finish(&unit, Some(0));
    f.wait_for("interrupted login", |meta| {
        meta.credential.state == CredentialState::LoginInterrupted
    });
}

#[test]
fn interrupted_login_removes_only_its_partial_token() {
    let f = fixture();
    let unit = f.start(Mode::Login, 1).unwrap()["unit"]
        .as_str()
        .unwrap()
        .to_owned();
    std::fs::write(f.token(), b"partial").unwrap();
    std::fs::write(f.home().join("history"), b"kept").unwrap();
    f.host.finish(&unit, Some(1));
    f.wait_for("interrupted login", |meta| {
        meta.credential.state == CredentialState::LoginInterrupted
    });
    assert!(!f.token().exists());
    assert!(f.home().join("history").exists());
    assert!(f.meta().cleanups.is_empty());
    assert_eq!(f.meta().credential.generation, 1);
}

#[test]
fn authentication_transitions_stop_runtime_units_first() {
    // Several units of one account (a raised `max_runtime_units`).
    let f = fixture_with(|config| config.max_runtime_units = 4);
    f.sign_in(1);
    let a = f.start(Mode::Runtime, 1).unwrap()["unit"]
        .as_str()
        .unwrap()
        .to_owned();
    let b = f.start(Mode::Runtime, 1).unwrap()["unit"]
        .as_str()
        .unwrap()
        .to_owned();
    let logout = f.start(Mode::Logout, 1).unwrap()["unit"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(f.host.unit_status_of(&a).is_none() && f.host.unit_status_of(&b).is_none());
    let meta = f.meta();
    assert_eq!(meta.units.len(), 1);
    assert_eq!(meta.credential.state, CredentialState::LogoutRunning);
    assert_eq!(
        code(f.call(Request::LogoutCleanup {
            key: f.key.clone(),
            credential_generation: 1,
            operation_id: None
        })),
        Code::Conflict
    );
    f.host.finish(&logout, Some(0));
    f.wait_for("logout pending", |meta| {
        meta.credential.state == CredentialState::LogoutPending
    });
    std::fs::write(
        f.home().join(".gemini/antigravity-cli/conversations"),
        b"kept",
    )
    .unwrap();
    let cleanup = f
        .call(Request::LogoutCleanup {
            key: f.key.clone(),
            credential_generation: 1,
            operation_id: None,
        })
        .unwrap();
    assert_eq!(cleanup["token_removed"], true);
    assert_eq!(cleanup["credential_cleared"], true);
    assert_eq!(cleanup["history_retained"], true);
    assert!(!f.token().exists());
    assert!(
        f.home()
            .join(".gemini/antigravity-cli/conversations")
            .exists()
    );
    let meta = f.meta();
    assert_eq!(meta.credential.state, CredentialState::LoggedOut);
    assert!(meta.cleanups.is_empty());
    assert_eq!(meta.last_logout.unwrap().generation, 1);
    assert_eq!(code(f.start(Mode::Runtime, 1)), Code::Precondition);
    f.sign_in(2);
    assert_eq!(
        code(f.start(Mode::Runtime, 1)),
        Code::Precondition,
        "older credential generation"
    );
    f.start(Mode::Runtime, 2).unwrap();
}

#[test]
fn the_fence_blocks_starts_until_admitted() {
    let f = fixture();
    f.sign_in(1);
    let unit = f.start(Mode::Runtime, 1).unwrap()["unit"]
        .as_str()
        .unwrap()
        .to_owned();
    let stopped = f
        .call(Request::StopAccount {
            key: f.key.clone(),
            revision: None,
            operation_id: None,
        })
        .unwrap();
    assert_eq!(stopped["fence_epoch"], 1);
    assert_eq!(stopped["stopped"], serde_json::json!([unit]));
    assert!(f.meta().units.is_empty());
    assert_eq!(f.meta().fenced, Some(1));
    assert_eq!(code(f.start(Mode::Runtime, 1)), Code::Fenced);
    assert_eq!(code(f.start(Mode::Probe, 1)), Code::Fenced);
    assert_eq!(code(f.start(Mode::Login, 2)), Code::Fenced);
    // Logout still runs while fenced.
    let logout = f.start(Mode::Logout, 1).unwrap()["unit"]
        .as_str()
        .unwrap()
        .to_owned();
    f.host.finish(&logout, Some(0));
    f.wait_for("logout pending", |meta| {
        meta.credential.state == CredentialState::LogoutPending
    });
    assert_eq!(
        f.meta().fenced,
        Some(1),
        "the auth transition keeps the owner fence"
    );
    f.call(Request::AdmitAccount {
        key: f.key.clone(),
        revision: None,
        operation_id: None,
    })
    .unwrap();
    assert_eq!(f.meta().fenced, None);
    assert_eq!(code(f.start(Mode::Runtime, 1)), Code::Precondition);
    f.start(Mode::Probe, 1).unwrap();
}

#[test]
fn closing_the_connection_stops_its_units() {
    // Several units of one account (a raised `max_runtime_units`).
    let f = fixture_with(|config| config.max_runtime_units = 4);
    f.sign_in(1);
    let other = f.daemon.connect(0);
    let mine = f.start_on(&other, Mode::Runtime, 1).unwrap()["unit"]
        .as_str()
        .unwrap()
        .to_owned();
    let kept = f.start(Mode::Runtime, 1).unwrap()["unit"]
        .as_str()
        .unwrap()
        .to_owned();
    let state = f.call(Request::UnitState { unit: mine.clone() }).unwrap();
    assert_eq!(
        (state["alive"].as_bool(), state["owned"].as_bool()),
        (Some(true), Some(true))
    );
    f.daemon.disconnect(&other);
    assert!(f.host.unit_status_of(&mine).is_none());
    assert!(f.host.unit_status_of(&kept).is_some());
    let units: Vec<String> = f.meta().units.into_iter().map(|r| r.unit).collect();
    assert_eq!(units, [kept]);
    let state = f.call(Request::UnitState { unit: mine }).unwrap();
    assert_eq!(state["recorded"], false);
    assert_eq!(state["state"], "gone");
}

#[test]
fn units_that_exit_are_settled() {
    let f = fixture();
    let probe = f.start(Mode::Probe, 0).unwrap()["unit"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(f.meta().units.len(), 1);
    f.host.finish(&probe, Some(0));
    f.wait_for("probe settled", |meta| meta.units.is_empty());
    let listed = f.call(Request::ListUnits { key: f.key.clone() }).unwrap();
    assert_eq!(listed["units"], serde_json::json!([]));
}

#[test]
fn starts_refuse_below_the_free_space_reserve() {
    let f = fixture();
    f.host
        .free
        .store(f.config.free_space_reserve_bytes - 1, Ordering::Relaxed);
    assert_eq!(code(f.start(Mode::Probe, 0)), Code::InsufficientSpace);
    let other = f.call(Request::EnsureAccount {
        service_id: "svc".into(),
        account_id: "other".into(),
    });
    assert_eq!(code(other), Code::InsufficientSpace);
}

#[test]
fn unknown_runtimes_and_accounts_are_refused() {
    let f = fixture();
    let request = Request::Start {
        key: f.key.clone(),
        mode: Mode::Probe,
        credential_generation: 0,
        runtime_sha256: "0".repeat(64),
        session_id: None,
        descriptor_b64: None,
    };
    assert_eq!(
        code(
            f.daemon
                .handle(&f.conn, request, &mut std::io::empty())
                .map(|r| r.value)
        ),
        Code::NotFound
    );
    // A runtime whose agy was removed from the allow-list no longer starts.
    std::fs::write(&f.config.agy_allow_list, "").unwrap();
    assert_eq!(code(f.start(Mode::Probe, 0)), Code::RuntimeRejected);
}

#[test]
fn remove_account_requires_fence_and_logout() {
    let f = fixture();
    f.sign_in(1);
    assert_eq!(
        code(f.call(Request::RemoveAccount {
            key: f.key.clone(),
            operation_id: None,
        })),
        Code::Precondition
    );
    f.call(Request::StopAccount {
        key: f.key.clone(),
        revision: None,
        operation_id: None,
    })
    .unwrap();
    assert_eq!(
        code(f.call(Request::RemoveAccount {
            key: f.key.clone(),
            operation_id: None,
        })),
        Code::Precondition
    );
    f.call(Request::LogoutCleanup {
        key: f.key.clone(),
        credential_generation: 1,
        operation_id: None,
    })
    .unwrap();
    f.call(Request::RemoveAccount {
        key: f.key.clone(),
        operation_id: None,
    })
    .unwrap();
    assert!(!f.account_dir().exists());
    assert!(!f.host.state().mounted.contains(&f.home()));
    assert_eq!(f.host.state().removed_users, [format!("cga-{}", f.key)]);
}

#[test]
fn a_removal_is_idempotent_per_operation_and_finishes_after_a_crash() {
    let f = fixture();
    f.sign_in(1);
    f.call(Request::StopAccount {
        key: f.key.clone(),
        revision: None,
        operation_id: None,
    })
    .unwrap();
    f.call(Request::LogoutCleanup {
        key: f.key.clone(),
        credential_generation: 1,
        operation_id: None,
    })
    .unwrap();
    let remove = |operation: &str| {
        f.call(Request::RemoveAccount {
            key: f.key.clone(),
            operation_id: Some(operation.into()),
        })
    };
    let first = remove("revoke-1").unwrap();
    assert_eq!(
        (
            first["home_removed"].as_bool(),
            first["user_removed"].as_bool()
        ),
        (Some(true), Some(true))
    );
    assert_eq!(first["replayed"], false);
    // A retry after a lost reply is the same removal's replay.
    let again = remove("revoke-1").unwrap();
    assert_eq!(again["replayed"], true);
    assert_eq!(again["operation_id"], "revoke-1");
    // A crash after the tree went but before the user did: the retry
    // removes the user it recorded.
    let user = format!("cga-{}", f.key);
    f.host.state().users.insert(user.clone(), (4242, 4242));
    let finished = remove("revoke-1").unwrap();
    assert_eq!(finished["user_removed"], true);
    assert!(!f.host.state().users.contains_key(&user));
    // The fence survives the removal, so a recreated account stays fenced.
    f.call(Request::EnsureAccount {
        service_id: "svc".into(),
        account_id: "acct".into(),
    })
    .unwrap();
    assert!(f.meta().fenced.is_some());
}

#[test]
fn token_cleanup_does_not_follow_symlinks() {
    let f = fixture();
    let outside = f.root.join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("antigravity-oauth-token"), b"victim").unwrap();
    let state = f.home().join(".gemini/antigravity-cli");
    std::fs::remove_dir_all(&state).unwrap();
    symlink(&outside, &state).unwrap();
    assert_eq!(
        code(f.call(Request::LogoutCleanup {
            key: f.key.clone(),
            credential_generation: 0,
            operation_id: None
        })),
        Code::Internal
    );
    assert_eq!(
        std::fs::read(outside.join("antigravity-oauth-token")).unwrap(),
        b"victim"
    );
    // A start refuses to prepare a home whose state directory is a symlink.
    assert!(f.start(Mode::Probe, 0).is_err());
}

#[test]
fn reconciliation_stops_units_of_a_previous_instance() {
    let f = fixture();
    f.sign_in(1);
    let runtime = f.start(Mode::Runtime, 1).unwrap()["unit"]
        .as_str()
        .unwrap()
        .to_owned();
    let login_key = f.key.clone();
    // A new daemon instance (a restart) over the same state.
    let restarted = Daemon::new(f.config.clone(), f.host.clone(), false).unwrap();
    restarted.reconcile().unwrap();
    assert!(f.host.unit_status_of(&runtime).is_none());
    assert!(f.meta().units.is_empty());
    assert_eq!(f.meta().credential.state, CredentialState::SignedIn);
    assert_eq!(f.meta().key, login_key);
}

#[test]
fn the_unit_command_line_is_fixed() {
    let f = fixture();
    f.start(Mode::Probe, 0).unwrap();
    let argv = f.host.state().spawned.last().unwrap().clone();
    let runtime = f.config.state_root.join("runtime").join(&f.sha);
    let expected = [
        "--".to_owned(),
        f.config.exec_path.display().to_string(),
        "exec-confined".to_owned(),
        "--".to_owned(),
        format!("{}/bin/agy-launch", runtime.display()),
        "--version".to_owned(),
    ];
    assert_eq!(argv[argv.len() - 6..], expected);
    assert!(argv.contains(&format!("User=cga-{}", f.key)));
    assert!(
        !argv.iter().any(|a| a.contains("session-1")),
        "caller strings never reach argv"
    );
}

/// Read one response line, collecting descriptors passed with it.
fn receive(stream: &std::os::unix::net::UnixStream) -> (Value, Vec<std::os::fd::OwnedFd>) {
    use rustix::net::{RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags};
    use std::mem::MaybeUninit;
    let mut line = Vec::new();
    let mut fds = Vec::new();
    loop {
        let mut byte = [0_u8; 1];
        let mut space = [MaybeUninit::<u8>::uninit(); rustix::cmsg_space!(ScmRights(8))];
        let mut control = RecvAncillaryBuffer::new(&mut space);
        let received = rustix::net::recvmsg(
            stream,
            &mut [std::io::IoSliceMut::new(&mut byte)],
            &mut control,
            RecvFlags::CMSG_CLOEXEC,
        )
        .unwrap();
        for message in control.drain() {
            if let RecvAncillaryMessage::ScmRights(received) = message {
                fds.extend(received);
            }
        }
        assert_eq!(received.bytes, 1, "connection closed mid-line");
        if byte[0] == b'\n' {
            return (serde_json::from_slice(&line).unwrap(), fds);
        }
        line.push(byte[0]);
    }
}

#[test]
fn the_socket_protocol_passes_pipes_and_stops_units_on_close() {
    use std::io::{Read as _, Write as _};
    let f = fixture();
    let (mut client, server) = std::os::unix::net::UnixStream::pair().unwrap();
    let daemon = Arc::clone(&f.daemon);
    let worker = std::thread::spawn(move || crate::server::handle_connection(&daemon, &server));
    client
        .write_all(b"{\"op\":\"stop_account\",\"key\":\"zz\"}\n")
        .unwrap();
    let (reply, _) = receive(&client);
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["error"]["code"], "invalid_request");
    let start = format!(
        "{{\"op\":\"start\",\"key\":\"{}\",\"mode\":\"probe\",\"credential_generation\":0,\"runtime_sha256\":\"{}\"}}\n",
        f.key, f.sha
    );
    client.write_all(start.as_bytes()).unwrap();
    let (reply, fds) = receive(&client);
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(fds.len(), 3);
    let unit = reply["result"]["unit"].as_str().unwrap().to_owned();
    // The unit side of stdout writes; the caller's end reads it.
    let unit_stdout = f
        .host
        .state()
        .stdio
        .last()
        .unwrap()
        .stdout
        .try_clone()
        .unwrap();
    std::fs::File::from(unit_stdout)
        .write_all(b"agy 1.5.9\n")
        .unwrap();
    f.host.state().stdio.clear();
    let mut output = String::new();
    std::fs::File::from(fds.into_iter().nth(1).unwrap())
        .read_to_string(&mut output)
        .unwrap();
    assert_eq!(output, "agy 1.5.9\n");
    assert!(f.host.unit_status_of(&unit).is_some());
    drop(client);
    worker.join().unwrap();
    assert!(f.host.unit_status_of(&unit).is_none());
    assert!(f.meta().units.is_empty());
}
