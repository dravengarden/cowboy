use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::process::Command;
use tokio::sync::RwLock;

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
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Request {
    Health,
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
}

#[derive(Clone, Copy)]
struct WorktreeLease {
    state: WorktreeState,
    leases: usize,
}

type Worktrees = Arc<RwLock<HashMap<PathBuf, WorktreeLease>>>;

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

async fn respond(request: Request, worktrees: &Worktrees) -> Result<Response> {
    Ok(match request {
        Request::Health => Response::Health {
            api_version: ADAPTER_VERSION,
            zed_version: ZED_VERSION,
            zed_revision: ZED_REVISION,
            worktrees: worktrees.read().await.len(),
        },
        Request::OpenWorktree { path, trusted } => {
            let path = tokio::fs::canonicalize(path).await?;
            if !path.is_dir() {
                bail!("worktree is not a directory");
            }
            let mut all = worktrees.write().await;
            let lease = all.entry(path.clone()).or_insert(WorktreeLease {
                state: if trusted {
                    WorktreeState::Warming
                } else {
                    WorktreeState::Restricted
                },
                leases: 0,
            });
            lease.leases += 1;
            if trusted {
                lease.state = WorktreeState::Warming;
            }
            Response::Worktree {
                api_version: ADAPTER_VERSION,
                path,
                state: lease.state,
                leases: lease.leases,
            }
        }
        Request::CloseWorktree { path } => {
            let path = tokio::fs::canonicalize(path).await?;
            let mut all = worktrees.write().await;
            let Some(lease) = all.get_mut(&path) else {
                bail!("worktree is not open");
            };
            lease.leases = lease.leases.saturating_sub(1);
            let response = Response::Worktree {
                api_version: ADAPTER_VERSION,
                path: path.clone(),
                state: lease.state,
                leases: lease.leases,
            };
            if lease.leases == 0 {
                all.remove(&path);
            }
            response
        }
    })
}

async fn handle(stream: UnixStream, worktrees: Worktrees) -> Result<()> {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    while let Some(line) = lines.next_line().await? {
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) => {
                respond(request, &worktrees)
                    .await
                    .unwrap_or_else(|error| Response::Error {
                        api_version: ADAPTER_VERSION,
                        message: error.to_string(),
                    })
            }
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
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let worktrees = Arc::clone(&worktrees);
                tokio::spawn(async move {
                    let _ = handle(stream, worktrees).await;
                });
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    Ok(())
}

async fn probe(socket: PathBuf) -> Result<()> {
    let stream = UnixStream::connect(&socket)
        .await
        .with_context(|| format!("failed to connect to {}", socket.display()))?;
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
        CommandKind::Probe { socket } => probe(socket).await,
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

        let _ = respond(Request::CloseWorktree { path: root.clone() }, &worktrees)
            .await
            .unwrap();
        assert_eq!(worktrees.read().await.get(&canonical).unwrap().leases, 1);
        let _ = respond(Request::CloseWorktree { path: root }, &worktrees)
            .await
            .unwrap();
        assert!(worktrees.read().await.is_empty());
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
            handle(stream, Arc::default()).await.unwrap();
        });

        probe(socket.clone()).await.unwrap();
        server.await.unwrap();
        tokio::fs::remove_file(socket).await.unwrap();
    }
}
