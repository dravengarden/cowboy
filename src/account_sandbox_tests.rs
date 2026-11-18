use super::*;

use std::io::{BufRead as _, BufReader, IoSlice};
use std::os::fd::{AsFd as _, BorrowedFd};
use std::os::unix::net::UnixListener;

use rustix::net::{SendAncillaryBuffer, SendAncillaryMessage, SendFlags};

const KEY: &str = "0123456789abcdef";
/// `account_key("service", "account")`, computed independently.
const ACCOUNT_KEY: &str = "c45ec8de121807c0";
const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

/// One scripted exchange of the fake daemon.
enum Reply {
    Line(Value),
    /// A start reply carrying fresh pipes; the server keeps the unit's ends.
    Start {
        mode: &'static str,
    },
    /// Read `size` payload bytes, then answer.
    Upload {
        size: usize,
        reply: Value,
    },
}

struct Fake {
    socket: PathBuf,
    _dir: tempfile::TempDir,
    server: std::thread::JoinHandle<Vec<Value>>,
}

/// A fake daemon serving one connection with `script`; it returns every
/// request line it received.
fn fake(script: Vec<Reply>) -> Fake {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("control.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut requests = Vec::new();
        let mut held = Vec::new();
        let mut dirs = Vec::new();
        for reply in script {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap() == 0 {
                break;
            }
            requests.push(serde_json::from_str::<Value>(&line).unwrap());
            match reply {
                Reply::Line(value) => send(&stream, &value, &[]),
                Reply::Upload { size, reply } => {
                    let mut payload = vec![0; size];
                    reader.read_exact(&mut payload).unwrap();
                    requests.push(json!({"payload": String::from_utf8(payload).unwrap()}));
                    send(&stream, &reply, &[]);
                }
                Reply::Start { mode } => {
                    let (stdin_read, stdin_write) = std::io::pipe().unwrap();
                    let (stdout_read, stdout_write) = std::io::pipe().unwrap();
                    let (stderr_read, stderr_write) = std::io::pipe().unwrap();
                    let mut names = vec!["stdin", "stdout", "stderr"];
                    let mut fds: Vec<OwnedFd> =
                        vec![stdin_write.into(), stdout_read.into(), stderr_read.into()];
                    if mode == "runtime" {
                        let exec_dir = tempfile::tempdir().unwrap();
                        let exec = UnixListener::bind(exec_dir.path().join("exec.sock")).unwrap();
                        fds.push(exec.into());
                        dirs.push(exec_dir);
                        names.push("exec_listener");
                    }
                    let value = json!({"ok": true, "result": {
                        "unit": format!("cga-{KEY}-{mode}-0123456789abcdef.service"),
                        "mode": mode, "invocation_id": "inv", "boot_id": "boot",
                        "fds": names,
                    }});
                    let borrowed: Vec<BorrowedFd<'_>> = fds.iter().map(|fd| fd.as_fd()).collect();
                    send(&stream, &value, &borrowed);
                    let mut stdout_write = stdout_write;
                    stdout_write.write_all(b"hello\n").unwrap();
                    held.push((stdin_read, stdout_write, stderr_write));
                }
            }
        }
        requests
    });
    Fake {
        socket,
        _dir: dir,
        server,
    }
}

