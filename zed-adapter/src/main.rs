use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use clap::{Parser, Subcommand};
use proto::Message as _;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, RwLock};

const ADAPTER_VERSION: u8 = 1;
const ZED_VERSION: &str = "1.13.0";
const ZED_REVISION: &str = "aaf5f57dd36c41cf2ed49b13bcb091d52d5aef45";

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: CommandKind,
}

#[derive(Subcommand)]
enum CommandKind {
    Serve {
        #[arg(long)]
        socket: PathBuf,
        #[arg(long)]
        zed_server: PathBuf,
        #[arg(long)]
        state_dir: PathBuf,
    },
    Probe {
        #[arg(long)]
        socket: PathBuf,
        #[arg(long, default_value_t = 0)]
        wait_ms: u64,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Request {
    Health,
    EnsureWorktree { path: PathBuf, trusted: bool },
    OpenWorktree { path: PathBuf, trusted: bool },
    CloseWorktree { path: PathBuf },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Response {
    Health {
        api_version: u8,
        zed_version: &'static str,
        zed_revision: &'static str,
        worktrees: usize,
    },
    Worktree {
        api_version: u8,
        path: PathBuf,
        state: WorktreeState,
        leases: usize,
    },
    Error {
        api_version: u8,
        message: String,
    },
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
enum WorktreeState {
    Restricted,
    Warming,
    Ready,
}

#[derive(Clone, Copy)]
struct WorktreeLease {
    state: WorktreeState,
    leases: usize,
    remote_id: u64,
}

type Worktrees = Arc<RwLock<HashMap<PathBuf, WorktreeLease>>>;
type Zed = Arc<Mutex<ZedRuntime>>;

struct ZedRuntime {
    _child: Child,
    input: UnixStream,
    output: UnixStream,
    next_message_id: u32,
    next_worktree_id: u64,
}

async fn verify_zed_server(path: &Path) -> Result<()> {
    let output = Command::new(path)
        .arg("version")
        .output()
        .await
        .with_context(|| format!("failed to execute {}", path.display()))?;
    if !output.status.success() {
        bail!("Zed server version command failed");
    }
    let actual = String::from_utf8(output.stdout)?.trim().to_owned();
    if actual != ZED_VERSION {
        bail!("expected Zed {ZED_VERSION}, found {actual}");
    }
    Ok(())
}

impl ZedRuntime {
    async fn start(server: &Path, state_dir: &Path) -> Result<Self> {
        let runtime_dir = state_dir.join("server");
        tokio::fs::create_dir_all(&runtime_dir).await?;
        let log = runtime_dir.join("server.log");
        let pid = runtime_dir.join("server.pid");
        let input_socket = runtime_dir.join("stdin.sock");
        let output_socket = runtime_dir.join("stdout.sock");
        let error_socket = runtime_dir.join("stderr.sock");
        for path in [&pid, &input_socket, &output_socket, &error_socket] {
            if path.exists() {
                tokio::fs::remove_file(path).await?;
            }
        }

        let mut child = Command::new(server)
            .arg("run")
            .arg("--log-file")
            .arg(&log)
            .arg("--pid-file")
            .arg(&pid)
            .arg("--stdin-socket")
            .arg(&input_socket)
            .arg("--stdout-socket")
            .arg(&output_socket)
            .arg("--stderr-socket")
            .arg(&error_socket)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("failed to start {}", server.display()))?;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            if input_socket.exists() && output_socket.exists() && error_socket.exists() {
                break;
            }
            if let Some(status) = child.try_wait()? {
                bail!("Zed server exited during startup with {status}");
            }
            if tokio::time::Instant::now() >= deadline {
                bail!("Zed server did not create its protocol sockets");
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }

        let input = UnixStream::connect(&input_socket).await?;
        let output = UnixStream::connect(&output_socket).await?;
        let mut errors = UnixStream::connect(&error_socket).await?;
        tokio::spawn(async move {
            let _ = tokio::io::copy(&mut errors, &mut tokio::io::sink()).await;
        });

        Ok(Self {
            _child: child,
            input,
            output,
            next_message_id: 1,
            next_worktree_id: 1,
        })
    }

    async fn write(&mut self, envelope: proto::Envelope) -> Result<()> {
        let mut encoded = Vec::with_capacity(envelope.encoded_len());
        envelope.encode(&mut encoded)?;
        let length = u32::try_from(encoded.len()).context("Zed message is too large")?;
        self.input.write_all(&length.to_le_bytes()).await?;
        self.input.write_all(&encoded).await?;
        Ok(())
    }

    async fn read(&mut self) -> Result<proto::Envelope> {
        let length = self.output.read_u32_le().await?;
        if length > 16 * 1024 * 1024 {
            bail!("Zed message exceeds the 16 MiB adapter limit");
        }
        let mut encoded = vec![0; length as usize];
        self.output.read_exact(&mut encoded).await?;
        Ok(proto::Envelope::decode(encoded.as_slice())?)
    }

    fn message(
        &mut self,
        payload: proto::envelope::Payload,
        responding_to: Option<u32>,
    ) -> proto::Envelope {
        let id = self.next_message_id;
        self.next_message_id = self.next_message_id.wrapping_add(1).max(1);
        proto::Envelope {
            id,
            responding_to,
            payload: Some(payload),
            ..Default::default()
        }
    }

    async fn handle_server_request(&mut self, envelope: &proto::Envelope) -> Result<bool> {
        if matches!(
            envelope.payload,
            Some(proto::envelope::Payload::AllocateWorktreeId(_))
        ) {
            let worktree_id = self.next_worktree_id;
            self.next_worktree_id += 1;
            let response = self.message(
                proto::envelope::Payload::AllocateWorktreeIdResponse(
                    proto::AllocateWorktreeIdResponse { worktree_id },
                ),
                Some(envelope.id),
            );
            self.write(response).await?;
            return Ok(true);
        }
        Ok(false)
    }

    async fn request(&mut self, payload: proto::envelope::Payload) -> Result<proto::Envelope> {
        let request = self.message(payload, None);
        let request_id = request.id;
        self.write(request).await?;
        loop {
            let response = self.read().await?;
            if response.responding_to == Some(request_id) {
                if let Some(proto::envelope::Payload::Error(error)) = &response.payload {
                    bail!("Zed request failed: {}", error.message);
                }
                return Ok(response);
            }
            self.handle_server_request(&response).await?;
        }
    }

    async fn open_worktree(&mut self, path: &Path, trusted: bool) -> Result<(u64, WorktreeState)> {
        let response = self
            .request(proto::envelope::Payload::AddWorktree(proto::AddWorktree {
                path: path.to_string_lossy().into_owned(),
                project_id: proto::REMOTE_SERVER_PROJECT_ID,
                visible: true,
            }))
            .await?;
        let Some(proto::envelope::Payload::AddWorktreeResponse(response)) = response.payload else {
            bail!("Zed returned the wrong AddWorktree response");
        };
        let worktree_id = response.worktree_id;

        let state = self.set_trust(worktree_id, trusted).await?;
        Ok((worktree_id, state))
    }

    async fn set_trust(&mut self, worktree_id: u64, trusted: bool) -> Result<WorktreeState> {
        Ok(if trusted {
            let trust = proto::TrustWorktrees {
                project_id: proto::REMOTE_SERVER_PROJECT_ID,
                trusted_paths: vec![proto::PathTrust {
                    content: Some(proto::path_trust::Content::WorktreeId(worktree_id)),
                }],
            };
            let response = self
                .request(proto::envelope::Payload::TrustWorktrees(trust))
                .await?;
            if !matches!(response.payload, Some(proto::envelope::Payload::Ack(_))) {
                bail!("Zed returned the wrong TrustWorktrees response");
            }
            WorktreeState::Ready
        } else {
            let restrict = proto::RestrictWorktrees {
                project_id: proto::REMOTE_SERVER_PROJECT_ID,
                worktree_ids: vec![worktree_id],
            };
            let response = self
                .request(proto::envelope::Payload::RestrictWorktrees(restrict))
                .await?;
            if !matches!(response.payload, Some(proto::envelope::Payload::Ack(_))) {
                bail!("Zed returned the wrong RestrictWorktrees response");
            }
            WorktreeState::Restricted
        })
    }

    async fn remove_worktree(&mut self, worktree_id: u64) -> Result<()> {
        let message = self.message(
            proto::envelope::Payload::RemoveWorktree(proto::RemoveWorktree { worktree_id }),
            None,
        );
        self.write(message).await
    }
}

async fn respond(request: Request, worktrees: &Worktrees, zed: Option<&Zed>) -> Result<Response> {
    Ok(match request {
        Request::Health => Response::Health {
            api_version: ADAPTER_VERSION,
            zed_version: ZED_VERSION,
            zed_revision: ZED_REVISION,
            worktrees: worktrees.read().await.len(),
        },
        Request::EnsureWorktree { path, trusted } => {
            ensure_worktree(path, trusted, false, worktrees, zed).await?
        }
        Request::OpenWorktree { path, trusted } => {
            ensure_worktree(path, trusted, true, worktrees, zed).await?
        }
        Request::CloseWorktree { path } => {
            let path = tokio::fs::canonicalize(path).await?;
            let mut all = worktrees.write().await;
            let Some(lease) = all.get_mut(&path) else {
                bail!("worktree is not open");
            };
            lease.leases = lease.leases.saturating_sub(1);
            let remote_id = lease.remote_id;
            let response = Response::Worktree {
                api_version: ADAPTER_VERSION,
                path: path.clone(),
                state: lease.state,
                leases: lease.leases,
            };
            if lease.leases == 0 {
                all.remove(&path);
                if let Some(zed) = zed {
                    zed.lock().await.remove_worktree(remote_id).await?;
                }
            }
            response
        }
    })
}

async fn ensure_worktree(
    path: PathBuf,
    trusted: bool,
    acquire_lease: bool,
    worktrees: &Worktrees,
    zed: Option<&Zed>,
) -> Result<Response> {
    let path = tokio::fs::canonicalize(path).await?;
    if !path.is_dir() {
        bail!("worktree is not a directory");
    }
    let mut all = worktrees.write().await;
    if !all.contains_key(&path) {
        let (remote_id, state) = if let Some(zed) = zed {
            zed.lock().await.open_worktree(&path, trusted).await?
        } else {
            (
                u64::try_from(all.len() + 1)?,
                if trusted {
                    WorktreeState::Warming
                } else {
                    WorktreeState::Restricted
                },
            )
        };
        all.insert(
            path.clone(),
            WorktreeLease {
                state,
                leases: 0,
                remote_id,
            },
        );
    }
    let lease = all.get_mut(&path).expect("worktree was just inserted");
    if acquire_lease {
        lease.leases += 1;
    }
    if trusted && matches!(lease.state, WorktreeState::Restricted) {
        lease.state = if let Some(zed) = zed {
            zed.lock().await.set_trust(lease.remote_id, true).await?
        } else {
            WorktreeState::Warming
        };
    }
    Ok(Response::Worktree {
        api_version: ADAPTER_VERSION,
        path,
        state: lease.state,
        leases: lease.leases,
    })
}

async fn handle(stream: UnixStream, worktrees: Worktrees, zed: Option<Zed>) -> Result<()> {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    while let Some(line) = lines.next_line().await? {
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) => respond(request, &worktrees, zed.as_ref())
                .await
                .unwrap_or_else(|error| Response::Error {
                    api_version: ADAPTER_VERSION,
                    message: error.to_string(),
                }),
            Err(error) => Response::Error {
                api_version: ADAPTER_VERSION,
                message: error.to_string(),
            },
        };
        let mut encoded = serde_json::to_vec(&response)?;
        encoded.push(b'\n');
        write.write_all(&encoded).await?;
    }
    Ok(())
}

async fn serve(socket: PathBuf, zed_server: PathBuf, state_dir: PathBuf) -> Result<()> {
    verify_zed_server(&zed_server).await?;
    tokio::fs::create_dir_all(&state_dir).await?;
    if let Some(parent) = socket.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    if socket.exists() {
        tokio::fs::remove_file(&socket).await?;
    }
    let listener = UnixListener::bind(&socket)?;
    let worktrees: Worktrees = Arc::default();
    let zed = Arc::new(Mutex::new(
        ZedRuntime::start(&zed_server, &state_dir).await?,
    ));
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let worktrees = Arc::clone(&worktrees);
                let zed = Arc::clone(&zed);
                tokio::spawn(async move {
                    let _ = handle(stream, worktrees, Some(zed)).await;
                });
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    Ok(())
}

async fn probe(socket: PathBuf, wait: Duration) -> Result<()> {
    let deadline = tokio::time::Instant::now() + wait;
    let stream = loop {
        match UnixStream::connect(&socket).await {
            Ok(stream) => break stream,
            Err(_) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to connect to {}", socket.display()));
            }
        }
    };
    let (read, mut write) = stream.into_split();
    let mut request = serde_json::to_vec(&Request::Health)?;
    request.push(b'\n');
    write.write_all(&request).await?;

