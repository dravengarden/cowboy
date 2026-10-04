//! Opt-in acceptance of two independently supplied immutable reader releases.

#![cfg(all(feature = "machine-host", target_os = "linux"))]

use cowboy::runtime_wire::{CoreCommand, Frame, PeerRole, read_frame, write_frame};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::fs::{File, read, write};
use std::net::TcpListener;
use std::os::unix::process::ExitStatusExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tokio::net::UnixStream;

const SERVICE: &str = "svc-0123456789abcdef0123456789abcdef";
const SESSION: &str = "release-fixture-terminal";

struct Release {
    root: PathBuf,
    revision: String,
    digest: String,
}

impl Release {
    fn supplied(variable: &str) -> Self {
        let root = PathBuf::from(std::env::var_os(variable).expect(variable));
        assert_eq!(root.canonicalize().unwrap(), root);
        assert_eq!(root.parent(), Some(Path::new("/nix/store")));
        let source: Value =
            serde_json::from_slice(&read(root.join("etc/cowboy-release/source.json")).unwrap())
                .unwrap();
        assert_eq!(source["schema"], 1);
        assert_eq!(source["component"], "cowboy");
        assert_eq!(source["lane"], "machine");
        assert_eq!(source["dirty"], false);
        assert_eq!(
            source["sessionDeletionJournal"],
            json!({"readerSchema": 1, "writerSchema": 0})
        );
        let revision = source["revision"].as_str().unwrap().to_owned();
        assert_eq!(revision.len(), 40);
        assert!(revision.bytes().all(|byte| byte.is_ascii_hexdigit()));
        let packaged_launcher = root.join("libexec/cowboy-machine").canonicalize().unwrap();
        let native = packaged_launcher
            .parent()
            .unwrap()
            .join(".cowboy-machine-wrapped");
        let native_bytes = read(&native).unwrap();
        assert!(native_bytes.starts_with(b"\x7fELF"));
        let digest = format!("{:x}", Sha256::digest(native_bytes));
        let launcher_digest = format!(
            "{:x}",
            Sha256::digest(read(root.join("bin/cowboy-machine")).unwrap())
        );
        let packaged_launcher_digest =
            format!("{:x}", Sha256::digest(read(packaged_launcher).unwrap()));
        println!(
            "release={} revision={revision} native={} native_sha256={digest} launcher_sha256={launcher_digest} packaged_launcher_sha256={packaged_launcher_digest}",
            root.display(),
            native.display()
        );
        Self {
            root,
            revision,
            digest,
        }
    }
}

struct Machine {
    child: Child,
    socket: PathBuf,
    log: PathBuf,
    // Retain an exclusively bound loopback endpoint: no real Controller is used.
    _controller: TcpListener,
}

impl Machine {
    fn spawn(release: &Release, state: &Path) -> Self {
        let controller = TcpListener::bind("127.0.0.1:0").unwrap();
        let socket = state.join("runtime.sock");
        let log = state.join("machine.log");
        let output = File::create(&log).unwrap();
        let child = Command::new(release.root.join("bin/cowboy-machine"))
            .env_clear()
            .env("HOME", state.join("home"))
            .env("XDG_CONFIG_HOME", state.join("config"))
            .env("XDG_CACHE_HOME", state.join("cache"))
            .env("XDG_DATA_HOME", state.join("data"))
            .env("RUST_LOG", "info")
            .args([
                "--controller-url",
                &format!("http://{}", controller.local_addr().unwrap()),
                "--service-id",
                SERVICE,
                "--machine-id",
                "release-fixture",
                "--spawn-mode",
                "direct",
                "--worker-command",
                "/no-fixture-worker",
            ])
            .arg("--state-dir")
            .arg(state)
            .arg("--socket")
            .arg(&socket)
            .arg("--provider-usage-socket")
            .arg(state.join("usage.sock"))
            .arg("--workspace-config")
            .arg(state.join("absent-workspaces.json"))
            .current_dir(state)
            .stdin(Stdio::null())
            .stdout(output.try_clone().unwrap())
            .stderr(output)
            .spawn()
            .unwrap();
        Self {
            child,
            socket,
            log,
            _controller: controller,
        }
    }

