//! Product commands over this computer's `cowboy login`: everything the Web UI
//! does through the same authenticated HTTP routes and WebSocket commands.
//! `session` covers the common lifecycle; `api` and `send` reach any other
//! route or command without waiting for a dedicated subcommand.

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use clap::{Args, Subcommand, ValueEnum};
use futures::{SinkExt as _, StreamExt as _};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;
use url::Url;

use crate::client_auth_client::{ClientAuthentication, logged_in_origins, normalized_base_url};
use crate::core::{Inbound, Outbound, SessionMeta};

const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(15);
/// How long `send` listens for a refusal of the command it sent.
const REFUSAL_WINDOW: Duration = Duration::from_secs(2);

#[derive(Args, Clone)]
pub struct ConnectArgs {
    /// Cowboy origin. Defaults to the only origin this computer is logged in to.
    #[arg(long, env = "COWBOY_ORIGIN", global = true)]
    origin: Option<String>,
    /// Private device-credential directory used by `cowboy login`.
    #[arg(long, env = "COWBOY_AUTH_STATE_DIR", global = true)]
    auth_state_dir: Option<PathBuf>,
}

#[derive(Args)]
pub struct SessionArgs {
    #[command(flatten)]
    connect: ConnectArgs,
    #[command(subcommand)]
    command: SessionCommand,
}

#[derive(Args)]
pub struct DirArgs {
    #[command(flatten)]
    connect: ConnectArgs,
    #[command(subcommand)]
    command: DirCommand,
}

#[derive(Subcommand)]
enum DirCommand {
    /// Create a directory, inheriting defaults from --parent without copying them.
    #[command(alias = "create")]
    New {
        name: String,
        #[arg(long)]
        parent: Option<String>,
    },
    /// List the visible directory tree and its explicit overrides.
    List,
    /// Show effective defaults and the source directory for each field.
    Defaults { directory: String },
    /// Set one field. Fields: project, machine, runtime_machine, provider, preset.
    /// Presets use provider/preset. No value requires --inherit or --clear.
    Set {
        directory: String,
        field: String,
        value: Option<String>,
        #[arg(long, conflicts_with_all = ["clear", "value"])]
        inherit: bool,
        #[arg(long, conflicts_with = "value")]
        clear: bool,
    },
}

pub async fn run_dir(args: DirArgs) -> Result<()> {
    let client = Client::new(&args.connect)?;
    let response = match args.command {
        DirCommand::New { name, parent } => {
            client
                .expect(
                    Method::POST,
                    "/api/folders",
                    Some(&json!({"name":name,"parent":parent})),
                )
                .await?
        }
        DirCommand::List => client.expect(Method::GET, "/api/folders", None).await?,
        DirCommand::Defaults { directory } => {
            client
                .expect(Method::GET, &directory_defaults_path(&directory)?, None)
                .await?
        }
        DirCommand::Set {
            directory,
            field,
            value,
            inherit,
            clear,
        } => {
            let _: crate::session_folders::defaults::DefaultKey =
                serde_json::from_value(json!(field)).context("unknown directory default field")?;
            ensure!(
                value.is_some() || inherit || clear,
                "provide a value, --inherit, or --clear"
            );
            let body = if inherit {
                json!({"values":{}, "inherit":[field]})
            } else {
                json!({"values":{field:value}})
            };
            client
                .expect(
                    Method::PATCH,
                    &directory_defaults_path(&directory)?,
                    Some(&body),
                )
                .await?
        }
    };
    println!("{response}");
    Ok(())
}

fn directory_defaults_path(directory: &str) -> Result<String> {
    ensure!(
        directory != "." && directory != "..",
        "invalid directory id"
    );
    ensure!(
        !directory.is_empty()
            && directory
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':')),
        "invalid directory id"
    );
    Ok(format!("/api/folders/{directory}/defaults"))
}

