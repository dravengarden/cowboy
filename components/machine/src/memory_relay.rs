//! Memory for sessions on any Machine.
//!
//! Each Machine serves a loopback HTTP endpoint that speaks the memory
//! backend's own routes, so Provider adapters and native MCP clients need no
//! special transport. A request carries a per-session token and travels over
//! the authenticated Controller channel to the home Machine that owns the
//! backend, so memory uses the route that Machine already has and no other
//! Machine holds a backend credential.
//!
//! Memory is never required: every failure here is an ordinary 503, and a lost
//! uplink fails what is waiting at once. Nothing is queued or replayed; a
//! client that must keep evidence owns its own outbox.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Weak};
use std::time::Duration;

use anyhow::{Context as _, Result, bail, ensure};
use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, Method, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response as HttpResponse};
use parking_lot::Mutex;
use rand::RngCore as _;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use tokio::sync::{Semaphore, mpsc, oneshot};

use crate::machine_protocol::MachineEvent;
use crate::machine_protocol::memory_relay::{
    Identity, MAX_BODY_BYTES, MCP_HEADERS, Request, Response,
};

/// A `/v1` request is part of a turn; MCP tools may run a bounded program.
const TURN_DEADLINE: Duration = Duration::from_secs(8);
const TOOL_DEADLINE: Duration = Duration::from_secs(30);
const HOME_CONNECT: Duration = Duration::from_secs(2);
/// Excess requests are refused, never queued behind a slow link.
const MAX_IN_FLIGHT: usize = 32;
const TOKEN_PREFIX: &str = "cbm1";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Persisted {
    port: u16,
    secret: String,
}

pub struct MemoryRelay {
    endpoint: String,
    secret: String,
    outbox: PathBuf,
    uplink: Mutex<Option<(u64, mpsc::UnboundedSender<MachineEvent>)>>,
    pending: Mutex<HashMap<String, oneshot::Sender<Response>>>,
    next: AtomicU64,
    instance: String,
    capacity: Semaphore,
}

/// Keeps one Controller connection attached. Dropping it fails every request
/// still waiting for that connection's reply.
pub struct Uplink {
    relay: Weak<MemoryRelay>,
    id: u64,
}

impl Drop for Uplink {
    fn drop(&mut self) {
        let Some(relay) = self.relay.upgrade() else {
            return;
        };
        let mut uplink = relay.uplink.lock();
        if uplink.as_ref().is_some_and(|(id, _)| *id == self.id) {
            *uplink = None;
            drop(uplink);
            relay.pending.lock().clear();
        }
    }
}

fn random_hex<const N: usize>() -> String {
    let mut entropy = [0u8; N];
    rand::rngs::OsRng.fill_bytes(&mut entropy);
    entropy.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A retained worker keeps the endpoint and token it was launched with, so
/// both survive a Machine restart.
fn load_or_create(path: &Path) -> Result<Persisted> {
    if let Ok(bytes) = std::fs::read(path) {
        if let Ok(persisted) = serde_json::from_slice::<Persisted>(&bytes)
            && persisted.secret.len() == 64
        {
            return Ok(persisted);
        }
        tracing::warn!("replacing an unreadable memory relay record");
    }
    Ok(Persisted {
        port: 0,
        secret: random_hex::<32>(),
    })
}

fn store(path: &Path, persisted: &Persisted) -> Result<()> {
    let partial = path.with_extension(format!("{}.partial", std::process::id()));
    let _ = std::fs::remove_file(&partial);
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&partial)?;
    serde_json::to_writer(&file, persisted)?;
    file.sync_all()?;
    std::fs::rename(&partial, path)?;
    Ok(())
}

