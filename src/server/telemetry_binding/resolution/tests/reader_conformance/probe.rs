use super::*;
use crate::machine_protocol::{MachineCommand, MachineEvent, MachineFrame};
use fixtures::{JOURNAL, MACHINE, SERVICE};
use futures::{SinkExt as _, StreamExt as _};
use std::os::unix::{
    fs::{OpenOptionsExt as _, PermissionsExt as _},
    process::CommandExt as _,
};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncReadExt as _;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{Message, protocol::WebSocketConfig},
};

const DEADLINE: Duration = Duration::from_secs(12);
const LOG_BYTES: usize = 128 * 1024;

fn private_write(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write as _;
    std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(path)?
        .write_all(bytes)?;
    Ok(())
}

fn database(root: &Path) -> String {
    format!(
        "sqlite://{}?mode=rwc",
        root.join("controller/store.sqlite3").display()
    )
}

fn row(fixture: &Fixture) -> Option<(String, String)> {
    fixture.document.as_ref().map(|document| {
        (
            document.clone(),
            if fixture.case == Case::ChecksumCorrupt {
                sha256(b"wrong checksum")
            } else {
                sha256(document.as_bytes())
            },
        )
    })
}

pub(super) async fn seed(root: &Path, fixture: &Fixture) -> Result<()> {
    for dir in [
        "controller",
        "machine/plugin-operations",
        "workspace",
        "tmp",
    ] {
        let dir = root.join(dir);
        std::fs::create_dir_all(&dir)?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    private_write(&root.join("controller/service-id"), SERVICE.as_bytes())?;
    if let Some(bytes) = &fixture.machine {
        private_write(&root.join("machine").join(JOURNAL), bytes)?;
    }
    let store =
        crate::store::Store::connect(&database(root), root.join("controller/artifacts")).await?;
    store.migrate().await?;
    let pool = sqlx::SqlitePool::connect(&database(root)).await?;
    if let Some((document, checksum)) = row(fixture) {
        sqlx::query("INSERT INTO telemetry_binding_journal (slot, document, document_sha256) VALUES ('telemetry', $1, $2)")
            .bind(document).bind(checksum).execute(&pool).await?;
    }
    pool.close().await;
    Ok(())
}

fn unchanged(root: &Path, fixture: &Fixture) -> Result<()> {
    use rusqlite::OptionalExtension as _;
    let db = rusqlite::Connection::open_with_flags(
        root.join("controller/store.sqlite3"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let retained: Option<(String, String)> = db.query_row("SELECT document, document_sha256 FROM telemetry_binding_journal WHERE slot='telemetry'", [], |row| Ok((row.get(0)?, row.get(1)?))).optional()?;
    ensure!(retained == row(fixture), "Service journal changed");
    let path = root.join("machine").join(JOURNAL);
    match &fixture.machine {
        Some(bytes) => ensure!(std::fs::read(path)? == *bytes, "Machine journal changed"),
        None => ensure!(
            matches!(path.symlink_metadata(), Err(e) if e.kind() == std::io::ErrorKind::NotFound),
            "absent Machine journal created"
        ),
    }
    Ok(())
}

fn command(executable: &Path, root: &Path) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(executable);
    command
        .env_clear()
        .env("PATH", "/no-ambient-commands")
        .env("LANG", "C.UTF-8")
        .env("RUST_LOG", "info")
        .env("TMPDIR", root.join("tmp"))
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command.as_std_mut().process_group(0);
    command
}

struct Running {
    child: tokio::process::Child,
    pid: rustix::process::Pid,
    logs: Arc<parking_lot::Mutex<Vec<u8>>>,
    drain: tokio::task::JoinHandle<()>,
    finished: bool,
}

impl Running {
    fn spawn(command: &mut tokio::process::Command) -> Result<Self, Failure> {
        let mut child = command.spawn().map_err(|_| Failure::Spawn)?;
        let pid = rustix::process::Pid::from_raw(
            i32::try_from(child.id().ok_or(Failure::Spawn)?).map_err(|_| Failure::Spawn)?,
        )
        .ok_or(Failure::Spawn)?;
        let mut stderr = child.stderr.take().ok_or(Failure::Spawn)?;
        let logs = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let captured = logs.clone();
        let drain = tokio::spawn(async move {
            let mut buffer = [0; 4096];
            while let Ok(count) = stderr.read(&mut buffer).await {
                if count == 0 {
                    break;
                }
                let mut logs = captured.lock();
                let remaining = LOG_BYTES.saturating_sub(logs.len());
                logs.extend_from_slice(&buffer[..count.min(remaining)]);
            }
        });
        Ok(Self {
            child,
            pid,
            logs,
            drain,
            finished: false,
        })
    }
    fn log_contains(&self, text: &str) -> bool {
        String::from_utf8_lossy(&self.logs.lock()).contains(text)
    }
    fn kill_group(&self) -> Result<(), Failure> {
        match rustix::process::kill_process_group(self.pid, rustix::process::Signal::KILL) {
            Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
            Err(_) => Err(Failure::Cleanup),
        }
    }
    async fn finish(&mut self) -> Result<(), Failure> {
        self.kill_group()?;
        tokio::time::timeout(Duration::from_secs(3), self.child.wait())
            .await
            .map_err(|_| Failure::Cleanup)?
            .map_err(|_| Failure::Cleanup)?;
        tokio::time::timeout(Duration::from_secs(3), &mut self.drain)
            .await
            .map_err(|_| Failure::Cleanup)?
            .map_err(|_| Failure::Cleanup)?;
        if rustix::process::test_kill_process_group(self.pid) != Err(rustix::io::Errno::SRCH) {
            return Err(Failure::Cleanup);
        }
        self.finished = true;
        Ok(())
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.kill_group();
        }
        self.drain.abort();
    }
}

pub(super) async fn run(
    artifact: &Artifact,
    fixture: &Fixture,
    root: &Path,
) -> Result<(), Failure> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|_| Failure::Setup)?;
    let address = listener.local_addr().map_err(|_| Failure::Setup)?;
    let mut command = command(&artifact.executable, root);
    match artifact.lane {
        Lane::Controller => {
            command
                .arg("serve")
                .arg("--bind")
                .arg(address.to_string())
                .arg("--data-dir")
                .arg(root.join("controller"))
                .arg("--database-url")
                .arg(database(root))
                .arg("--workspace-root")
                .arg(root.join("workspace"))
                .arg("--web-root")
                .arg(root.join("no-web"))
                .arg("--telemetry-dir")
                .arg(root.join("controller/telemetry"));
        }
        Lane::Machine => {
            command
                .arg("--controller-url")
                .arg(format!("http://{address}"))
                .arg("--service-id")
                .arg(SERVICE)
                .arg("--machine-id")
                .arg(MACHINE)
                .arg("--display-name")
                .arg("Isolated telemetry reader")
                .arg("--state-dir")
                .arg(root.join("machine"))
                .arg("--socket")
                .arg(root.join("broker.sock"))
                .arg("--spawn-mode")
                .arg("direct")
                // Any unexpected worker dispatch must fail, not start an agent.
                .arg("--worker-command")
                .arg(root.join("no-worker"))
                .arg("--provider-usage-socket")
                .arg(root.join("usage.sock"))
                .arg("--workspace-config")
                .arg(root.join("no-workspace-config"))
                .arg("--workspace")
                .arg(format!("fixture={}", root.join("workspace").display()));
        }
    }
    let listener = if artifact.lane == Lane::Controller {
        drop(listener);
        None
    } else {
        Some(listener)
    };
    let mut running = Running::spawn(&mut command)?;
    let result = tokio::time::timeout(DEADLINE, async {
        if fixture.case.corrupt() {
            // A flag error, unrelated crash or timeout is NOT a corrupt-reader pass.
            let status = running
                .child
                .wait()
                .await
                .map_err(|_| Failure::ExitedBeforeReady)?;
            tokio::time::sleep(Duration::from_millis(20)).await;
            let marker = corruption_marker(artifact.lane, fixture.case);
            if status.code().is_none_or(|code| code == 0) || !running.log_contains(marker) {
                return Err(Failure::UnexpectedReadiness);
            }
            return Ok(());
        }
        match listener {
            Some(listener) => machine(&mut running, listener, fixture, root).await,
            None => controller(&mut running, address, fixture).await,
        }
    })
    .await
    .unwrap_or(Err(Failure::Timeout));
    let cleanup = running.finish().await;
    let retained = unchanged(root, fixture).map_err(|_| Failure::EvidenceChanged);
    if result.is_err() && fixture.case == Case::Absent && artifact.role == Role::Active {
        eprintln!(
            "isolated fixture stderr: {}",
            String::from_utf8_lossy(&running.logs.lock())
        );
    }
    cleanup.and(retained).and(result)
}