#[derive(Subcommand)]
enum SessionCommand {
    /// List sessions with status, Provider version and any waiting update.
    List {
        #[arg(long)]
        json: bool,
        /// Only sessions with a newer installed Provider release.
        #[arg(long)]
        outdated: bool,
    },
    /// Print one session's full record as JSON.
    Info { session: String },
    /// Move sessions to their Machine's newest installed Provider release.
    ///
    /// Without --force a busy session is refused; with --force its running
    /// turn is cancelled first, as the Web UI's stop does.
    Update {
        sessions: Vec<String>,
        /// Every visible session with an update waiting.
        #[arg(long, conflicts_with = "sessions")]
        all: bool,
        /// Interrupt a running turn instead of refusing a busy session.
        #[arg(long)]
        force: bool,
        /// Update once the current work finishes instead of now.
        #[arg(long, conflicts_with = "force")]
        when_idle: bool,
        /// Keep these sessions on every future release once idle.
        #[arg(long, value_enum)]
        auto: Option<Toggle>,
    },
    /// Rebuild a session's agent runtime on its current release.
    Reload {
        session: String,
        #[arg(long)]
        force: bool,
    },
    /// Free an idle session's agent process and Machine slot.
    Hibernate { session: String },
    /// Send a prompt (queued if the session is busy).
    Prompt { session: String, text: String },
    /// Cancel the running turn.
    Cancel { session: String },
    /// Clear the conversation context (destructive, like the Web UI).
    Clear {
        session: String,
        #[arg(long)]
        yes: bool,
    },
    /// Change a session's title.
    Rename { session: String, title: String },
    /// Delete a session and its transcript.
    Delete {
        session: String,
        #[arg(long)]
        yes: bool,
    },
    /// Start a session with a Provider, optionally in a directory.
    New {
        provider: Option<String>,
        #[arg(long)]
        cwd: Option<String>,
        #[arg(long)]
        machine: Option<String>,
        #[arg(long)]
        runtime_machine: Option<String>,
        #[arg(long)]
        dir: Option<String>,
        /// Provider-qualified preset, or an empty string for agent defaults.
        #[arg(long)]
        preset: Option<String>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Toggle {
    On,
    Off,
}

#[derive(Args)]
pub struct ApiArgs {
    #[command(flatten)]
    connect: ConnectArgs,
    /// HTTP method, for example GET, POST, PUT or DELETE.
    method: String,
    /// Path below the origin, for example /api/sessions/ID/info.
    path: String,
    /// JSON request body; `-` reads it from standard input.
    #[arg(long)]
    data: Option<String>,
}

#[derive(Args)]
pub struct SendArgs {
    #[command(flatten)]
    connect: ConnectArgs,
    /// One WebSocket command as JSON, for example
    /// '{"type":"rename_session","session_id":"ID","title":"T"}'.
    command: String,
}

struct Client {
    base_url: Url,
    auth: ClientAuthentication,
    http: reqwest::Client,
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

impl Client {
    #[cfg(test)]
    fn fixture(base_url: Url) -> Self {
        Self {
            auth: ClientAuthentication::fixture(base_url.clone()),
            base_url,
            http: reqwest::Client::new(),
        }
    }

    fn new(args: &ConnectArgs) -> Result<Self> {
        let origin = match &args.origin {
            Some(origin) => origin.clone(),
            None => {
                let origins = logged_in_origins(args.auth_state_dir.clone())?;
                match origins.as_slice() {
                    [origin] => origin.clone(),
                    [] => bail!("not logged in; run `cowboy login https://<origin>` first"),
                    _ => bail!("logged in to several origins; pass --origin or COWBOY_ORIGIN"),
                }
            }
        };
        let base_url = normalized_base_url(&origin)?;
        let auth =
            ClientAuthentication::new(base_url.clone(), None, args.auth_state_dir.clone(), None)?;
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            base_url,
            auth,
            http,
        })
    }

    async fn request(
        &self,
        method: Method,
        path_and_query: &str,
        body: Option<&Value>,
    ) -> Result<(StatusCode, String)> {
        ensure!(
            path_and_query.starts_with('/'),
            "path must start with /, for example /api/sessions/ID/info"
        );
        let url = self.base_url.join(path_and_query.trim_start_matches('/'))?;
        let target = request_target(&url);
        let mut rejected = None;
        for attempt in 0..2 {
            let authorization = self
                .auth
                .authorize(&method, &target, rejected.as_deref())
                .await?;
            let mut request =
                authorization.apply_reqwest(self.http.request(method.clone(), url.clone()))?;
            if let Some(body) = body {
                request = request.json(body);
            }
            let response = request
                .send()
                .await
                .with_context(|| format!("{method} {path_and_query}"))?;
            let status = response.status();
            if status == StatusCode::UNAUTHORIZED && attempt == 0 {
                rejected = authorization.bearer;
                continue;
            }
            return Ok((status, response.text().await.unwrap_or_default()));
        }
        unreachable!("the second attempt always returns")
    }

    async fn expect(
        &self,
        method: Method,
        path_and_query: &str,
        body: Option<&Value>,
    ) -> Result<String> {
        let (status, text) = self.request(method.clone(), path_and_query, body).await?;
        ensure!(
            status.is_success(),
            "{method} {path_and_query}: {status}: {}",
            text.trim()
        );
        Ok(text)
    }

    /// An authenticated WebSocket after bootstrap, with the visible sessions.
    async fn connect(&self) -> Result<(Socket, Vec<SessionMeta>)> {
        let mut url = self.base_url.join("ws")?;
        let scheme = if self.base_url.scheme() == "https" {
            "wss"
        } else {
            "ws"
        };
        url.set_scheme(scheme)
            .map_err(|()| anyhow!("cannot form the WebSocket URL"))?;
        let target = request_target(&url);
        let mut rejected = None;
        let mut socket = None;
        for attempt in 0..2 {
            let authorization = self
                .auth
                .authorize(&Method::GET, &target, rejected.as_deref())
                .await?;
            let mut request = url.as_str().into_client_request()?;
            if let Some(bearer) = &authorization.bearer {
                request
                    .headers_mut()
                    .insert("authorization", format!("Bearer {bearer}").parse()?);
            }
            for (name, value) in &authorization.proof_headers {
                request.headers_mut().insert(
                    tokio_tungstenite::tungstenite::http::HeaderName::from_bytes(name.as_bytes())?,
                    value.parse()?,
                );
            }
            match tokio_tungstenite::connect_async(request).await {
                Ok((connected, _)) => {
                    socket = Some(connected);
                    break;
                }
                Err(tokio_tungstenite::tungstenite::Error::Http(response))
                    if response.status() == StatusCode::UNAUTHORIZED && attempt == 0 =>
                {
                    rejected = authorization.bearer;
                }
                Err(error) => return Err(anyhow!(error).context("connecting to Cowboy")),
            }
        }
        let mut socket = socket.context("Cowboy rejected this computer's login")?;
        let mut sessions = Vec::new();
        tokio::time::timeout(BOOTSTRAP_TIMEOUT, async {
            loop {
                let message = socket
                    .next()
                    .await
                    .ok_or_else(|| anyhow!("Cowboy closed the connection during bootstrap"))??;
                let Message::Text(text) = message else {
                    continue;
                };
                match serde_json::from_str::<Outbound>(&text) {
                    Ok(Outbound::Sessions { sessions: listed }) => sessions = listed,
                    Ok(Outbound::BootstrapComplete) => return Ok::<_, anyhow::Error>(()),
                    _ => {}
                }
            }
        })
        .await
        .context("timed out waiting for Cowboy to bootstrap")??;
        Ok((socket, sessions))
    }

    async fn sessions(&self) -> Result<Vec<SessionMeta>> {
        let (mut socket, sessions) = self.connect().await?;
        let _ = socket.close(None).await;
        Ok(sessions)
    }

    /// Send one WebSocket command and report a refusal addressed to it.
    async fn send(&self, command: &Inbound) -> Result<()> {
        let (mut socket, _) = self.connect().await?;
        socket
            .send(Message::Text(serde_json::to_string(command)?.into()))
            .await?;
        let session = serde_json::to_value(command)?
            .get("session_id")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let refusal = tokio::time::timeout(REFUSAL_WINDOW, async {
            while let Some(Ok(message)) = socket.next().await {
                let Message::Text(text) = message else {
                    continue;
                };
                if let Ok(outbound) = serde_json::from_str::<Value>(&text)
                    && matches!(
                        outbound.get("type").and_then(Value::as_str),
                        Some("error" | "rejected" | "not_found")
                    )
                    && (session.is_none()
                        || outbound.get("session_id").and_then(Value::as_str) == session.as_deref())
                {
                    return Some(outbound);
                }
            }
            None
        })
        .await
        .ok()
        .flatten();
        let _ = socket.close(None).await;
        if let Some(refusal) = refusal {
            bail!("Cowboy refused the command: {refusal}");
        }
        Ok(())
    }
}

fn request_target(url: &Url) -> String {
    url.query().map_or_else(
        || url.path().to_owned(),
        |query| format!("{}?{query}", url.path()),
    )
}

fn status_name(meta: &SessionMeta) -> String {
    serde_json::to_value(meta.status)
        .ok()
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .unwrap_or_default()
}

fn find<'a>(sessions: &'a [SessionMeta], id: &str) -> Result<&'a SessionMeta> {
    sessions
        .iter()
        .find(|meta| meta.id == id)
        .with_context(|| format!("no visible session {id}"))
}