impl MemoryRelay {
    pub async fn start(state_dir: &Path) -> Result<Arc<Self>> {
        let directory = state_dir.join("extensions");
        std::fs::create_dir_all(&directory)?;
        let record = directory.join("memory-relay.json");
        let mut persisted = load_or_create(&record)?;
        let listener = match tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, persisted.port))
            .await
        {
            Ok(listener) => listener,
            // Retained sessions lose memory until they reload; new ones are fine.
            Err(error) if persisted.port != 0 => {
                tracing::warn!(%error, port = persisted.port, "memory relay port is unavailable");
                tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?
            }
            Err(error) => return Err(error.into()),
        };
        persisted.port = listener.local_addr()?.port();
        store(&record, &persisted)?;
        let relay = Arc::new(Self {
            endpoint: format!("http://127.0.0.1:{}", persisted.port),
            secret: persisted.secret,
            outbox: directory.join("memory-outbox"),
            uplink: Mutex::new(None),
            pending: Mutex::new(HashMap::new()),
            next: AtomicU64::new(1),
            instance: random_hex::<4>(),
            capacity: Semaphore::new(MAX_IN_FLIGHT),
        });
        let router = Router::new()
            .fallback(serve)
            .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
            .with_state(Arc::clone(&relay));
        tokio::spawn(async move {
            if let Err(error) = axum::serve(listener, router).await {
                tracing::warn!(%error, "memory relay stopped");
            }
        });
        Ok(relay)
    }

    fn proof(&self, session: &str) -> String {
        format!(
            "{:x}",
            Sha256::digest(format!("{TOKEN_PREFIX}:{}:{session}", self.secret))
        )
    }

    fn token(&self, session: &str) -> String {
        format!("{TOKEN_PREFIX}.{session}.{}", self.proof(session))
    }

    /// The session named by a bearer token, when the token is this Machine's.
    fn authenticate(&self, token: &str) -> Option<String> {
        let (session, proof) = token.strip_prefix("cbm1.")?.rsplit_once('.')?;
        (Sha256::digest(proof) == Sha256::digest(self.proof(session))).then(|| session.to_owned())
    }

    /// What a session's descriptor needs to use this relay.
    #[must_use]
    pub fn session_access(&self, session: &str) -> Value {
        json!({
            "endpoint": self.endpoint,
            "token": self.token(session),
            "state_dir": self.outbox.join(session),
        })
    }

    pub fn attach(self: &Arc<Self>, events: mpsc::UnboundedSender<MachineEvent>) -> Uplink {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        *self.uplink.lock() = Some((id, events));
        self.pending.lock().clear();
        Uplink {
            relay: Arc::downgrade(self),
            id,
        }
    }

    /// Complete one relayed request. Unknown or late replies are ignored.
    pub fn reply(&self, request_id: &str, response: Response) {
        if let Some(waiter) = self.pending.lock().remove(request_id) {
            let _ = waiter.send(response);
        }
    }

    async fn forward(&self, session_id: String, request: Request) -> Response {
        let Ok(_permit) = self.capacity.try_acquire() else {
            return Response::unavailable();
        };
        let deadline = if request.path == "/mcp" {
            TOOL_DEADLINE
        } else {
            TURN_DEADLINE
        };
        let request_id = format!(
            "memory-{}-{}",
            self.instance,
            self.next.fetch_add(1, Ordering::Relaxed)
        );
        let (sender, receiver) = oneshot::channel();
        {
            let uplink = self.uplink.lock();
            let Some((_, events)) = uplink.as_ref() else {
                return Response::unavailable();
            };
            self.pending.lock().insert(request_id.clone(), sender);
            if events
                .send(MachineEvent::MemoryRelayRequest {
                    request_id: request_id.clone(),
                    session_id,
                    request,
                })
                .is_err()
            {
                self.pending.lock().remove(&request_id);
                return Response::unavailable();
            }
        }
        let result = tokio::time::timeout(deadline, receiver).await;
        self.pending.lock().remove(&request_id);
        match result {
            Ok(Ok(response)) => response,
            _ => Response::unavailable(),
        }
    }
}