fn corruption_marker(lane: Lane, case: Case) -> &'static str {
    match (lane, case) {
        (Lane::Controller, Case::ChecksumCorrupt) => "invalid Service binding checksum or capacity",
        (Lane::Controller, Case::AuditCorrupt) => "invalid binding resolution audit",
        (Lane::Machine, Case::ChecksumCorrupt) => "telemetry binding evidence integrity failure",
        (Lane::Machine, Case::AuditCorrupt) => "invalid Machine binding recovery audit",
        _ => unreachable!("not a corrupt case"),
    }
}

async fn controller(
    running: &mut Running,
    address: std::net::SocketAddr,
    fixture: &Fixture,
) -> Result<(), Failure> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_millis(250))
        .build()
        .map_err(|_| Failure::Setup)?;
    loop {
        if running
            .child
            .try_wait()
            .map_err(|_| Failure::ExitedBeforeReady)?
            .is_some()
        {
            return Err(Failure::ExitedBeforeReady);
        }
        if client
            .get(format!("http://{address}/healthz"))
            .send()
            .await
            .is_ok_and(|r| r.status().is_success())
        {
            let managed = if fixture.document.is_some() {
                "managed_namespace=true"
            } else {
                "managed_namespace=false"
            };
            return if running.log_contains("Service telemetry binding reader recovered")
                && running.log_contains("admission_enabled=false")
                && running.log_contains(managed)
            {
                Ok(())
            } else {
                Err(Failure::MissingReaderFence)
            };
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

type Socket = WebSocketStream<TcpStream>;

async fn send(socket: &mut Socket, frame: MachineFrame) -> Result<(), Failure> {
    let json = serde_json::to_string(&frame).map_err(|_| Failure::WrongProtocol)?;
    socket
        .send(Message::Text(json.into()))
        .await
        .map_err(|_| Failure::WrongProtocol)
}

async fn receive(socket: &mut Socket) -> Result<MachineFrame, Failure> {
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => {
                return serde_json::from_str(&text).map_err(|_| Failure::WrongProtocol);
            }
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => {}
            _ => return Err(Failure::WrongProtocol),
        }
    }
}