pub async fn run_session(args: SessionArgs) -> Result<()> {
    let client = Client::new(&args.connect)?;
    match args.command {
        SessionCommand::List { json, outdated } => {
            let mut sessions = client.sessions().await?;
            if outdated {
                sessions.retain(|meta| meta.provider_update_available.is_some());
            }
            if json {
                println!("{}", serde_json::to_string_pretty(&sessions)?);
                return Ok(());
            }
            for meta in &sessions {
                let update = meta
                    .provider_update_available
                    .as_ref()
                    .map(|offer| format!(" -> {}", offer.version))
                    .unwrap_or_default();
                println!(
                    "{}\t{}\t{}\t{} {}{}\t{}",
                    meta.id,
                    status_name(meta),
                    meta.machine_id,
                    meta.provider,
                    meta.provider_version,
                    update,
                    meta.title
                );
            }
        }
        SessionCommand::Info { session } => {
            let text = client
                .expect(Method::GET, &format!("/api/sessions/{session}/info"), None)
                .await?;
            println!("{text}");
        }
        SessionCommand::Update {
            sessions,
            all,
            force,
            when_idle,
            auto,
        } => {
            let ids = if all {
                client
                    .sessions()
                    .await?
                    .into_iter()
                    .filter(|meta| meta.provider_update_available.is_some())
                    .map(|meta| meta.id)
                    .collect()
            } else {
                ensure!(!sessions.is_empty(), "name sessions or pass --all");
                sessions
            };
            let mut failed = 0_usize;
            for id in &ids {
                match update(&client, id, force, when_idle, auto).await {
                    Ok(message) => println!("{id}\t{message}"),
                    Err(error) => {
                        failed += 1;
                        eprintln!("{id}\t{error:#}");
                    }
                }
            }
            if ids.is_empty() {
                println!("no session has an update waiting");
            }
            ensure!(
                failed == 0,
                "{failed} of {} sessions did not update",
                ids.len()
            );
        }
        SessionCommand::Reload { session, force } => {
            let text = client
                .expect(
                    Method::POST,
                    &format!("/api/sessions/{session}/reload?confirm_active_turn={force}"),
                    None,
                )
                .await?;
            println!("{session}\t{}", text.trim());
        }
        SessionCommand::Hibernate { session } => {
            let text = client
                .expect(
                    Method::POST,
                    &format!("/api/sessions/{session}/hibernate"),
                    None,
                )
                .await?;
            println!("{session}\t{}", text.trim());
        }
        SessionCommand::Prompt { session, text } => {
            client
                .expect(
                    Method::POST,
                    &format!("/api/sessions/{session}/prompt"),
                    Some(&json!({ "text": text })),
                )
                .await?;
            println!("{session}\tsent");
        }
        SessionCommand::Cancel { session } => {
            client
                .send(&Inbound::Cancel {
                    session_id: session.clone(),
                })
                .await?;
            println!("{session}\tcancel requested");
        }
        SessionCommand::Clear { session, yes } => {
            ensure!(
                yes,
                "clearing discards the agent context and transcript; pass --yes"
            );
            client
                .send(&Inbound::ResetSession {
                    session_id: session.clone(),
                })
                .await?;
            println!("{session}\tclear requested");
        }
        SessionCommand::Rename { session, title } => {
            ensure!(!title.trim().is_empty(), "title cannot be empty");
            client
                .send(&Inbound::RenameSession {
                    session_id: session.clone(),
                    title,
                })
                .await?;
            println!("{session}\trenamed");
        }
        SessionCommand::Delete { session, yes } => {
            ensure!(
                yes,
                "deleting removes the session and its transcript; pass --yes"
            );
            let sessions = client.sessions().await?;
            find(&sessions, &session)?;
            client
                .send(&Inbound::DeleteSession {
                    session_id: session.clone(),
                })
                .await?;
            println!("{session}\tdelete requested");
        }
        SessionCommand::New {
            provider,
            cwd,
            machine,
            runtime_machine,
            dir,
            preset,
        } => {
            let mut body = json!({"provider":provider.unwrap_or_default(),"cwd":cwd,"folder_id":dir,"preset":preset});
            if let Some(machine) = machine {
                body["machine_id"] = json!(machine);
            }
            if let Some(runtime) = runtime_machine {
                body["runtime_machine_id"] = json!(runtime);
            }
            println!(
                "{}",
                client
                    .expect(Method::POST, "/api/sessions", Some(&body))
                    .await?
            );
        }
    }
    Ok(())
}