async fn serve(
    State(relay): State<Arc<MemoryRelay>>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> HttpResponse {
    let session = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .and_then(|token| relay.authenticate(token));
    let Some(session) = session else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    // MCP clients probe GET for a server stream; the backend has none.
    if method != Method::POST {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    let Ok(text) = String::from_utf8(body.to_vec()) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let request = Request {
        path: uri.path().to_owned(),
        body: text,
        headers: MCP_HEADERS
            .iter()
            .filter_map(|name| {
                let value = headers.get(*name)?.to_str().ok()?;
                Some(((*name).to_owned(), value.to_owned()))
            })
            .collect(),
    };
    if request.validate().is_err() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let response = relay.forward(session, request).await;
    (
        StatusCode::from_u16(response.status).unwrap_or(StatusCode::SERVICE_UNAVAILABLE),
        [(header::CONTENT_TYPE, "application/json")],
        response.body,
    )
        .into_response()
}

// ---- Home Machine ----------------------------------------------------------

#[derive(Deserialize)]
struct HomeBinding {
    endpoint: String,
    token: String,
    #[serde(default)]
    projects: Vec<ProjectMapping>,
}

#[derive(Deserialize)]
struct ProjectMapping {
    workspace: Option<String>,
    machine: Option<String>,
    project: String,
}

/// The backend identity of a Cowboy Provider. Each has its own credential.
fn client_name(provider: &str) -> &str {
    match provider {
        "claude-code" => "claude",
        other => other,
    }
}

fn config_home() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
}

fn read_private(path: &Path) -> Result<Option<Vec<u8>>> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        metadata.is_file()
            && metadata.mode() & 0o077 == 0
            && metadata.uid() == rustix::process::geteuid().as_raw()
            && metadata.len() <= 65_536,
        "memory binding must be a private regular file"
    );
    Ok(Some(std::fs::read(path)?))
}

fn home_binding(config: &Path, provider: &str) -> Result<HomeBinding> {
    let name = client_name(provider);
    ensure!(
        !name.is_empty()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-'),
        "invalid memory client"
    );
    for path in [
        config.join(format!("cowboy/memory/matrix/{name}.json")),
        config.join(format!("matrix/{name}.json")),
    ] {
        if let Some(bytes) = read_private(&path)? {
            return serde_json::from_slice(&bytes).context("parsing memory binding");
        }
    }
    bail!("no memory binding for {name}");
}

/// One logical project for a workspace on any Machine: an exact mapping wins,
/// then the workspace's mapping elsewhere, then the workspace id. The backend
/// still checks the project against this client's grant.
fn project_for(binding: &HomeBinding, identity: &Identity) -> Result<String> {
    let workspace = identity
        .workspace
        .as_deref()
        .context("session has no workspace")?;
    let mapped: Vec<_> = binding
        .projects
        .iter()
        .filter(|entry| entry.workspace.as_deref() == Some(workspace))
        .collect();
    if let Some(entry) = mapped
        .iter()
        .find(|entry| entry.machine.as_deref() == Some(identity.machine.as_str()))
    {
        return Ok(entry.project.clone());
    }
    Ok(match mapped.as_slice() {
        [first, rest @ ..] if rest.iter().all(|entry| entry.project == first.project) => {
            first.project.clone()
        }
        _ => workspace.to_owned(),
    })
}

fn backend_binding(identity: &Identity, project: String) -> Value {
    json!({
        "project": project,
        "machine": identity.machine,
        "session": identity.session,
    })
}

/// `/v1` bodies carry the binding in JSON; the client's claim is replaced.
fn bind_body(request: &Request, binding: &Value) -> Result<String> {
    if request.path == "/mcp" {
        return Ok(request.body.clone());
    }
    let mut body: Value = serde_json::from_str(&request.body).context("parsing memory request")?;
    body.as_object_mut()
        .context("memory request is not an object")?
        .insert("binding".to_owned(), binding.clone());
    Ok(serde_json::to_string(&body)?)
}

