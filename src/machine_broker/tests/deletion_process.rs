//! Disposable OS-process fixtures, never a production writer switch.

use super::*;
use std::io::{BufRead as _, Write as _};
use std::os::unix::process::ExitStatusExt as _;
use std::process::{Child, Stdio};
use std::sync::mpsc::{Receiver, channel};

const CHILD: &str = "machine_broker::tests::deletion_process::child";
const READY: &str = "COWBOY_DELETION_CHILD_READY";
const CHECKPOINT: &str = "COWBOY_DELETION_WRITE_CHECKPOINT";

struct Process {
    child: Child,
    lines: Receiver<String>,
    output: Arc<Mutex<Vec<String>>>,
    pumps: Vec<std::thread::JoinHandle<()>>,
}

impl Process {
    fn spawn(root: &Path, mode: &str, checkpoint: &str) -> Self {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", CHILD, "--ignored", "--nocapture"])
            .env("COWBOY_TEST_DELETION_ROOT", root)
            .env("COWBOY_TEST_DELETION_MODE", mode)
            .env("COWBOY_TEST_DELETION_CHECKPOINT", checkpoint)
            .env_remove("COWBOY_PROVIDER_PACKAGE_PATH")
            .env_remove("LISTEN_PID")
            .env_remove("LISTEN_FDS")
            .env_remove("LISTEN_FDNAMES")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn isolated broker fixture");
        let (tx, lines) = channel();
        let output = Arc::new(Mutex::new(Vec::new()));
        let mut pumps = Vec::new();
        for pipe in [
            Box::new(child.stdout.take().unwrap()) as Box<dyn std::io::Read + Send>,
            Box::new(child.stderr.take().unwrap()),
        ] {
            let tx = tx.clone();
            let output = Arc::clone(&output);
            pumps.push(std::thread::spawn(move || {
                for line in std::io::BufReader::new(pipe).lines().map_while(Result::ok) {
                    output.lock().push(line.clone());
                    let _ = tx.send(line);
                }
            }));
        }
        Self {
            child,
            lines,
            output,
            pumps,
        }
    }

    fn marker(&self, marker: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let line = self
                .lines
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|error| {
                    panic!("child marker {marker}: {error}; {:?}", self.output.lock())
                });
            if line.contains(marker) {
                return;
            }
        }
    }

    fn kill(&mut self) {
        self.child.kill().expect("SIGKILL isolated broker");
        assert_eq!(self.child.wait().unwrap().signal(), Some(libc::SIGKILL));
        self.drain_output();
    }

    fn drain_output(&mut self) {
        for pump in self.pumps.drain(..) {
            pump.join().expect("child output reader");
        }
    }

    fn refused(&mut self, expected: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                self.drain_output();
                assert!(!status.success(), "invalid namespace was admitted");
                assert!(
                    self.output
                        .lock()
                        .iter()
                        .any(|line| line.contains(expected)),
                    "wrong startup refusal: {:?}",
                    self.output.lock()
                );
                assert!(!self.output.lock().iter().any(|line| line.contains(READY)));
                return;
            }
            assert!(
                Instant::now() < deadline,
                "invalid reader stayed alive: {:?}",
                self.output.lock()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.drain_output();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "launched only by disposable deletion process fixtures"]