async fn update(
    client: &Client,
    id: &str,
    force: bool,
    when_idle: bool,
    auto: Option<Toggle>,
) -> Result<String> {
    let path = format!("/api/sessions/{id}/reload");
    if let Some(auto) = auto {
        client
            .expect(
                Method::PUT,
                &path,
                Some(&json!({ "enabled": matches!(auto, Toggle::On) })),
            )
            .await?;
    }
    if when_idle {
        client
            .expect(Method::PUT, &path, Some(&json!({ "when_idle": true })))
            .await?;
        return Ok("updates once the current work finishes".to_owned());
    }
    let plan: Value = serde_json::from_str(&client.expect(Method::GET, &path, None).await?)
        .context("decoding the reload plan")?;
    if plan.get("upgrade_available").and_then(Value::as_bool) != Some(true) {
        let current = plan
            .get("current_version")
            .and_then(Value::as_str)
            .unwrap_or("?");
        return match plan.get("blocked_reason").and_then(Value::as_str) {
            Some(reason) if !(force && reason.starts_with("wait for the current turn")) => {
                Err(anyhow!("stays on {current}: {reason}"))
            }
            Some(_) => {
                // Busy: no target resolves until the turn ends. Cancel it
                // as the Web UI's stop does, then update once.
                client
                    .send(&Inbound::Cancel {
                        session_id: id.to_owned(),
                    })
                    .await?;
                wait_until_idle(client, id).await?;
                Box::pin(update(client, id, false, false, None)).await
            }
            None if auto.is_some() => Ok(format!("already current ({current}); policy saved")),
            None => Ok(format!("already current ({current})")),
        };
    }
    let digest = plan
        .get("target_digest")
        .and_then(Value::as_str)
        .context("reload plan has no target digest")?;
    let target = plan
        .get("target_version")
        .and_then(Value::as_str)
        .unwrap_or("?");
    client
        .expect(
            Method::POST,
            &format!("{path}?upgrade_provider=true&expected_generation_digest={digest}"),
            None,
        )
        .await?;
    Ok(format!("updating to {target}"))
}

