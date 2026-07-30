use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use clap::{Parser, Subcommand};
use proto::Message as _;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, RwLock, broadcast, mpsc, oneshot};

const ADAPTER_VERSION: u8 = 1;
const ZED_VERSION: &str = "1.13.0";
const ZED_REVISION: &str = "aaf5f57dd36c41cf2ed49b13bcb091d52d5aef45";
const MAX_DIAGNOSTICS: usize = 1_000;
const MAX_INLAY_HINTS: usize = 2_000;
const MAX_SEMANTIC_TOKEN_WORDS: usize = 50_000;
const MAX_DOCUMENT_SYMBOLS: usize = 2_000;

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
    EnsureWorktree {
        path: PathBuf,
        trusted: bool,
    },
    OpenWorktree {
        path: PathBuf,
        trusted: bool,
    },
    CloseWorktree {
        path: PathBuf,
    },
    OpenBuffer {
        worktree: PathBuf,
        path: PathBuf,
        #[serde(rename = "leaseId")]
        lease_id: String,
    },
    CloseBuffer {
        worktree: PathBuf,
        path: PathBuf,
        #[serde(rename = "leaseId")]
        lease_id: String,
    },
    BufferLanguage {
        worktree: PathBuf,
        path: PathBuf,
    },
    BufferHover {
        worktree: PathBuf,
        path: PathBuf,
        row: u32,
        column: u32,
    },
    BufferNavigate {
        worktree: PathBuf,
        path: PathBuf,
        row: u32,
        column: u32,
        kind: NavigationKind,
    },
    BufferSymbols {
        worktree: PathBuf,
        path: PathBuf,
    },
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
    Buffer {
        api_version: u8,
        worktree: PathBuf,
        path: PathBuf,
        leases: usize,
        buffer_id: u64,
        version: Vec<BufferVersionEntry>,
    },
    BufferLanguage {
        api_version: u8,
        worktree: PathBuf,
        path: PathBuf,
        version: Vec<BufferVersionEntry>,
        diagnostics: Vec<LanguageDiagnostic>,
        inlay_hints: Vec<LanguageInlayHint>,
        semantic_tokens: Vec<u32>,
    },
    BufferHover {
        api_version: u8,
        worktree: PathBuf,
        path: PathBuf,
        contents: Vec<LanguageHoverBlock>,
    },
    BufferNavigation {
        api_version: u8,
        worktree: PathBuf,
        path: PathBuf,
        locations: Vec<LanguageLocation>,
    },
    BufferSymbols {
        api_version: u8,
        worktree: PathBuf,
        path: PathBuf,
        symbols: Vec<LanguageDocumentSymbol>,
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
struct BufferLease {
    lease_ids: HashSet<String>,
    remote_id: u64,
    version: Vec<BufferVersionEntry>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BufferVersionEntry {
    replica_id: u32,
    timestamp: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LanguagePoint {
    row: u32,
    column: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LanguageDiagnostic {
    start: LanguagePoint,
    end: LanguagePoint,
    severity: i32,
    source: Option<String>,
    message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LanguageInlayHint {
    offset: u64,
    label: String,
    kind: Option<String>,
    padding_left: bool,
    padding_right: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LanguageHoverBlock {
    text: String,
    language: Option<String>,
    markdown: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
enum NavigationKind {
    Definition,
    Declaration,
    TypeDefinition,
    Implementation,
    References,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LanguageLocation {
    path: PathBuf,
    start: LanguagePoint,
    end: LanguagePoint,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LanguageDocumentSymbol {
    name: String,
    kind: i32,
    start: LanguagePoint,
    end: LanguagePoint,
    selection_start: LanguagePoint,
    selection_end: LanguagePoint,
    children: Vec<LanguageDocumentSymbol>,
}

type Buffers = Arc<RwLock<HashMap<(PathBuf, PathBuf), BufferLease>>>;
type BufferFiles = Arc<RwLock<HashMap<u64, proto::File>>>;
type WorktreePaths = Arc<RwLock<HashMap<u64, PathBuf>>>;
type Zed = Arc<ZedRuntime>;
type PendingRequests = Arc<Mutex<HashMap<u32, oneshot::Sender<proto::Envelope>>>>;

struct ZedRuntime {
    _child: Mutex<Child>,
    outbound: mpsc::UnboundedSender<proto::Envelope>,
    pending: PendingRequests,
    events: broadcast::Sender<proto::Envelope>,
    buffer_files: BufferFiles,
    worktree_paths: WorktreePaths,
    next_message_id: AtomicU32,
    next_lsp_request_id: AtomicU64,
}

struct ExtensionManifest {
    id: String,
    version: String,
}

fn extension_manifest(contents: &str) -> Result<ExtensionManifest> {
    fn string_field(contents: &str, wanted: &str) -> Option<String> {
        contents
            .lines()
            .map(str::trim)
            .take_while(|line| !line.starts_with('['))
            .filter_map(|line| line.split_once('='))
            .find_map(|(key, value)| {
                (key.trim() == wanted).then(|| {
                    value
                        .trim()
                        .strip_prefix('"')?
                        .strip_suffix('"')
                        .map(str::to_owned)
                })?
            })
    }
    let id = string_field(contents, "id").context("extension manifest has no string id")?;
    let version =
        string_field(contents, "version").context("extension manifest has no string version")?;
    Ok(ExtensionManifest { id, version })
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

        let (outbound, outbound_rx) = mpsc::unbounded_channel();
        let pending = PendingRequests::default();
        let (events, _) = broadcast::channel(1_024);
        let buffer_files = BufferFiles::default();
        let worktree_paths = WorktreePaths::default();
        tokio::spawn(write_messages(input, outbound_rx));
        tokio::spawn(read_messages(
            output,
            outbound.clone(),
            Arc::clone(&pending),
            events.clone(),
            Arc::clone(&buffer_files),
        ));
        outbound
            .send(proto::Envelope {
                id: 1,
                payload: Some(proto::envelope::Payload::RemoteStarted(
                    proto::RemoteStarted {},
                )),
                ..Default::default()
            })
            .map_err(|_| anyhow::anyhow!("Zed writer task stopped during handshake"))?;

        Ok(Self {
            _child: Mutex::new(child),
            outbound,
            pending,
            events,
            buffer_files,
            worktree_paths,
            next_message_id: AtomicU32::new(2),
            next_lsp_request_id: AtomicU64::new(1),
        })
    }

    fn message(
        &self,
        payload: proto::envelope::Payload,
        responding_to: Option<u32>,
    ) -> proto::Envelope {
        let id = self.next_message_id.fetch_add(1, Ordering::Relaxed);
        proto::Envelope {
            id: id.max(1),
            responding_to,
            payload: Some(payload),
            ..Default::default()
        }
    }

    fn send(&self, envelope: proto::Envelope) -> Result<()> {
        self.outbound
            .send(envelope)
            .map_err(|_| anyhow::anyhow!("Zed writer task stopped"))
    }

    async fn sync_extensions(&self, data_home: &Path) -> Result<usize> {
        let root = data_home.join("zed/remote_extensions");
        let mut directory = match tokio::fs::read_dir(&root).await {
            Ok(directory) => directory,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(error.into()),
        };
        let mut extensions = Vec::new();
        while let Some(entry) = directory.next_entry().await? {
            if !entry.file_type().await?.is_dir() {
                continue;
            }
            let manifest_path = entry.path().join("extension.toml");
            // Zed may leave a partially downloaded extension directory behind
            // (manifest present, wasm absent). Advertising it makes the server
            // reject the entire sync. Ignore it until the official Zed import
            // becomes complete on a later adapter restart.
            if !entry.path().join("extension.wasm").is_file() {
                continue;
            }
            let manifest = match tokio::fs::read_to_string(&manifest_path).await {
                Ok(contents) => extension_manifest(&contents)
                    .with_context(|| format!("invalid {}", manifest_path.display()))?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            extensions.push(proto::Extension {
                id: manifest.id,
                version: manifest.version,
                dev: false,
            });
        }
        extensions.sort_by(|left, right| left.id.cmp(&right.id));
        let count = extensions.len();
        let response = tokio::time::timeout(
            Duration::from_secs(30),
            self.request(proto::envelope::Payload::SyncExtensions(
                proto::SyncExtensions { extensions },
            )),
        )
        .await
        .context("Zed extension sync timed out")??;
        let Some(proto::envelope::Payload::SyncExtensionsResponse(response)) = response.payload
        else {
            bail!("Zed returned the wrong SyncExtensions response");
        };
        if !response.missing_extensions.is_empty() {
            let missing = response
                .missing_extensions
                .iter()
                .map(|extension| extension.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            bail!("Zed could not load extensions: {missing}");
        }
        Ok(count)
    }

    async fn request(&self, payload: proto::envelope::Payload) -> Result<proto::Envelope> {
        let request = self.message(payload, None);
        let request_id = request.id;
        let (response_tx, response_rx) = oneshot::channel();
        self.pending.lock().await.insert(request_id, response_tx);
        if let Err(error) = self.send(request) {
            self.pending.lock().await.remove(&request_id);
            return Err(error);
        }
        let response = match tokio::time::timeout(Duration::from_secs(30), response_rx).await {
            Ok(Ok(response)) => response,
            Ok(Err(_)) => bail!("Zed reader task stopped while waiting for request {request_id}"),
            Err(_) => {
                self.pending.lock().await.remove(&request_id);
                bail!("Zed request {request_id} timed out");
            }
        };
        if let Some(proto::envelope::Payload::Error(error)) = &response.payload {
            bail!("Zed request failed: {}", error.message);
        }
        Ok(response)
    }

    async fn open_worktree(&self, path: &Path, trusted: bool) -> Result<(u64, WorktreeState)> {
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
        self.worktree_paths
            .write()
            .await
            .insert(worktree_id, path.to_path_buf());

        let state = self.set_trust(worktree_id, trusted).await?;
        Ok((worktree_id, state))
    }

    async fn set_trust(&self, worktree_id: u64, trusted: bool) -> Result<WorktreeState> {
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

    fn remove_worktree(&self, worktree_id: u64) -> Result<()> {
        let message = self.message(
            proto::envelope::Payload::RemoveWorktree(proto::RemoveWorktree { worktree_id }),
            None,
        );
        self.send(message)
    }

    async fn open_buffer(
        &self,
        worktree_id: u64,
        path: &Path,
    ) -> Result<(u64, Vec<BufferVersionEntry>)> {
        let mut opened = None;
        for attempt in 0..2 {
            let mut events = self.events.subscribe();
            let response = self
                .request(proto::envelope::Payload::OpenBufferByPath(
                    proto::OpenBufferByPath {
                        project_id: proto::REMOTE_SERVER_PROJECT_ID,
                        worktree_id,
                        path: path.to_string_lossy().into_owned(),
                    },
                ))
                .await?;
            let Some(proto::envelope::Payload::OpenBufferResponse(response)) = response.payload
            else {
                bail!("Zed returned the wrong OpenBufferByPath response");
            };
            let buffer_id = response.buffer_id;
            let version = tokio::time::timeout(Duration::from_secs(5), async {
                let mut version = HashMap::<u32, u32>::new();
                let mut received_state = false;
                loop {
                    let envelope = events.recv().await?;
                    let Some(proto::envelope::Payload::CreateBufferForPeer(message)) =
                        envelope.payload
                    else {
                        continue;
                    };
                    match message.variant {
                        Some(proto::create_buffer_for_peer::Variant::State(state))
                            if state.id == buffer_id =>
                        {
                            received_state = true;
                            merge_version(&mut version, state.saved_version);
                        }
                        Some(proto::create_buffer_for_peer::Variant::Chunk(chunk))
                            if chunk.buffer_id == buffer_id && received_state =>
                        {
                            for operation in chunk.operations {
                                merge_operation_version(&mut version, operation);
                            }
                            if chunk.is_last {
                                let mut version = version
                                    .into_iter()
                                    .map(|(replica_id, timestamp)| BufferVersionEntry {
                                        replica_id,
                                        timestamp,
                                    })
                                    .collect::<Vec<_>>();
                                version.sort_by_key(|entry| entry.replica_id);
                                break anyhow::Ok(version);
                            }
                        }
                        _ => {}
                    }
                }
            })
            .await;
            match version {
                Ok(Ok(version)) => {
                    opened = Some((buffer_id, version));
                    break;
                }
                Ok(Err(error)) if attempt == 1 => {
                    return Err(error).context("Zed did not publish the initial buffer state");
                }
                Err(error) if attempt == 1 => {
                    return Err(error).context("Zed did not publish the initial buffer state");
                }
                Ok(Err(_)) | Err(_) => {
                    // CloseBuffer is foreground work while OpenBufferByPath is
                    // background work in Zed's protocol. Sending the close
                    // before retrying therefore clears a stale shared-buffer
                    // registration before the second open is handled.
                    self.close_buffer(buffer_id)?;
                }
            }
        }
        let (buffer_id, version) =
            opened.context("Zed did not publish the initial buffer state")?;
        // Opening shares the buffer contents but, like Zed's own remote client,
        // the peer must explicitly register that buffer with the headless
        // project's language servers. Without this request every later LSP
        // query is valid yet has no servers to answer it.
        let response = self
            .request(proto::envelope::Payload::RegisterBufferWithLanguageServers(
                proto::RegisterBufferWithLanguageServers {
                    project_id: proto::REMOTE_SERVER_PROJECT_ID,
                    buffer_id,
                    only_servers: Vec::new(),
                },
            ))
            .await?;
        if !matches!(response.payload, Some(proto::envelope::Payload::Ack(_))) {
            bail!("Zed returned the wrong RegisterBufferWithLanguageServers response");
        }
        Ok((buffer_id, version))
    }

    fn close_buffer(&self, buffer_id: u64) -> Result<()> {
        let message = self.message(
            proto::envelope::Payload::CloseBuffer(proto::CloseBuffer {
                project_id: proto::REMOTE_SERVER_PROJECT_ID,
                buffer_id,
            }),
            None,
        );
        self.send(message)
    }

    async fn language(
        &self,
        buffer_id: u64,
        version: &[BufferVersionEntry],
    ) -> Result<(Vec<LanguageDiagnostic>, Vec<LanguageInlayHint>, Vec<u32>)> {
        let diagnostics_request = self.lsp_query(
            proto::lsp_query::Request::GetDocumentDiagnostics(proto::GetDocumentDiagnostics {
                project_id: proto::REMOTE_SERVER_PROJECT_ID,
                buffer_id,
                version: proto_version(version),
            }),
        );
        let inlay_request =
            self.lsp_query(proto::lsp_query::Request::InlayHints(proto::InlayHints {
                project_id: proto::REMOTE_SERVER_PROJECT_ID,
                buffer_id,
                start: Some(proto::Anchor {
                    replica_id: u32::MIN,
                    timestamp: u32::MIN,
                    offset: u64::MIN,
                    bias: proto::Bias::Left as i32,
                    buffer_id: Some(buffer_id),
                }),
                end: Some(proto::Anchor {
                    replica_id: u16::MAX.into(),
                    timestamp: u32::MAX,
                    offset: u64::from(u32::MAX),
                    bias: proto::Bias::Right as i32,
                    buffer_id: Some(buffer_id),
                }),
                version: proto_version(version),
            }));
        let semantic_request = self.lsp_query(proto::lsp_query::Request::SemanticTokens(
            proto::SemanticTokens {
                project_id: proto::REMOTE_SERVER_PROJECT_ID,
                buffer_id,
                for_server: None,
                version: proto_version(version),
            },
        ));
        let (diagnostics, inlay_hints, semantic_tokens) =
            tokio::join!(diagnostics_request, inlay_request, semantic_request);

        let diagnostics = diagnostics
            .unwrap_or_default()
            .into_iter()
            .filter_map(|response| match response.response? {
                proto::lsp_response::Response::GetDocumentDiagnosticsResponse(value) => Some(value),
                _ => None,
            })
            .flat_map(|response| response.pulled_diagnostics)
            .flat_map(|pulled| pulled.diagnostics)
            .filter_map(|diagnostic| {
                Some(LanguageDiagnostic {
                    start: language_point(&diagnostic.start?),
                    end: language_point(&diagnostic.end?),
                    severity: diagnostic.severity,
                    source: diagnostic.source,
                    message: diagnostic.message,
                })
            })
            .take(MAX_DIAGNOSTICS)
            .collect();

        let inlay_hints = inlay_hints
            .unwrap_or_default()
            .into_iter()
            .filter_map(|response| match response.response? {
                proto::lsp_response::Response::InlayHintsResponse(value) => Some(value),
                _ => None,
            })
            .flat_map(|response| response.hints)
            .filter_map(language_inlay_hint)
            .take(MAX_INLAY_HINTS)
            .collect();

        let semantic_tokens = semantic_tokens
            .unwrap_or_default()
            .into_iter()
            .filter_map(|response| match response.response? {
                proto::lsp_response::Response::SemanticTokensResponse(value) => Some(value),
                _ => None,
            })
            .flat_map(|response| response.data)
            .take(MAX_SEMANTIC_TOKEN_WORDS)
            .collect();

        Ok((diagnostics, inlay_hints, semantic_tokens))
    }

    async fn hover(
        &self,
        buffer_id: u64,
        version: &[BufferVersionEntry],
        offset: u64,
    ) -> Result<Vec<LanguageHoverBlock>> {
        let responses = self
            .lsp_query(proto::lsp_query::Request::GetHover(proto::GetHover {
                project_id: proto::REMOTE_SERVER_PROJECT_ID,
                buffer_id,
                position: Some(position_anchor(buffer_id, offset)),
                version: proto_version(version),
            }))
            .await?;
        Ok(responses
            .into_iter()
            .filter_map(|response| match response.response? {
                proto::lsp_response::Response::GetHoverResponse(value) => Some(value),
                _ => None,
            })
            .flat_map(|response| response.contents)
            .map(|block| LanguageHoverBlock {
                text: block.text,
                language: block.language,
                markdown: block.is_markdown,
            })
            .take(32)
            .collect())
    }

    async fn navigate(
        &self,
        buffer_id: u64,
        version: &[BufferVersionEntry],
        offset: u64,
        kind: NavigationKind,
    ) -> Result<Vec<LanguageLocation>> {
        let responses = self
            .lsp_query(navigation_request(buffer_id, version, offset, kind))
            .await?;
        let locations = responses
            .into_iter()
            .flat_map(|response| match response.response {
                Some(proto::lsp_response::Response::GetDefinitionResponse(value)) => value
                    .links
                    .into_iter()
                    .filter_map(|link| link.target)
                    .collect(),
                Some(proto::lsp_response::Response::GetDeclarationResponse(value)) => value
                    .links
                    .into_iter()
                    .filter_map(|link| link.target)
                    .collect(),
                Some(proto::lsp_response::Response::GetTypeDefinitionResponse(value)) => value
                    .links
                    .into_iter()
                    .filter_map(|link| link.target)
                    .collect(),
                Some(proto::lsp_response::Response::GetImplementationResponse(value)) => value
                    .links
                    .into_iter()
                    .filter_map(|link| link.target)
                    .collect(),
                Some(proto::lsp_response::Response::GetReferencesResponse(value)) => {
                    value.locations
                }
                _ => Vec::new(),
            })
            .take(256)
            .collect::<Vec<_>>();
        let files = self.buffer_files.read().await;
        let worktrees = self.worktree_paths.read().await;
        let mut result = Vec::with_capacity(locations.len());
        for location in locations {
            let Some(file) = files.get(&location.buffer_id) else {
                continue;
            };
            let Some(start) = location.start else {
                continue;
            };
            let end = location.end.unwrap_or_else(|| start.clone());
            let Some(worktree) = worktrees.get(&file.worktree_id) else {
                continue;
            };
            let path = worktree.join(&file.path);
            let text = tokio::fs::read_to_string(&path).await.ok();
            let Some(text) = text else {
                continue;
            };
            let Some(start) = offset_to_utf16_point(&text, start.offset) else {
                continue;
            };
            let Some(end) = offset_to_utf16_point(&text, end.offset) else {
                continue;
            };
            result.push(LanguageLocation { path, start, end });
        }
        Ok(result)
    }

    async fn document_symbols(
        &self,
        buffer_id: u64,
        version: &[BufferVersionEntry],
    ) -> Result<Vec<LanguageDocumentSymbol>> {
        let responses = self
            .lsp_query(proto::lsp_query::Request::GetDocumentSymbols(
                proto::GetDocumentSymbols {
                    project_id: proto::REMOTE_SERVER_PROJECT_ID,
                    buffer_id,
                    version: proto_version(version),
                },
            ))
            .await?;
        let mut remaining = MAX_DOCUMENT_SYMBOLS;
        Ok(responses
            .into_iter()
            .filter_map(|response| match response.response? {
                proto::lsp_response::Response::GetDocumentSymbolsResponse(value) => Some(value),
                _ => None,
            })
            .flat_map(|response| response.symbols)
            .filter_map(|symbol| document_symbol(symbol, &mut remaining))
            .collect())
    }

    async fn lsp_query(
        &self,
        request: proto::lsp_query::Request,
    ) -> Result<Vec<proto::LspResponse>> {
        let lsp_request_id = self.next_lsp_request_id.fetch_add(1, Ordering::Relaxed);
        let mut events = self.events.subscribe();
        let response = self
            .request(proto::envelope::Payload::LspQuery(proto::LspQuery {
                project_id: proto::REMOTE_SERVER_PROJECT_ID,
                lsp_request_id,
                server_id: None,
                request: Some(request),
            }))
            .await?;
        if !matches!(response.payload, Some(proto::envelope::Payload::Ack(_))) {
            bail!("Zed returned the wrong LspQuery acknowledgement");
        }
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let envelope = events.recv().await?;
                if let Some(proto::envelope::Payload::LspQueryResponse(response)) = envelope.payload
                    && response.lsp_request_id == lsp_request_id
                {
                    break anyhow::Ok(response.responses);
                }
            }
        })
        .await
        .context("Zed language query timed out")?
    }
}

fn navigation_request(
    buffer_id: u64,
    version: &[BufferVersionEntry],
    offset: u64,
    kind: NavigationKind,
) -> proto::lsp_query::Request {
    let position = Some(position_anchor(buffer_id, offset));
    let version = proto_version(version);
    match kind {
        NavigationKind::Definition => {
            proto::lsp_query::Request::GetDefinition(proto::GetDefinition {
                project_id: proto::REMOTE_SERVER_PROJECT_ID,
                buffer_id,
                position,
                version,
            })
        }
        NavigationKind::Declaration => {
            proto::lsp_query::Request::GetDeclaration(proto::GetDeclaration {
                project_id: proto::REMOTE_SERVER_PROJECT_ID,
                buffer_id,
                position,
                version,
            })
        }
        NavigationKind::TypeDefinition => {
            proto::lsp_query::Request::GetTypeDefinition(proto::GetTypeDefinition {
                project_id: proto::REMOTE_SERVER_PROJECT_ID,
                buffer_id,
                position,
                version,
            })
        }
        NavigationKind::Implementation => {
            proto::lsp_query::Request::GetImplementation(proto::GetImplementation {
                project_id: proto::REMOTE_SERVER_PROJECT_ID,
                buffer_id,
                position,
                version,
            })
        }
        NavigationKind::References => {
            proto::lsp_query::Request::GetReferences(proto::GetReferences {
                project_id: proto::REMOTE_SERVER_PROJECT_ID,
                buffer_id,
                position,
                version,
            })
        }
    }
}

fn position_anchor(buffer_id: u64, offset: u64) -> proto::Anchor {
    // Zed initializes every disk buffer's base insertion at Lamport
    // { replica: LOCAL (0), value: 1 }. MIN/MAX are boundary sentinels and
    // panic when paired with an interior offset.
    proto::Anchor {
        replica_id: 0,
        timestamp: 1,
        offset,
        bias: proto::Bias::Left as i32,
        buffer_id: Some(buffer_id),
    }
}

fn merge_version(
    current: &mut HashMap<u32, u32>,
    entries: impl IntoIterator<Item = proto::VectorClockEntry>,
) {
    for entry in entries {
        current
            .entry(entry.replica_id)
            .and_modify(|timestamp| *timestamp = (*timestamp).max(entry.timestamp))
            .or_insert(entry.timestamp);
    }
}

fn merge_operation_version(current: &mut HashMap<u32, u32>, operation: proto::Operation) {
    use proto::operation::Variant;
    match operation.variant {
        Some(Variant::Edit(edit)) => {
            merge_version(current, edit.version);
            current
                .entry(edit.replica_id)
                .and_modify(|value| *value = (*value).max(edit.lamport_timestamp))
                .or_insert(edit.lamport_timestamp);
        }
        Some(Variant::Undo(undo)) => {
            merge_version(current, undo.version);
            current
                .entry(undo.replica_id)
                .and_modify(|value| *value = (*value).max(undo.lamport_timestamp))
                .or_insert(undo.lamport_timestamp);
        }
        _ => {}
    }
}

fn proto_version(version: &[BufferVersionEntry]) -> Vec<proto::VectorClockEntry> {
    version
        .iter()
        .map(|entry| proto::VectorClockEntry {
            replica_id: entry.replica_id,
            timestamp: entry.timestamp,
        })
        .collect()
}

fn language_point(point: &proto::PointUtf16) -> LanguagePoint {
    LanguagePoint {
        row: point.row,
        column: point.column,
    }
}

fn document_symbol(
    symbol: proto::DocumentSymbol,
    remaining: &mut usize,
) -> Option<LanguageDocumentSymbol> {
    if *remaining == 0 {
        return None;
    }
    *remaining -= 1;
    Some(LanguageDocumentSymbol {
        name: symbol.name,
        kind: symbol.kind,
        start: language_point(&symbol.start?),
        end: language_point(&symbol.end?),
        selection_start: language_point(&symbol.selection_start?),
        selection_end: language_point(&symbol.selection_end?),
        children: symbol
            .children
            .into_iter()
            .filter_map(|child| document_symbol(child, remaining))
            .collect(),
    })
}

fn language_inlay_hint(hint: proto::InlayHint) -> Option<LanguageInlayHint> {
    let label = match hint.label?.label? {
        proto::inlay_hint_label::Label::Value(value) => value,
        proto::inlay_hint_label::Label::LabelParts(parts) => parts
            .parts
            .into_iter()
            .map(|part| part.value)
            .collect::<String>(),
    };
    Some(LanguageInlayHint {
        offset: hint.position?.offset,
        label,
        kind: hint.kind,
        padding_left: hint.padding_left,
        padding_right: hint.padding_right,
    })
}

async fn write_messages(
    mut input: UnixStream,
    mut messages: mpsc::UnboundedReceiver<proto::Envelope>,
) {
    while let Some(envelope) = messages.recv().await {
        let result = async {
            let mut encoded = Vec::with_capacity(envelope.encoded_len());
            envelope.encode(&mut encoded)?;
            let length = u32::try_from(encoded.len()).context("Zed message is too large")?;
            input.write_all(&length.to_le_bytes()).await?;
            input.write_all(&encoded).await?;
            anyhow::Ok(())
        }
        .await;
        if result.is_err() {
            break;
        }
    }
}

async fn read_messages(
    mut output: UnixStream,
    outbound: mpsc::UnboundedSender<proto::Envelope>,
    pending: PendingRequests,
    events: broadcast::Sender<proto::Envelope>,
    buffer_files: BufferFiles,
) {
    let next_message_id = AtomicU32::new(1_000_000_000);
    let next_worktree_id = AtomicU64::new(1);
    loop {
        let result = async {
            let length = output.read_u32_le().await?;
            if length > 16 * 1024 * 1024 {
                bail!("Zed message exceeds the 16 MiB adapter limit");
            }
            let mut encoded = vec![0; length as usize];
            output.read_exact(&mut encoded).await?;
            anyhow::Ok(proto::Envelope::decode(encoded.as_slice())?)
        }
        .await;
        let Ok(envelope) = result else {
            pending.lock().await.clear();
            break;
        };
        if std::env::var_os("COWBOY_ZED_TRACE").is_some() {
            eprintln!("Zed envelope: {envelope:?}");
        }

        if let Some(request_id) = envelope.responding_to
            && let Some(response) = pending.lock().await.remove(&request_id)
        {
            let _ = response.send(envelope);
            continue;
        }

        if matches!(
            envelope.payload,
            Some(proto::envelope::Payload::AllocateWorktreeId(_))
        ) {
            let response = proto::Envelope {
                id: next_message_id.fetch_add(1, Ordering::Relaxed),
                responding_to: Some(envelope.id),
                payload: Some(proto::envelope::Payload::AllocateWorktreeIdResponse(
                    proto::AllocateWorktreeIdResponse {
                        worktree_id: next_worktree_id.fetch_add(1, Ordering::Relaxed),
                    },
                )),
                ..Default::default()
            };
            if outbound.send(response).is_err() {
                break;
            }
            continue;
        }

        if matches!(
            envelope.payload,
            Some(proto::envelope::Payload::RemoteStarted(_) | proto::envelope::Payload::Ping(_))
        ) {
            let response = proto::Envelope {
                id: next_message_id.fetch_add(1, Ordering::Relaxed),
                responding_to: Some(envelope.id),
                payload: Some(proto::envelope::Payload::Ack(proto::Ack {})),
                ..Default::default()
            };
            if outbound.send(response).is_err() {
                break;
            }
            continue;
        }

        if let Some(proto::envelope::Payload::CreateBufferForPeer(message)) = &envelope.payload
            && let Some(proto::create_buffer_for_peer::Variant::State(state)) = &message.variant
            && let Some(file) = &state.file
        {
            buffer_files.write().await.insert(state.id, file.clone());
        }
        let _ = events.send(envelope);
    }
    // The adapter cannot serve valid requests after the owned Zed protocol
    // process or socket dies. Exit so systemd restarts the isolated pair.
    std::process::exit(1);
}

async fn respond(
    request: Request,
    worktrees: &Worktrees,
    buffers: &Buffers,
    zed: Option<&Zed>,
) -> Result<Response> {
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
                    zed.remove_worktree(remote_id)?;
                }
            }
            response
        }
        Request::OpenBuffer {
            worktree,
            path,
            lease_id,
        } => open_buffer(worktree, path, lease_id, worktrees, buffers, zed).await?,
        Request::CloseBuffer {
            worktree,
            path,
            lease_id,
        } => close_buffer(worktree, path, lease_id, buffers, zed).await?,
        Request::BufferLanguage { worktree, path } => {
            buffer_language(worktree, path, buffers, zed).await?
        }
        Request::BufferHover {
            worktree,
            path,
            row,
            column,
        } => buffer_hover(worktree, path, row, column, buffers, zed).await?,
        Request::BufferNavigate {
            worktree,
            path,
            row,
            column,
            kind,
        } => buffer_navigate(worktree, path, row, column, kind, buffers, zed).await?,
        Request::BufferSymbols { worktree, path } => {
            buffer_symbols(worktree, path, buffers, zed).await?
        }
    })
}

async fn buffer_language(
    worktree: PathBuf,
    path: PathBuf,
    buffers: &Buffers,
    zed: Option<&Zed>,
) -> Result<Response> {
    let (worktree, path) = buffer_key(worktree, path).await?;
    let (buffer_id, version) = {
        let all = buffers.read().await;
        let lease = all
            .get(&(worktree.clone(), path.clone()))
            .context("buffer is not open")?;
        (lease.remote_id, lease.version.clone())
    };
    let (diagnostics, inlay_hints, semantic_tokens) = if let Some(zed) = zed {
        zed.language(buffer_id, &version).await?
    } else {
        (Vec::new(), Vec::new(), Vec::new())
    };
    Ok(Response::BufferLanguage {
        api_version: ADAPTER_VERSION,
        worktree,
        path,
        version,
        diagnostics,
        inlay_hints,
        semantic_tokens,
    })
}

fn utf16_point_to_offset(text: &str, row: u32, column: u32) -> Result<u64> {
    let mut line_start = 0usize;
    let mut lines = text.split_inclusive('\n');
    let line = lines
        .nth(usize::try_from(row)?)
        .context("hover row is outside the buffer")?;
    for preceding in text.split_inclusive('\n').take(usize::try_from(row)?) {
        line_start += preceding.len();
    }
    let line = line.strip_suffix('\n').unwrap_or(line);
    let mut utf16 = 0u32;
    let mut byte = 0usize;
    for character in line.chars() {
        if utf16 >= column {
            break;
        }
        let width = u32::try_from(character.len_utf16())?;
        if utf16 + width > column {
            bail!("hover column splits a UTF-16 character");
        }
        utf16 += width;
        byte += character.len_utf8();
    }
    if utf16 < column {
        bail!("hover column is outside the buffer");
    }
    Ok(u64::try_from(line_start + byte)?)
}

fn offset_to_utf16_point(text: &str, offset: u64) -> Option<LanguagePoint> {
    let offset = usize::try_from(offset).ok()?;
    if offset > text.len() || !text.is_char_boundary(offset) {
        return None;
    }
    let prefix = &text[..offset];
    let row = u32::try_from(prefix.bytes().filter(|byte| *byte == b'\n').count()).ok()?;
    let line = prefix.rsplit_once('\n').map_or(prefix, |(_, line)| line);
    let column = u32::try_from(line.encode_utf16().count()).ok()?;
    Some(LanguagePoint { row, column })
}

async fn buffer_hover(
    worktree: PathBuf,
    path: PathBuf,
    row: u32,
    column: u32,
    buffers: &Buffers,
    zed: Option<&Zed>,
) -> Result<Response> {
    let (worktree, path) = buffer_key(worktree, path).await?;
    let (buffer_id, version) = {
        let all = buffers.read().await;
        let lease = all
            .get(&(worktree.clone(), path.clone()))
            .context("buffer is not open")?;
        (lease.remote_id, lease.version.clone())
    };
    let text = tokio::fs::read_to_string(worktree.join(&path)).await?;
    let offset = utf16_point_to_offset(&text, row, column)?;
    let contents = if let Some(zed) = zed {
        zed.hover(buffer_id, &version, offset).await?
    } else {
        Vec::new()
    };
    Ok(Response::BufferHover {
        api_version: ADAPTER_VERSION,
        worktree,
        path,
        contents,
    })
}

async fn buffer_navigate(
    worktree: PathBuf,
    path: PathBuf,
    row: u32,
    column: u32,
    kind: NavigationKind,
    buffers: &Buffers,
    zed: Option<&Zed>,
) -> Result<Response> {
    let (worktree, path) = buffer_key(worktree, path).await?;
    let (buffer_id, version) = {
        let all = buffers.read().await;
        let lease = all
            .get(&(worktree.clone(), path.clone()))
            .context("buffer is not open")?;
        (lease.remote_id, lease.version.clone())
    };
    let text = tokio::fs::read_to_string(worktree.join(&path)).await?;
    let offset = utf16_point_to_offset(&text, row, column)?;
    let mut locations = if let Some(zed) = zed {
        zed.navigate(buffer_id, &version, offset, kind).await?
    } else {
        Vec::new()
    };
    for location in &mut locations {
        if let Ok(relative) = location.path.strip_prefix(&worktree) {
            location.path = relative.to_path_buf();
        }
    }
    locations.retain(|location| !location.path.is_absolute());
    Ok(Response::BufferNavigation {
        api_version: ADAPTER_VERSION,
        worktree,
        path,
        locations,
    })
}

async fn buffer_symbols(
    worktree: PathBuf,
    path: PathBuf,
    buffers: &Buffers,
    zed: Option<&Zed>,
) -> Result<Response> {
    let (worktree, path) = buffer_key(worktree, path).await?;
    let (buffer_id, version) = {
        let all = buffers.read().await;
        let lease = all
            .get(&(worktree.clone(), path.clone()))
            .context("buffer is not open")?;
        (lease.remote_id, lease.version.clone())
    };
    let symbols = if let Some(zed) = zed {
        zed.document_symbols(buffer_id, &version).await?
    } else {
        Vec::new()
    };
    Ok(Response::BufferSymbols {
        api_version: ADAPTER_VERSION,
        worktree,
        path,
        symbols,
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
            zed.open_worktree(&path, trusted).await?
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
            zed.set_trust(lease.remote_id, true).await?
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

async fn buffer_key(worktree: PathBuf, relative: PathBuf) -> Result<(PathBuf, PathBuf)> {
    let worktree = tokio::fs::canonicalize(worktree).await?;
    if relative.is_absolute()
        || relative
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        bail!("buffer path must be relative to its worktree");
    }
    let file = tokio::fs::canonicalize(worktree.join(&relative)).await?;
    if !file.starts_with(&worktree) || !file.is_file() {
        bail!("buffer path is outside the worktree or is not a file");
    }
    let relative = file.strip_prefix(&worktree)?.to_path_buf();
    Ok((worktree, relative))
}

async fn open_buffer(
    worktree: PathBuf,
    path: PathBuf,
    lease_id: String,
    worktrees: &Worktrees,
    buffers: &Buffers,
    zed: Option<&Zed>,
) -> Result<Response> {
    validate_lease_id(&lease_id)?;
    let (worktree, path) = buffer_key(worktree, path).await?;
    let worktree_id = worktrees
        .read()
        .await
        .get(&worktree)
        .context("worktree is not open")?
        .remote_id;
    let key = (worktree.clone(), path.clone());
    let mut all = buffers.write().await;
    if !all.contains_key(&key) {
        let (remote_id, version) = if let Some(zed) = zed {
            zed.open_buffer(worktree_id, &path).await?
        } else {
            (u64::try_from(all.len() + 1)?, Vec::new())
        };
        all.insert(
            key.clone(),
            BufferLease {
                lease_ids: HashSet::new(),
                remote_id,
                version,
            },
        );
    }
    let lease = all.get_mut(&key).expect("buffer was just inserted");
    lease.lease_ids.insert(lease_id);
    Ok(Response::Buffer {
        api_version: ADAPTER_VERSION,
        worktree,
        path,
        leases: lease.lease_ids.len(),
        buffer_id: lease.remote_id,
        version: lease.version.clone(),
    })
}

async fn close_buffer(
    worktree: PathBuf,
    path: PathBuf,
    lease_id: String,
    buffers: &Buffers,
    zed: Option<&Zed>,
) -> Result<Response> {
    validate_lease_id(&lease_id)?;
    let (worktree, path) = buffer_key(worktree, path).await?;
    let key = (worktree.clone(), path.clone());
    let mut all = buffers.write().await;
    let Some(lease) = all.get_mut(&key) else {
        return Ok(Response::Buffer {
            api_version: ADAPTER_VERSION,
            worktree,
            path,
            leases: 0,
            buffer_id: 0,
            version: Vec::new(),
        });
    };
    lease.lease_ids.remove(&lease_id);
    let remote_id = lease.remote_id;
    let leases = lease.lease_ids.len();
    let response = Response::Buffer {
        api_version: ADAPTER_VERSION,
        worktree,
        path,
        leases,
        buffer_id: lease.remote_id,
        version: lease.version.clone(),
    };
    if leases == 0 {
        all.remove(&key);
        if let Some(zed) = zed {
            zed.close_buffer(remote_id)?;
        }
    }
    Ok(response)
}

fn validate_lease_id(lease_id: &str) -> Result<()> {
    if lease_id.is_empty() || lease_id.len() > 128 || lease_id.chars().any(char::is_whitespace) {
        bail!("buffer lease ID must be 1-128 non-whitespace characters");
    }
    Ok(())
}

async fn handle(
    stream: UnixStream,
    worktrees: Worktrees,
    buffers: Buffers,
    zed: Option<Zed>,
) -> Result<()> {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    while let Some(line) = lines.next_line().await? {
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) => respond(request, &worktrees, &buffers, zed.as_ref())
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
    let buffers: Buffers = Arc::default();
    let zed = Arc::new(ZedRuntime::start(&zed_server, &state_dir).await?);
    if let Some(data_home) = std::env::var_os("XDG_DATA_HOME") {
        zed.sync_extensions(Path::new(&data_home)).await?;
    }
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let worktrees = Arc::clone(&worktrees);
                let buffers = Arc::clone(&buffers);
                let zed = Arc::clone(&zed);
                tokio::spawn(async move {
                    let _ = handle(stream, worktrees, buffers, Some(zed)).await;
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
        let buffers: Buffers = Arc::default();

        let first = respond(
            Request::OpenWorktree {
                path: root.clone(),
                trusted: false,
            },
            &worktrees,
            &buffers,
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
            &buffers,
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
            &buffers,
            None,
        )
        .await
        .unwrap();
        assert_eq!(worktrees.read().await.get(&canonical).unwrap().leases, 1);
        let _ = respond(
            Request::CloseWorktree { path: root },
            &worktrees,
            &buffers,
            None,
        )
        .await
        .unwrap();
        assert!(worktrees.read().await.is_empty());
    }

    #[tokio::test]
    async fn ensure_worktree_is_idempotent_and_does_not_acquire_a_lease() {
        let root = std::env::current_dir().unwrap();
        let worktrees: Worktrees = Arc::default();
        let buffers: Buffers = Arc::default();
        for _ in 0..2 {
            let response = respond(
                Request::EnsureWorktree {
                    path: root.clone(),
                    trusted: true,
                },
                &worktrees,
                &buffers,
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
    async fn buffer_leases_are_bounded_to_an_open_worktree() {
        let root = std::env::current_dir().unwrap();
        let worktrees: Worktrees = Arc::default();
        let buffers: Buffers = Arc::default();
        respond(
            Request::EnsureWorktree {
                path: root.clone(),
                trusted: true,
            },
            &worktrees,
            &buffers,
            None,
        )
        .await
        .unwrap();
        for expected in [1, 2] {
            let response = respond(
                Request::OpenBuffer {
                    worktree: root.clone(),
                    path: PathBuf::from("Cargo.toml"),
                    lease_id: format!("lease-{expected}"),
                },
                &worktrees,
                &buffers,
                None,
            )
            .await
            .unwrap();
            assert!(matches!(
                response,
                Response::Buffer {
                    leases,
                    ..
                } if leases == expected
            ));
        }
        for expected in [1, 0] {
            let response = respond(
                Request::CloseBuffer {
                    worktree: root.clone(),
                    path: PathBuf::from("Cargo.toml"),
                    lease_id: format!("lease-{}", expected + 1),
                },
                &worktrees,
                &buffers,
                None,
            )
            .await
            .unwrap();
            assert!(matches!(
                response,
                Response::Buffer {
                    leases,
                    ..
                } if leases == expected
            ));
        }
        assert!(buffers.read().await.is_empty());
        let response = respond(
            Request::CloseBuffer {
                worktree: root.clone(),
                path: PathBuf::from("Cargo.toml"),
                lease_id: "lease-1".to_owned(),
            },
            &worktrees,
            &buffers,
            None,
        )
        .await
        .unwrap();
        assert!(matches!(response, Response::Buffer { leases: 0, .. }));
    }

    #[tokio::test]
    async fn buffer_lease_ids_make_retries_idempotent() {
        let root = std::env::current_dir().unwrap();
        let worktrees: Worktrees = Arc::default();
        let buffers: Buffers = Arc::default();
        respond(
            Request::EnsureWorktree {
                path: root.clone(),
                trusted: true,
            },
            &worktrees,
            &buffers,
            None,
        )
        .await
        .unwrap();
        for _ in 0..2 {
            let response = respond(
                Request::OpenBuffer {
                    worktree: root.clone(),
                    path: PathBuf::from("Cargo.toml"),
                    lease_id: "stable-client-lease".to_owned(),
                },
                &worktrees,
                &buffers,
                None,
            )
            .await
            .unwrap();
            assert!(matches!(response, Response::Buffer { leases: 1, .. }));
        }
    }

    #[test]
    fn buffer_requests_use_camel_case_lease_ids() {
        let request: Request = serde_json::from_str(
            r#"{"type":"openBuffer","worktree":"/tmp","path":"file.rs","leaseId":"tab-1"}"#,
        )
        .unwrap();
        assert!(matches!(
            request,
            Request::OpenBuffer { lease_id, .. } if lease_id == "tab-1"
        ));
    }

    #[test]
    fn buffer_versions_use_the_stable_camel_case_contract() {
        let value = serde_json::to_value(Response::BufferLanguage {
            api_version: ADAPTER_VERSION,
            worktree: PathBuf::from("/tmp"),
            path: PathBuf::from("file.rs"),
            version: vec![BufferVersionEntry {
                replica_id: 7,
                timestamp: 11,
            }],
            diagnostics: Vec::new(),
            inlay_hints: Vec::new(),
            semantic_tokens: Vec::new(),
        })
        .unwrap();
        assert_eq!(value["version"][0]["replicaId"], 7);
        assert_eq!(value["version"][0]["timestamp"], 11);
        assert!(value["version"][0].get("replica_id").is_none());
    }

    #[test]
    fn hover_points_convert_utf16_columns_to_utf8_offsets() {
        assert_eq!(utf16_point_to_offset("zero\nα😀x\n", 1, 0).unwrap(), 5);
        assert_eq!(utf16_point_to_offset("zero\nα😀x\n", 1, 1).unwrap(), 7);
        assert_eq!(utf16_point_to_offset("zero\nα😀x\n", 1, 3).unwrap(), 11);
        assert!(utf16_point_to_offset("zero\nα😀x\n", 1, 2).is_err());
        assert!(utf16_point_to_offset("zero\nα😀x\n", 9, 0).is_err());
        let point = offset_to_utf16_point("zero\nα😀x\n", 11).unwrap();
        assert_eq!((point.row, point.column), (1, 3));
        assert!(offset_to_utf16_point("😀", 1).is_none());
        let anchor = position_anchor(42, 11);
        assert_eq!(
            (anchor.replica_id, anchor.timestamp, anchor.offset),
            (0, 1, 11)
        );
    }

    #[test]
    fn extension_manifest_reads_only_top_level_identity() {
        let manifest = extension_manifest(
            r#"
id = "nix"
name = "Nix"
version = "0.1.4"

[lib]
version = "0.7.0"
"#,
        )
        .unwrap();
        assert_eq!(manifest.id, "nix");
        assert_eq!(manifest.version, "0.1.4");
    }

    #[tokio::test]
    async fn missing_worktree_fails_closed() {
        let worktrees: Worktrees = Arc::default();
        let buffers: Buffers = Arc::default();
        let result = respond(
            Request::OpenWorktree {
                path: PathBuf::from("/definitely/missing/cowboy-zed-worktree"),
                trusted: true,
            },
            &worktrees,
            &buffers,
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
            handle(stream, Arc::default(), Arc::default(), None)
                .await
                .unwrap();
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
            handle(stream, Arc::default(), Arc::default(), None)
                .await
                .unwrap();
        });

        probe(socket.clone(), Duration::from_secs(1)).await.unwrap();
        server.await.unwrap();
        tokio::fs::remove_file(socket).await.unwrap();
    }
}