fn send(stream: &UnixStream, value: &Value, fds: &[BorrowedFd<'_>]) {
    let mut bytes = serde_json::to_vec(value).unwrap();
    bytes.push(b'\n');
    if fds.is_empty() {
        (&*stream).write_all(&bytes).unwrap();
        return;
    }
    let mut space = [MaybeUninit::<u8>::uninit(); rustix::cmsg_space!(ScmRights(8))];
    let mut control = SendAncillaryBuffer::new(&mut space);
    assert!(control.push(SendAncillaryMessage::ScmRights(fds)));
    let sent = rustix::net::sendmsg(
        stream,
        &[IoSlice::new(&bytes)],
        &mut control,
        SendFlags::NOSIGNAL,
    )
    .unwrap();
    (&*stream).write_all(&bytes[sent..]).unwrap();
}

fn ok(result: Value) -> Reply {
    Reply::Line(json!({"ok": true, "result": result}))
}

#[test]
fn account_registration_and_queries_round_trip() {
    let fake = fake(vec![
        ok(json!({"key": ACCOUNT_KEY, "user": format!("cga-{ACCOUNT_KEY}")})),
        Reply::Upload {
            size: 7,
            reply: json!({"ok": true, "result": {"sha256": SHA, "already_present": false}}),
        },
        ok(json!({
            "key": KEY, "boot_id": "boot", "fenced": null,
            "credential": {"generation": 3, "state": "signed_in"},
            "units": [{
                "unit": format!("cga-{KEY}-runtime-0123456789abcdef.service"), "mode": "runtime",
                "invocation_id": "inv", "boot_id": "boot", "credential_generation": 3,
                "session_id": "s", "state": "active", "alive": true, "owned": true,
            }],
            "unrecorded": [],
        })),
        ok(json!({
            "unit": "u", "recorded": false, "state": "gone", "alive": false, "owned": false,
            "current_boot_id": "boot",
        })),
        ok(json!({"key": KEY, "fence_epoch": 2, "fenced": true, "stopped": []})),
        ok(json!({"key": KEY, "fence_epoch": 2, "fenced": false})),
        ok(json!({
            "key": KEY, "credential_generation": 3, "token_removed": true,
            "credential_cleared": true, "history_retained": true,
        })),
    ]);
    let mut client = Client::connect(&fake.socket).unwrap();
    let account = client.ensure_account("service", "account").unwrap();
    assert_eq!(account.key, ACCOUNT_KEY);
    let registered = client
        .register_runtime(&mut &b"archive"[..], 7, SHA)
        .unwrap();
    assert!(!registered.already_present);
    let units = client.list_units(KEY).unwrap();
    assert!(units.credential.signed_in(3) && !units.credential.signed_in(2));
    assert_eq!(units.units[0].mode, Mode::Runtime);
    assert!(!client.unit_state("u").unwrap().alive);
    let order = FenceOrder {
        revision: 7,
        operation_id: "fence-7",
    };
    assert_eq!(
        client.stop_account(KEY, Some(order)).unwrap().fence_epoch,
        2
    );
    assert_eq!(client.admit_account(KEY, None).unwrap().key, KEY);
    let cleanup = client.logout_cleanup(KEY, 3, Some("logout-op")).unwrap();
    assert!(cleanup.credential_cleared && cleanup.history_retained);
    drop(client);
    let requests = fake.server.join().unwrap();
    assert_eq!(
        requests[requests.len() - 3],
        json!({"op": "stop_account", "key": KEY, "revision": 7, "operation_id": "fence-7"})
    );
    assert_eq!(
        requests[requests.len() - 1],
        json!({"op": "logout_cleanup", "key": KEY, "credential_generation": 3,
               "operation_id": "logout-op"})
    );
    assert_eq!(
        requests[0],
        json!({"op": "ensure_account", "service_id": "service", "account_id": "account"})
    );
    assert_eq!(
        requests[1],
        json!({"op": "register_runtime", "sha256": SHA, "size": 7})
    );
    assert_eq!(requests[2], json!({"payload": "archive"}));
}

#[test]
fn start_receives_named_pipes_and_refusals_keep_their_code() {
    let fake = fake(vec![
        Reply::Line(
            json!({"ok": false, "error": {"code": "fenced", "message": "the account is fenced"}}),
        ),
        Reply::Start { mode: "login" },
        Reply::Start { mode: "runtime" },
        ok(json!({"unit": "u", "stopped": true})),
    ]);
    let mut client = Client::connect(&fake.socket).unwrap();
    let request = StartRequest {
        key: KEY,
        mode: Mode::Login,
        credential_generation: 4,
        runtime_sha256: SHA,
        session_id: None,
        descriptor: None,
    };
    let refused = client.start(&request).unwrap_err();
    assert_eq!(DaemonError::code_of(&refused), Some("fenced"));
    let login = client.start(&request).unwrap();
    assert_eq!(login.mode, Mode::Login);
    assert!(login.exec_listener.is_none());
    assert_eq!(login.handle.invocation_id.as_deref(), Some("inv"));
    let mut output = String::new();
    std::io::BufReader::new(std::fs::File::from(login.stdout))
        .read_line(&mut output)
        .unwrap();
    assert_eq!(output, "hello\n");
    let runtime = client
        .start(&StartRequest {
            mode: Mode::Runtime,
            session_id: Some("session"),
            descriptor: Some(b"{}"),
            ..request
        })
        .unwrap();
    assert!(runtime.exec_listener.is_some());
    assert!(client.stop_unit("u").unwrap());
    drop(client);
    let requests = fake.server.join().unwrap();
    assert_eq!(requests[2]["session_id"], json!("session"));
    assert_eq!(requests[2]["descriptor_b64"], json!("e30="));
    assert!(requests[1].get("session_id").is_none());
}

#[test]
fn malformed_and_closed_responses_fail() {
    assert!(response(b"{\"ok\":true}").is_err());
    assert!(response(b"not json").is_err());
    let error = response(br#"{"ok":false,"error":{"code":"limit","message":"m"}}"#).unwrap_err();
    assert_eq!(DaemonError::code_of(&error), Some("limit"));
    let fake = fake(vec![]);
    let mut client = Client::connect(&fake.socket).unwrap();
    // Closed before answering: a reset or an end of stream, never a result.
    assert!(client.list_units(KEY).is_err());
    fake.server.join().unwrap();
    assert!(Client::connect(Path::new("/nonexistent/control.sock")).is_err());
}

#[test]
fn launch_round_trips_strictly() {
    let launch = Launch {
        socket: DEFAULT_SOCKET.into(),
        service_id: "service".into(),
        account_id: "account".into(),
        credential_generation: 2,
        runtime_sha256: SHA.into(),
        runtime_archive: "/state/runtime.tar".into(),
    };
    let value = serde_json::to_value(&launch).unwrap();
    assert_eq!(
        serde_json::from_value::<Launch>(value.clone()).unwrap(),
        launch
    );
    let mut extra = value;
    extra["extra"] = json!(true);
    assert!(serde_json::from_value::<Launch>(extra).is_err());
}

#[test]
fn account_keys_follow_the_daemon_derivation() {
    assert_eq!(account_key("service", "account"), ACCOUNT_KEY);
    assert_ne!(account_key("ab", "c"), account_key("a", "bc"));
}

#[test]
fn execution_ledgers_are_keyed_by_a_plain_session_id() {
    let root = Path::new("/var/lib/cowboy-machine");
    assert_eq!(
        execution_ledger_file(root, "sess-1_a").unwrap(),
        root.join("execution-ledgers/sess-1_a.json")
    );
    for invalid in ["", "../x", "a/b", "a.b", &"x".repeat(129)] {
        assert!(execution_ledger_file(root, invalid).is_none(), "{invalid}");
    }
}

#[test]
fn runtime_probes_name_the_probed_runtime() {
    let fake = fake(vec![
        ok(json!({"runtime_sha256": SHA, "unit": "u", "exit_code": 0, "passed": true})),
        ok(json!({"runtime_sha256": "b".repeat(64), "unit": "u", "exit_code": 0, "passed": true})),
    ]);
    let mut client = Client::connect(&fake.socket).unwrap();
    assert!(client.probe_runtime(SHA).unwrap().passed);
    assert!(client.probe_runtime(SHA).is_err());
    drop(client);
    let requests = fake.server.join().unwrap();
    assert_eq!(
        requests[0],
        json!({"op": "probe_runtime", "runtime_sha256": SHA})
    );
}