async fn home_request(identity: &Identity, request: &Request) -> Result<Response> {
    request.validate()?;
    let config = config_home().context("no configuration directory")?;
    let binding = home_binding(&config, &identity.provider)?;
    let selected = backend_binding(identity, project_for(&binding, identity)?);
    let body = bind_body(request, &selected)?;
    let url = format!("{}{}", binding.endpoint.trim_end_matches('/'), request.path);
    let deadline = if request.path == "/mcp" {
        TOOL_DEADLINE
    } else {
        TURN_DEADLINE
    };
    let client = HOME_CLIENT
        .as_ref()
        .map_err(|error| anyhow::anyhow!("{error}"))?;
    let mut outgoing = client
        .post(url)
        .timeout(deadline)
        .bearer_auth(&binding.token)
        .header(header::CONTENT_TYPE, "application/json")
        .header("X-Matrix-Binding", selected.to_string());
    for (name, value) in &request.headers {
        outgoing = outgoing.header(name, value);
    }
    let mut response = outgoing.body(body).send().await?;
    let status = response.status().as_u16();
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes.len() + chunk.len() <= MAX_BODY_BYTES,
            "memory response exceeds its limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(Response {
        status,
        body: String::from_utf8(bytes).context("memory response is not text")?,
    })
}

// The backend listens on this host: never send it through a proxy.
static HOME_CLIENT: LazyLock<Result<reqwest::Client, String>> = LazyLock::new(|| {
    // Building a client without a provider panics; this is idempotent.
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(HOME_CONNECT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| error.to_string())
});
static HOME_CAPACITY: Semaphore = Semaphore::const_new(MAX_IN_FLIGHT);