async fn machine(
    running: &mut Running,
    listener: TcpListener,
    fixture: &Fixture,
    root: &Path,
) -> Result<(), Failure> {
    let stream = tokio::select! {
        connection = listener.accept() => connection.map_err(|_| Failure::WrongProtocol)?.0,
        _ = running.child.wait() => return Err(Failure::ExitedBeforeReady),
    };
    let config = WebSocketConfig::default()
        .max_message_size(Some(256 * 1024))
        .max_frame_size(Some(256 * 1024));
    let mut socket = tokio_tungstenite::accept_async_with_config(stream, Some(config))
        .await
        .map_err(|_| Failure::WrongProtocol)?;
    let expires = chrono::Utc::now().timestamp_millis() + 30_000;
    send(
        &mut socket,
        MachineFrame::Challenge {
            challenge_id: "isolated-reader-challenge".into(),
            nonce: "isolated-reader-nonce".into(),
            expires_at_ms: expires,
            proof_version: 3,
        },
    )
    .await?;
    let MachineFrame::Hello { hello } = receive(&mut socket).await? else {
        return Err(Failure::WrongProtocol);
    };
    if hello.machine_id != MACHINE
        || hello.min_protocol > 17
        || hello.max_protocol < 17
        || !hello.plugins.is_empty()
    {
        return Err(Failure::WrongProtocol);
    }
    let public = std::fs::read_to_string(root.join("machine/identity_ed25519.pub"))
        .map_err(|_| Failure::WrongProtocol)?;
    let proof = crate::machine_protocol::challenge_proof_v3(
        "isolated-reader-challenge",
        "isolated-reader-nonce",
        expires,
        &hello,
    );
    if hello.challenge_id.as_deref() != Some("isolated-reader-challenge")
        || !crate::machine_auth::verify(
            &public,
            &proof,
            hello
                .challenge_signature
                .as_deref()
                .ok_or(Failure::WrongProtocol)?,
        )
        .map_err(|_| Failure::WrongProtocol)?
    {
        return Err(Failure::WrongProtocol);
    }
    send(
        &mut socket,
        MachineFrame::Welcome {
            protocol: 17,
            controller_epoch: 1,
            heartbeat_interval_ms: 1000,
            desired_components: Vec::new(),
        },
    )
    .await?;
    send(
        &mut socket,
        MachineFrame::Command {
            command: MachineCommand::QueryTelemetryBinding {
                request_id: "binding-read-only".into(),
                step: Box::new(fixture.step.clone()),
            },
        },
    )
    .await?;
    loop {
        match receive(&mut socket).await? {
            MachineFrame::Event {
                event:
                    MachineEvent::TelemetryBindingObservation {
                        request_id,
                        observation,
                    },
            } => {
                if request_id != "binding-read-only" || *observation != fixture.binding {
                    return Err(Failure::WrongObservation);
                }
                break;
            }
            MachineFrame::Heartbeat { .. }
            | MachineFrame::Event {
                event: MachineEvent::Inventory { .. },
            } => {}
            frame => {
                eprintln!("isolated fixture event: {frame:?}");
                return Err(Failure::WrongObservation);
            }
        }
    }
    send(
        &mut socket,
        MachineFrame::Command {
            command: MachineCommand::QueryTelemetryRecovery {
                request_id: "recovery-read-only".into(),
                recovery: Box::new(fixture.recovery.clone()),
            },
        },
    )
    .await?;
    loop {
        match receive(&mut socket).await? {
            MachineFrame::Event {
                event:
                    MachineEvent::TelemetryRecoveryObservation {
                        request_id,
                        observation,
                    },
            } => {
                return if request_id == "recovery-read-only" && *observation == fixture.audit {
                    Ok(())
                } else {
                    Err(Failure::WrongObservation)
                };
            }
            MachineFrame::Heartbeat { .. }
            | MachineFrame::Event {
                event: MachineEvent::Inventory { .. },
            } => {}
            frame => {
                eprintln!("isolated fixture event: {frame:?}");
                return Err(Failure::WrongObservation);
            }
        }
    }
}

#[test]
fn child_environment_is_closed_and_cannot_inherit_auth_or_host_paths() {
    let root = Path::new("/tmp/isolated-fixture");
    let command = command(Path::new("/nix/store/example/bin/cowboy"), root);
    let env: std::collections::BTreeMap<_, _> = command
        .as_std()
        .get_envs()
        .map(|(k, v)| (k.to_str().unwrap(), v.unwrap().to_str().unwrap()))
        .collect();
    assert_eq!(
        env.keys().copied().collect::<Vec<_>>(),
        ["LANG", "PATH", "RUST_LOG", "TMPDIR"]
    );
    assert_eq!(env["PATH"], "/no-ambient-commands");
    assert_eq!(command.as_std().get_current_dir(), Some(root));
}