    fn output(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap()
    }

    async fn ready(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "Machine exited before admission: {}",
                self.output()
            );
            if UnixStream::connect(&self.socket).await.is_ok() {
                assert!(
                    self.output()
                        .contains("Session deletion journal reader ready")
                );
                assert!(self.output().contains("writer_enabled=false"));
                return;
            }
            assert!(Instant::now() < deadline, "{}", self.output());
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn refused(&mut self, reason: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(!status.success(), "invalid record admitted");
                assert!(self.output().contains(reason), "{}", self.output());
                assert!(!self.socket.exists(), "broker bound before refusing record");
                return;
            }
            assert!(Instant::now() < deadline, "{}", self.output());
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    fn kill(&mut self) {
        self.child.kill().unwrap();
        assert_eq!(self.child.wait().unwrap().signal(), Some(libc::SIGKILL));
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn next(reader: &mut tokio::net::unix::OwnedReadHalf) -> Frame {
    tokio::time::timeout(Duration::from_secs(5), read_frame(reader))
        .await
        .unwrap()
        .unwrap()
        .unwrap()
}

async fn peer(
    socket: &Path,
    role: PeerRole,
) -> (
    tokio::net::unix::OwnedReadHalf,
    tokio::net::unix::OwnedWriteHalf,
    Frame,
) {
    let (mut reader, mut writer) = UnixStream::connect(socket).await.unwrap().into_split();
    write_frame(
        &mut writer,
        &Frame::Hello {
            role,
            min_protocol: 1,
            max_protocol: 2,
            build: "release-fixture".into(),
            session_id: (role == PeerRole::Worker).then(|| SESSION.into()),
            worker_epoch: (role == PeerRole::Worker).then(|| "surviving-epoch".into()),
            generation: Some("fixture-generation".into()),
            executable: Some("/no-fixture-worker".into()),
            fallback_for: None,
        },
    )
    .await
    .unwrap();
    let reply = next(&mut reader).await;
    (reader, writer, reply)
}

async fn terminal_fence(machine: &Machine) {
    let (_, _, reply) = peer(&machine.socket, PeerRole::Worker).await;
    assert!(matches!(reply, Frame::Reject { reason } if reason.contains("deleted")));
    let (mut reader, mut writer, reply) = peer(&machine.socket, PeerRole::Core).await;
    assert!(matches!(reply, Frame::Welcome { .. }));
    for adopt_only in [false, true] {
        let session = serde_json::from_value(json!({
            "session_id": SESSION, "provider": "no-fixture-provider",
            "cwd": "/no-fixture-workspace", "generation": "fixture-generation",
            "adopt_only": adopt_only
        }))
        .unwrap();
        write_frame(
            &mut writer,
            &Frame::CoreCommand {
                command: CoreCommand::EnsureSession { session },
            },
        )
        .await
        .unwrap();
        assert!(matches!(
            next(&mut reader).await,
            Frame::CommandAck { accepted: false, reason: Some(reason), .. }
                if reason.contains("session was deleted")
        ));
    }
}

fn record() -> Value {
    json!({"schema": 1, "owner": {"machine_id": "release-fixture", "service_id": SERVICE}, "deleted": [SESSION]})
}

fn namespace(state: &Path) -> PathBuf {
    let root = state.join("session-deletions");
    std::fs::create_dir_all(&root).unwrap();
    root
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires COWBOY_TEST_OLD_MACHINE_RELEASE and COWBOY_TEST_NEW_MACHINE_RELEASE"]
async fn immutable_namespace_link_refusal_and_preceding_admission() {
    let old = Release::supplied("COWBOY_TEST_OLD_MACHINE_RELEASE");
    let new = Release::supplied("COWBOY_TEST_NEW_MACHINE_RELEASE");
    assert_ne!(old.revision, new.revision);
    assert_ne!(old.digest, new.digest);
    let mut observations = Vec::new();
    for (release, preceding) in [(&old, true), (&new, false)] {
        for committed in [false, true] {
            let state = tempfile::tempdir_in("/tmp").unwrap();
            let target = state.path().join("retained-namespace");
            std::fs::create_dir(&target).unwrap();
            let retained = serde_json::to_vec(&record()).unwrap();
            if committed {
                write(target.join("deletions.json"), &retained).unwrap();
            }
            let root = state.path().join("session-deletions");
            std::os::unix::fs::symlink(&target, &root).unwrap();
            let mut machine = Machine::spawn(release, state.path());
            if preceding {
                machine.ready().await;
                assert!(target.join(".lock").is_file());
                machine.kill();
            } else {
                machine.refused("without following namespace links").await;
                assert!(!target.join(".lock").exists());
                assert_eq!(
                    std::fs::read_dir(&target).unwrap().count(),
                    usize::from(committed)
                );
            }
            assert_eq!(std::fs::read_link(root).unwrap(), target);
            if committed {
                assert_eq!(read(target.join("deletions.json")).unwrap(), retained);
            }
            observations.push(json!({
                "release": release.root, "revision": release.revision,
                "nativeSha256": release.digest, "committed": committed,
                "precedingReaderAdmittedLink": preceding,
                "newReaderRefusedBeforeBinding": !preceding,
                "targetRecordAndLinkUnchanged": true,
            }));
        }
    }
    if let Ok(path) = std::env::var("COWBOY_TEST_DELETION_NAMESPACE_RECEIPT") {
        write(
            path,
            serde_json::to_vec_pretty(&json!({
                "schema": 1, "accepted": true, "observations": observations,
                "productionWriterEnabled": false,
            }))
            .unwrap(),
        )
        .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires COWBOY_TEST_OLD_MACHINE_RELEASE and COWBOY_TEST_NEW_MACHINE_RELEASE"]
async fn immutable_reader_upgrade_rollback_and_refusal_matrix() {
    let old = Release::supplied("COWBOY_TEST_OLD_MACHINE_RELEASE");
    let new = Release::supplied("COWBOY_TEST_NEW_MACHINE_RELEASE");
    assert_ne!(old.root, new.root);
    assert_ne!(old.revision, new.revision);
    assert_ne!(old.digest, new.digest, "release executables must differ");

    // Synthetic reader input, never a claim of a production deletion ACK.
    let state = tempfile::tempdir_in("/tmp").unwrap();
    let committed = namespace(state.path()).join("deletions.json");
    let bytes = serde_json::to_vec(&record()).unwrap();
    write(&committed, &bytes).unwrap();
    for release in [&old, &new, &old] {
        let mut machine = Machine::spawn(release, state.path());
        machine.ready().await;
        terminal_fence(&machine).await;
        assert_eq!(read(&committed).unwrap(), bytes);
        assert!(!state.path().join("worktrees").exists());
        machine.kill();
    }

    for release in [&old, &new] {
        for pending in [false, true] {
            let state = tempfile::tempdir_in("/tmp").unwrap();
            let root = namespace(state.path());
            let staging = root.join(".pending-fixture");
            if pending {
                write(&staging, b"uncommitted invalid staging").unwrap();
            }
            let mut machine = Machine::spawn(release, state.path());
            machine.ready().await;
            let (mut reader, mut writer, reply) = peer(&machine.socket, PeerRole::Core).await;
            assert!(matches!(reply, Frame::Welcome { .. }));
            write_frame(
                &mut writer,
                &Frame::CoreCommand {
                    command: CoreCommand::StopSession {
                        session_id: SESSION.into(),
                        command_id: "fixture-volatile-delete".into(),
                    },
                },
            )
            .await
            .unwrap();
            assert!(matches!(
                next(&mut reader).await,
                Frame::CommandAck { accepted: true, command_id, .. }
                    if command_id == "fixture-volatile-delete"
            ));
            let (_, _, reply) = peer(&machine.socket, PeerRole::Worker).await;
            assert!(matches!(reply, Frame::Reject { reason } if reason.contains("deleted")));
            assert!(!root.join("deletions.json").exists());
            machine.kill();
            let mut reopened = Machine::spawn(release, state.path());
            reopened.ready().await;
            let (_, _, reply) = peer(&reopened.socket, PeerRole::Worker).await;
            assert!(matches!(reply, Frame::Welcome { .. }));
            assert!(!root.join("deletions.json").exists());
            if pending {
                assert_eq!(read(&staging).unwrap(), b"uncommitted invalid staging");
            }
            reopened.kill();
        }

        for (case, reason) in [
            ("malformed", "invalid deletion journal"),
            ("schema", "unsupported deletion journal schema"),
            ("machine", "another Machine or Service"),
            ("service", "another Machine or Service"),
            ("duplicate", "invalid or duplicate deletion identity"),
            ("unknown", "invalid deletion journal"),
            ("limit", "exceeds record limit"),
            ("directory", "not a regular file"),
            ("dangling", "Too many levels of symbolic links"),
            ("symlink", "Too many levels of symbolic links"),
        ] {
            let state = tempfile::tempdir_in("/tmp").unwrap();
            let committed = namespace(state.path()).join("deletions.json");
            let mut invalid = record();
            match case {
                "malformed" => write(&committed, b"{").unwrap(),
                "directory" => std::fs::create_dir(&committed).unwrap(),
                "dangling" => {
                    std::os::unix::fs::symlink(state.path().join("absent"), &committed).unwrap();
                }
                "symlink" => {
                    let target = state.path().join("untrusted.json");
                    write(&target, serde_json::to_vec(&invalid).unwrap()).unwrap();
                    std::os::unix::fs::symlink(target, &committed).unwrap();
                }
                _ => {
                    match case {
                        "schema" => invalid["schema"] = json!(2),
                        "machine" => invalid["owner"]["machine_id"] = json!("another-machine"),
                        "service" => {
                            invalid["owner"]["service_id"] =
                                json!("svc-ffffffffffffffffffffffffffffffff");
                        }
                        "duplicate" => invalid["deleted"] = json!([SESSION, SESSION]),
                        "unknown" => invalid["future"] = json!(true),
                        "limit" => {
                            invalid["deleted"] =
                                json!((0..4097).map(|i| format!("id-{i}")).collect::<Vec<_>>());
                        }
                        _ => unreachable!(),
                    }
                    write(&committed, serde_json::to_vec(&invalid).unwrap()).unwrap();
                }
            }
            let original = std::fs::symlink_metadata(&committed).unwrap();
            let original_bytes = original.is_file().then(|| read(&committed).unwrap());
            let original_link = original
                .file_type()
                .is_symlink()
                .then(|| std::fs::read_link(&committed).unwrap());
            let mut machine = Machine::spawn(release, state.path());
            machine.refused(reason).await;
            assert_eq!(
                std::fs::symlink_metadata(&committed).unwrap().file_type(),
                original.file_type()
            );
            if let Some(bytes) = original_bytes {
                assert_eq!(read(&committed).unwrap(), bytes);
            }
            if let Some(target) = original_link {
                assert_eq!(std::fs::read_link(&committed).unwrap(), target);
            }
            if original.is_dir() {
                assert_eq!(std::fs::read_dir(&committed).unwrap().count(), 0);
            }
        }
    }
    println!("matrix passed: 31 Machine processes; 11 SIGKILL/reaped; 20 startup refusals");
}