/// Serve one relayed request on the home Machine and answer on `events`.
pub fn serve_home(
    request_id: String,
    identity: Identity,
    request: Request,
    events: mpsc::UnboundedSender<MachineEvent>,
) {
    tokio::spawn(async move {
        let response = match HOME_CAPACITY.try_acquire() {
            Err(_) => Response::unavailable(),
            Ok(_permit) => home_request(&identity, &request)
                .await
                .unwrap_or_else(|error| {
                    tracing::warn!(error = %format!("{error:#}"), "memory relay request failed");
                    Response::unavailable()
                }),
        };
        let _ = events.send(MachineEvent::MemoryRelayResponse {
            request_id,
            response,
        });
    });
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use super::*;

    fn identity(workspace: Option<&str>, machine: &str) -> Identity {
        Identity {
            provider: "grok".to_owned(),
            session: "sess-1".to_owned(),
            machine: machine.to_owned(),
            workspace: workspace.map(str::to_owned),
        }
    }

    fn binding(projects: Value) -> HomeBinding {
        serde_json::from_value(
            json!({"endpoint":"http://127.0.0.1:1","token":"t","projects":projects}),
        )
        .unwrap()
    }

    #[test]
    fn a_workspace_is_one_project_on_every_machine() {
        let mapped = binding(json!([
            {"workspace":"cow","machine":"hawk","project":"cowboy"},
            {"workspace":"cow","machine":"falcon","project":"cowboy"},
            {"workspace":"split","machine":"hawk","project":"a"},
            {"workspace":"split","machine":"falcon","project":"b"},
        ]));
        let project = |workspace, machine| project_for(&mapped, &identity(workspace, machine));
        assert_eq!(project(Some("cow"), "hawk").unwrap(), "cowboy");
        assert_eq!(project(Some("cow"), "ovh").unwrap(), "cowboy");
        assert_eq!(project(Some("split"), "falcon").unwrap(), "b");
        // An ambiguous or unknown workspace is left for the grant check.
        assert_eq!(project(Some("split"), "ovh").unwrap(), "split");
        assert_eq!(project(Some("new"), "ovh").unwrap(), "new");
        assert!(project(None, "ovh").is_err());
    }

    #[test]
    fn a_client_cannot_claim_another_binding() {
        let selected = backend_binding(&identity(Some("w"), "hawk"), "p".to_owned());
        let claimed = Request {
            path: "/v1/context".to_owned(),
            body: r#"{"binding":{"project":"secret","machine":"x","session":"y"},"payload":1}"#
                .to_owned(),
            headers: Vec::new(),
        };
        let bound: Value = serde_json::from_str(&bind_body(&claimed, &selected).unwrap()).unwrap();
        assert_eq!(bound["binding"]["project"], "p");
        assert_eq!(bound["binding"]["session"], "sess-1");
        assert_eq!(bound["payload"], 1);
    }

    #[test]
    fn claude_code_uses_its_backend_client_and_bindings_stay_private() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("cowboy/memory/matrix");
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("claude.json");
        std::fs::write(&path, r#"{"endpoint":"http://127.0.0.1:1","token":"t"}"#).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(home_binding(root.path(), "claude-code").is_ok());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(home_binding(root.path(), "claude-code").is_err());
        assert!(home_binding(root.path(), "../claude").is_err());
        assert!(home_binding(root.path(), "grok").is_err());
    }

    #[tokio::test]
    async fn tokens_name_their_session_and_survive_a_restart() {
        let root = tempfile::tempdir().unwrap();
        let first = MemoryRelay::start(root.path()).await.unwrap();
        let access = first.session_access("sess-a.b");
        let token = access["token"].as_str().unwrap();
        assert_eq!(first.authenticate(token).unwrap(), "sess-a.b");
        assert!(first.authenticate("cbm1.sess-a.b.00").is_none());
        assert!(
            first
                .authenticate(&token.replace("sess-a", "sess-c"))
                .is_none()
        );
        let second = MemoryRelay::start(root.path())
            .await
            .unwrap_or_else(|_| first.clone());
        assert_eq!(second.authenticate(token).unwrap(), "sess-a.b");
    }

    #[tokio::test]
    async fn a_lost_uplink_answers_at_once() {
        let root = tempfile::tempdir().unwrap();
        let relay = MemoryRelay::start(root.path()).await.unwrap();
        let request = Request {
            path: "/v1/context".to_owned(),
            body: "{}".to_owned(),
            headers: Vec::new(),
        };
        // Never attached.
        let response = relay.forward("s".to_owned(), request.clone()).await;
        assert_eq!(response.status, 503);
        // Attached, then dropped while waiting.
        let (sender, mut events) = mpsc::unbounded_channel();
        let uplink = relay.attach(sender);
        let waiting = tokio::spawn({
            let relay = Arc::clone(&relay);
            async move { relay.forward("s".to_owned(), request).await }
        });
        assert!(events.recv().await.is_some());
        drop(uplink);
        assert_eq!(waiting.await.unwrap().status, 503);
    }

    #[tokio::test]
    async fn a_reply_reaches_its_waiter_over_http() {
        let root = tempfile::tempdir().unwrap();
        let relay = MemoryRelay::start(root.path()).await.unwrap();
        let (sender, mut events) = mpsc::unbounded_channel();
        let _uplink = relay.attach(sender);
        let answering = Arc::clone(&relay);
        tokio::spawn(async move {
            let Some(MachineEvent::MemoryRelayRequest {
                request_id,
                session_id,
                request,
            }) = events.recv().await
            else {
                panic!("expected a relay request");
            };
            assert_eq!((session_id.as_str(), request.path.as_str()), ("s1", "/mcp"));
            assert_eq!(
                request.headers,
                [("mcp-method".to_owned(), "tools/list".to_owned())]
            );
            answering.reply(
                &request_id,
                Response {
                    status: 200,
                    body: r#"{"ok":true}"#.to_owned(),
                },
            );
        });
        let access = relay.session_access("s1");
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("{}/mcp", access["endpoint"].as_str().unwrap());
        let refused = client.post(&url).body("{}").send().await.unwrap();
        assert_eq!(refused.status(), 401);
        let answered = client
            .post(&url)
            .bearer_auth(access["token"].as_str().unwrap())
            .header("Mcp-Method", "tools/list")
            .header("X-Other", "dropped")
            .body("{}")
            .send()
            .await
            .unwrap();
        assert_eq!(answered.status(), 200);
        assert_eq!(answered.text().await.unwrap(), r#"{"ok":true}"#);
    }
}