async fn wait_until_idle(client: &Client, id: &str) -> Result<()> {
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let text = client
            .expect(Method::GET, &format!("/api/sessions/{id}/info"), None)
            .await?;
        let info: Value = serde_json::from_str(&text)?;
        let status = info
            .pointer("/meta/status")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if matches!(status, "running" | "exited") {
            return Ok(());
        }
    }
    bail!("session did not become idle after its turn was interrupted")
}

pub async fn run_api(args: ApiArgs) -> Result<()> {
    let client = Client::new(&args.connect)?;
    let method = Method::from_bytes(args.method.to_ascii_uppercase().as_bytes())
        .context("invalid HTTP method")?;
    let body = match args.data.as_deref() {
        None => None,
        Some("-") => {
            let mut text = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)?;
            Some(serde_json::from_str(&text).context("request body is not JSON")?)
        }
        Some(text) => Some(serde_json::from_str(text).context("request body is not JSON")?),
    };
    let (status, text) = client.request(method, &args.path, body.as_ref()).await?;
    println!("{text}");
    ensure!(status.is_success(), "{status}");
    Ok(())
}

pub async fn run_send(args: SendArgs) -> Result<()> {
    let client = Client::new(&args.connect)?;
    let command: Inbound =
        serde_json::from_str(&args.command).context("not a Cowboy WebSocket command")?;
    client.send(&command).await?;
    println!("sent");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::{Path, Query};
    use axum::routing::get;
    use axum::{Json, Router};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn update_confirms_the_planned_release_and_reports_blocked_sessions() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let posted = Arc::new(Mutex::new(Vec::<String>::new()));
        let record = Arc::clone(&posted);
        let app = Router::new().route(
            "/api/sessions/{id}/reload",
            get(|Path(id): Path<String>| async move {
                Json(match id.as_str() {
                    "old" => json!({"current_version":"1.0.0","upgrade_available":true,
                        "target_version":"1.1.0","target_digest":"sha256:new"}),
                    "current" => json!({"current_version":"1.1.0","upgrade_available":false}),
                    _ => json!({"current_version":"1.0.0","upgrade_available":false,
                        "blocked_reason":"a saved native session is required to load a new Provider version"}),
                })
            })
            .post(
                move |Path(id): Path<String>, Query(query): Query<HashMap<String, String>>| {
                    let record = Arc::clone(&record);
                    async move {
                        let mut keys: Vec<_> =
                            query.iter().map(|(key, value)| format!("{key}={value}")).collect();
                        keys.sort();
                        record.lock().unwrap().push(format!("{id}?{}", keys.join("&")));
                        (StatusCode::ACCEPTED, "reloading installed Provider version")
                    }
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = Client::fixture(base);

        assert_eq!(
            update(&client, "old", false, false, None).await.unwrap(),
            "updating to 1.1.0"
        );
        assert_eq!(
            update(&client, "current", false, false, None)
                .await
                .unwrap(),
            "already current (1.1.0)"
        );
        let blocked = update(&client, "unsaved", true, false, None)
            .await
            .unwrap_err();
        assert!(
            blocked.to_string().contains("saved native session"),
            "{blocked}"
        );
        assert_eq!(
            *posted.lock().unwrap(),
            ["old?expected_generation_digest=sha256:new&upgrade_provider=true"]
        );
        server.abort();
    }

    #[test]
    fn request_targets_keep_the_query_signed() {
        let url = Url::parse("https://cowboy.example/base/api/x?a=1").unwrap();
        assert_eq!(request_target(&url), "/base/api/x?a=1");
    }
}