async fn child() {
    let root = PathBuf::from(std::env::var_os("COWBOY_TEST_DELETION_ROOT").expect("fixture root"));
    let mode = std::env::var("COWBOY_TEST_DELETION_MODE").unwrap();
    assert!(matches!(
        mode.as_str(),
        "writer"
            | "reader"
            | "foreign-machine"
            | "foreign-service"
            | "release-writer"
            | "release-reader"
    ));
    let mut owner = deletion_fixture_owner();
    let release_compatible = mode.starts_with("release-");
    if release_compatible {
        owner.machine_id = "release-fixture".into();
        owner.service_id = Some("svc-0123456789abcdef0123456789abcdef".into());
    }
    if mode == "foreign-machine" {
        owner.machine_id = "foreign-machine".into();
    }
    if mode == "foreign-service" {
        owner.service_id = Some("foreign-service".into());
    }
    let namespace = if release_compatible {
        "session-deletions"
    } else {
        "deletions"
    };
    let mut journal = deletions::Journal::open(
        &root.join(namespace),
        owner,
        matches!(mode.as_str(), "writer" | "release-writer"),
    )
    .expect("fixture journal admission");
    let checkpoint = std::env::var("COWBOY_TEST_DELETION_CHECKPOINT").unwrap();
    if !checkpoint.is_empty() {
        journal.set_checkpoint(move |stage| {
            if format!("{stage:?}") == checkpoint {
                println!("{CHECKPOINT} {stage:?}");
                std::io::stdout().flush().unwrap();
                loop {
                    std::thread::park();
                }
            }
        });
    }
    let (fixture, _, _) = reconnecting_worker_fixture();
    let mut args = fixture.args.clone();
    args.socket = root.join("runtime.sock");
    args.worktree_root = root.join("worktrees");
    let socket = args.socket.clone();
    let server = tokio::spawn(run_broker(args, Some(journal), None, None));
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if UnixStream::connect(&socket).await.is_ok() {
                break;
            }
            if server.is_finished() {
                panic!("fixture broker ended before binding");
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("fixture listener bound");
    println!("{READY}");
    std::io::stdout().flush().unwrap();
    server.await.unwrap().unwrap();
}

async fn frame(reader: &mut tokio::net::unix::OwnedReadHalf) -> Option<Frame> {
    tokio::time::timeout(Duration::from_secs(5), read_frame(reader))
        .await
        .unwrap()
        .unwrap()
}

async fn stop(writer: &mut tokio::net::unix::OwnedWriteHalf) {
    write_frame(
        writer,
        &Frame::CoreCommand {
            command: CoreCommand::StopSession {
                session_id: "sess-1".into(),
                command_id: "process-delete".into(),
            },
        },
    )
    .await
    .unwrap();
}

async fn cold_fence(root: &Path, deleted: bool) {
    let mut reader_process = Process::spawn(root, "reader", "");
    reader_process.marker(READY);
    let socket = root.join("runtime.sock");
    let (worker_reader, worker_writer, reply) =
        connect_peer(&socket, PeerRole::Worker, Some("sess-1"), Some("old-epoch")).await;
    if deleted {
        assert!(matches!(reply, Frame::Reject { reason } if reason.contains("deleted")));
    } else {
        assert!(matches!(reply, Frame::Welcome { .. }));
    }
    drop(worker_reader);
    drop(worker_writer);
    if deleted {
        let (mut reader, mut writer, _) = connect_peer(&socket, PeerRole::Core, None, None).await;
        for adopt_only in [false, true] {
            let (_, mut launch, _) = reconnecting_worker_fixture();
            launch.adopt_only = adopt_only;
            write_frame(
                &mut writer,
                &Frame::CoreCommand {
                    command: CoreCommand::EnsureSession { session: launch },
                },
            )
            .await
            .unwrap();
            assert!(matches!(
                frame(&mut reader).await,
                Some(Frame::CommandAck { accepted: false, reason: Some(reason), .. }) if reason.contains("session was deleted")
            ));
        }
        assert!(!root.join("worktrees").exists());
    }
    reader_process.kill();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sigkill_at_each_write_boundary_reopens_without_replaying_staging() {
    for (checkpoint, published) in [
        ("Staged", false),
        ("FileSynced", false),
        ("Renamed", true),
        ("DirectorySynced", true),
    ] {
        let root = tempfile::tempdir().unwrap();
        let mut process = Process::spawn(root.path(), "writer", checkpoint);
        process.marker(READY);
        let (mut reader, mut writer, _) = connect_peer(
            &root.path().join("runtime.sock"),
            PeerRole::Core,
            None,
            None,
        )
        .await;
        stop(&mut writer).await;
        process.marker(CHECKPOINT);
        process.kill();
        // EOF after SIGKILL proves no positive acknowledgement was buffered.
        assert!(
            frame(&mut reader).await.is_none(),
            "unexpected ACK at {checkpoint}"
        );
        let journal_root = root.path().join("deletions");
        assert_eq!(journal_root.join("deletions.json").exists(), published);
        let pending = std::fs::read_dir(&journal_root)
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".pending-")
            })
            .count();
        assert_eq!(pending, usize::from(!published));
        cold_fence(root.path(), published).await;
        assert_eq!(journal_root.join("deletions.json").exists(), published);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn acknowledged_delete_survives_sigkill_and_a_second_reader_death() {
    let root = tempfile::tempdir().unwrap();
    let mut process = Process::spawn(root.path(), "writer", "");
    process.marker(READY);
    let (mut reader, mut writer, _) = connect_peer(
        &root.path().join("runtime.sock"),
        PeerRole::Core,
        None,
        None,
    )
    .await;
    stop(&mut writer).await;
    assert!(
        matches!(frame(&mut reader).await, Some(Frame::CommandAck { accepted: true, command_id, .. }) if command_id == "process-delete")
    );
    process.kill();
    cold_fence(root.path(), true).await;
    cold_fence(root.path(), true).await;
    for mode in ["foreign-machine", "foreign-service"] {
        Process::spawn(root.path(), mode, "").refused("another Machine or Service");
    }
    // Neither rejected owner modified the terminal record.
    cold_fence(root.path(), true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn linked_reader_and_writer_namespaces_refuse_before_broker_admission() {
    for mode in ["reader", "writer"] {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("retained");
        std::fs::create_dir(&target).unwrap();
        let evidence = b"unrelated retained evidence";
        std::fs::write(target.join("evidence"), evidence).unwrap();
        std::os::unix::fs::symlink(&target, root.path().join("deletions")).unwrap();
        Process::spawn(root.path(), mode, "").refused("without following namespace links");
        assert!(!root.path().join("runtime.sock").exists());
        assert_eq!(std::fs::read_dir(&target).unwrap().count(), 1);
        assert_eq!(std::fs::read(target.join("evidence")).unwrap(), evidence);
        assert_eq!(
            std::fs::read_link(root.path().join("deletions")).unwrap(),
            target
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn process_replaced_lock_refuses_delete_without_mutating_evidence() {
    let root = tempfile::tempdir().unwrap();
    let mut process = Process::spawn(root.path(), "writer", "");
    process.marker(READY);
    let namespace = root.path().join("deletions");
    std::fs::rename(namespace.join(".lock"), namespace.join("retained-lock")).unwrap();
    let replacement = b"replacement lock evidence";
    std::fs::write(namespace.join(".lock"), replacement).unwrap();
    let (mut reader, mut writer, _) = connect_peer(
        &root.path().join("runtime.sock"),
        PeerRole::Core,
        None,
        None,
    )
    .await;
    stop(&mut writer).await;
    assert!(matches!(
        frame(&mut reader).await,
        Some(Frame::CommandAck { accepted: false, reason: Some(reason), .. })
            if reason.contains("lock was replaced")
    ));
    assert!(!namespace.join("deletions.json").exists());
    assert_eq!(std::fs::read(namespace.join(".lock")).unwrap(), replacement);
    assert_eq!(std::fs::read_dir(&namespace).unwrap().count(), 2);
    process.kill();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn process_storage_failure_refuses_ack_and_cold_adoption() {
    let root = tempfile::tempdir().unwrap();
    let mut process = Process::spawn(root.path(), "writer", "");
    process.marker(READY);
    std::fs::create_dir(root.path().join("deletions/deletions.json")).unwrap();
    let (mut reader, mut writer, _) = connect_peer(
        &root.path().join("runtime.sock"),
        PeerRole::Core,
        None,
        None,
    )
    .await;
    stop(&mut writer).await;
    assert!(
        matches!(frame(&mut reader).await, Some(Frame::CommandAck { accepted: false, command_id, reason: Some(reason), .. }) if command_id == "process-delete" && reason.contains("durable Session deletion was not confirmed"))
    );
    let (_, launch, _) = reconnecting_worker_fixture();
    write_frame(
        &mut writer,
        &Frame::CoreCommand {
            command: CoreCommand::EnsureSession { session: launch },
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        frame(&mut reader).await,
        Some(Frame::CommandAck { accepted: false, reason: Some(reason), .. }) if reason.contains("reader unavailable")
    ));
    process.kill();
    Process::spawn(root.path(), "reader", "").refused("not a regular file");
}