    let mut line = String::new();
    BufReader::new(read).read_line(&mut line).await?;
    let response: serde_json::Value = serde_json::from_str(&line)?;
    if response.get("type").and_then(serde_json::Value::as_str) != Some("health")
        || response
            .get("api_version")
            .and_then(serde_json::Value::as_u64)
            != Some(u64::from(ADAPTER_VERSION))
    {
        bail!("adapter returned an incompatible health response");
    }
    println!("{line}", line = line.trim_end());
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        CommandKind::Serve {
            socket,
            zed_server,
            state_dir,
        } => serve(socket, zed_server, state_dir).await,
        CommandKind::Probe { socket, wait_ms } => {
            probe(socket, Duration::from_millis(wait_ms)).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn worktree_leases_are_reference_counted_and_trust_is_monotonic() {
        let root = std::env::current_dir().unwrap();
        let canonical = tokio::fs::canonicalize(&root).await.unwrap();
        let worktrees: Worktrees = Arc::default();

        let first = respond(
            Request::OpenWorktree {
                path: root.clone(),
                trusted: false,
            },
            &worktrees,
            None,
        )
        .await
        .unwrap();
        assert!(matches!(
            first,
            Response::Worktree {
                state: WorktreeState::Restricted,
                leases: 1,
                ..
            }
        ));

        let second = respond(
            Request::OpenWorktree {
                path: root.clone(),
                trusted: true,
            },
            &worktrees,
            None,
        )
        .await
        .unwrap();
        assert!(matches!(
            second,
            Response::Worktree {
                state: WorktreeState::Warming,
                leases: 2,
                ..
            }
        ));

        let _ = respond(
            Request::CloseWorktree { path: root.clone() },
            &worktrees,
            None,
        )
        .await
        .unwrap();
        assert_eq!(worktrees.read().await.get(&canonical).unwrap().leases, 1);
        let _ = respond(Request::CloseWorktree { path: root }, &worktrees, None)
            .await
            .unwrap();
        assert!(worktrees.read().await.is_empty());
    }

    #[tokio::test]
    async fn ensure_worktree_is_idempotent_and_does_not_acquire_a_lease() {
        let root = std::env::current_dir().unwrap();
        let worktrees: Worktrees = Arc::default();
        for _ in 0..2 {
            let response = respond(
                Request::EnsureWorktree {
                    path: root.clone(),
                    trusted: true,
                },
                &worktrees,
                None,
            )
            .await
            .unwrap();
            assert!(matches!(
                response,
                Response::Worktree {
                    state: WorktreeState::Warming,
                    leases: 0,
                    ..
                }
            ));
        }
        assert_eq!(worktrees.read().await.len(), 1);
    }

    #[tokio::test]
    async fn missing_worktree_fails_closed() {
        let worktrees: Worktrees = Arc::default();
        let result = respond(
            Request::OpenWorktree {
                path: PathBuf::from("/definitely/missing/cowboy-zed-worktree"),
                trusted: true,
            },
            &worktrees,
            None,
        )
        .await;
        assert!(result.is_err());
        assert!(worktrees.read().await.is_empty());
    }

    #[tokio::test]
    async fn probe_validates_the_socket_protocol() {
        let socket = std::env::temp_dir().join(format!(
            "cowboy-zed-adapter-test-{}-{}.sock",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let listener = UnixListener::bind(&socket).unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            handle(stream, Arc::default(), None).await.unwrap();
        });

        probe(socket.clone(), Duration::ZERO).await.unwrap();
        server.await.unwrap();
        tokio::fs::remove_file(socket).await.unwrap();
    }

    #[tokio::test]
    async fn probe_waits_for_a_late_socket() {
        let socket = std::env::temp_dir().join(format!(
            "cowboy-zed-adapter-late-test-{}-{}.sock",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let server_socket = socket.clone();
        let server = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let listener = UnixListener::bind(&server_socket).unwrap();
            let (stream, _) = listener.accept().await.unwrap();
            handle(stream, Arc::default(), None).await.unwrap();
        });

        probe(socket.clone(), Duration::from_secs(1)).await.unwrap();
        server.await.unwrap();
        tokio::fs::remove_file(socket).await.unwrap();
    }
}
