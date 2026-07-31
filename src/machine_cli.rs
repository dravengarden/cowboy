//! Shared CLI for the stable Machine host and its `cowboy-agentd` transition
//! alias. Provider and Zed lifecycle subcommands join this surface without
//! copying the ACP broker implementation.

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context as _, bail};
use clap::{Parser, ValueEnum};
use futures::{SinkExt as _, StreamExt as _};
use serde::{Deserialize, Serialize};
use tokio_tungstenite::tungstenite::Message;

use crate::agentd::{AgentdArgs, SpawnMode};
use crate::machine_auth::MachineIdentity;
use crate::machine_protocol::{
    ComponentInventory, ConnectionMode, MACHINE_PROTOCOL_VERSION, MIN_MACHINE_PROTOCOL_VERSION,
    MachineFrame, MachineHello, Platform,
};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliSpawnMode {
    Direct,
    SystemdUser,
}

#[derive(Debug, Parser)]
#[command(version)]
pub struct Args {
    #[arg(long, default_value = "/run/user/1000/cowboy/agentd.sock")]
    socket: PathBuf,
    #[arg(long, default_value = "cowboy-acp-worker")]
    worker_command: PathBuf,
    /// Optional bootstrap generation. Production leaves this empty and lets
    /// the active Cowboy controller declare the desired generation + exact
    /// worker executable over IPC.
    #[arg(long, default_value = "")]
    desired_generation: String,
    #[arg(long, value_enum, default_value_t = CliSpawnMode::Direct)]
    spawn_mode: CliSpawnMode,
    // ACP itself permits one 60 s handshake plus one retry; stay above both so
    // the Machine host never kills the legitimate retry path prematurely.
    #[arg(long, default_value_t = 135)]
    worker_ready_timeout_seconds: u64,
    /// Cowboy controller base URL. Omit for the existing local-only broker.
    #[arg(long, env = "COWBOY_MACHINE_CONTROLLER_URL")]
    controller_url: Option<String>,
    #[arg(long, env = "COWBOY_MACHINE_ID", default_value = "local")]
    machine_id: String,
    #[arg(long, env = "COWBOY_MACHINE_DISPLAY_NAME")]
    display_name: Option<String>,
    #[arg(
        long,
        env = "COWBOY_MACHINE_STATE_DIR",
        default_value = ".cowboy-machine"
    )]
    state_dir: PathBuf,
    /// One-time secret. Prefer the environment variable so it is absent from
    /// shell history; omit after the first successful enrollment.
    #[arg(long, env = "COWBOY_MACHINE_ENROLLMENT_TOKEN")]
    enrollment_token: Option<String>,
}

#[derive(Serialize)]
struct EnrollmentRequest<'a> {
    token: &'a str,
    public_key: &'a str,
}

#[derive(Deserialize)]
struct EnrollmentResponse {
    machine_id: String,
    fingerprint: String,
}

/// Parse the stable local-mode CLI and run the existing detached ACP broker.
///
/// # Errors
/// Returns when broker startup or its listener fails.
pub async fn run(command_name: &'static str) -> anyhow::Result<()> {
    // Unlike the full Cowboy daemon, the small Machine binary does not start
    // SQLx first. Make the shared TLS provider explicit instead of depending
    // on another subsystem's initialization order.
    let _ = rustls::crypto::ring::default_provider().install_default();
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let args =
        Args::parse_from(std::iter::once(command_name.to_owned()).chain(std::env::args().skip(1)));
    let agentd = AgentdArgs {
        socket: args.socket,
        worker_command: args.worker_command,
        desired_generation: args.desired_generation,
        spawn_mode: match args.spawn_mode {
            CliSpawnMode::Direct => SpawnMode::Direct,
            CliSpawnMode::SystemdUser => SpawnMode::SystemdUser,
        },
        worker_ready_timeout: std::time::Duration::from_secs(args.worker_ready_timeout_seconds),
    };
    let Some(controller_url) = args.controller_url else {
        return crate::agentd::run(agentd).await;
    };
    validate_controller_url(&controller_url)?;
    let identity = MachineIdentity::load_or_create(&args.state_dir)?;
    let display_name = args.display_name.unwrap_or_else(default_display_name);
    if let Some(token) = args.enrollment_token.as_deref() {
        enroll(&controller_url, token, identity.public_key()).await?;
    }
    let controller = controller_loop(controller_url, args.machine_id, display_name, identity);
    tokio::try_join!(crate::agentd::run(agentd), controller)?;
    Ok(())
}

fn validate_controller_url(value: &str) -> anyhow::Result<()> {
    let url = reqwest::Url::parse(value).context("parsing Machine controller URL")?;
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "::1" | "localhost"));
    if url.scheme() != "https" && !(url.scheme() == "http" && loopback) {
        bail!("remote Machine controller must use https; http is allowed only on loopback");
    }
    Ok(())
}

fn default_display_name() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "Cowboy Machine".to_owned())
}

async fn enroll(controller_url: &str, token: &str, public_key: &str) -> anyhow::Result<()> {
    let endpoint = format!(
        "{}/api/machine/enroll",
        controller_url.trim_end_matches('/')
    );
    let response = reqwest::Client::new()
        .post(endpoint)
        .json(&EnrollmentRequest { token, public_key })
        .send()
        .await
        .context("sending Machine enrollment")?
        .error_for_status()
        .context("Machine enrollment rejected")?
        .json::<EnrollmentResponse>()
        .await
        .context("decoding Machine enrollment response")?;
    tracing::info!(
        machine = %response.machine_id,
        fingerprint = %response.fingerprint,
        "Machine identity enrolled"
    );
    Ok(())
}

async fn controller_loop(
    controller_url: String,
    machine_id: String,
    display_name: String,
    identity: MachineIdentity,
) -> anyhow::Result<()> {
    let mut retry = Duration::from_secs(1);
    loop {
        match controller_connection(&controller_url, &machine_id, &display_name, &identity).await {
            Ok(()) => retry = Duration::from_secs(1),
            Err(error) => tracing::warn!(%error, "Machine controller disconnected"),
        }
        tokio::time::sleep(retry).await;
        retry = (retry * 2).min(Duration::from_secs(30));
    }
}

async fn controller_connection(
    controller_url: &str,
    machine_id: &str,
    display_name: &str,
    identity: &MachineIdentity,
) -> anyhow::Result<()> {
    let mut endpoint = reqwest::Url::parse(controller_url).context("parsing controller URL")?;
    endpoint
        .set_scheme(if endpoint.scheme() == "https" {
            "wss"
        } else {
            "ws"
        })
        .map_err(|()| anyhow::anyhow!("invalid controller URL scheme"))?;
    endpoint.set_path("/api/machine/connect");
    endpoint.set_query(None);
    let (mut socket, _) = tokio_tungstenite::connect_async(endpoint.as_str())
        .await
        .context("connecting Machine WebSocket")?;
    let challenge = receive_frame(&mut socket).await?;
    let MachineFrame::Challenge {
        challenge_id,
        nonce,
        expires_at_ms,
    } = challenge
    else {
        bail!("controller did not begin with a challenge");
    };
    if expires_at_ms < unix_ms() {
        bail!("controller challenge already expired");
    }
    let mut hello = MachineHello {
        machine_id: machine_id.to_owned(),
        display_name: display_name.to_owned(),
        platform: current_platform(),
        arch: std::env::consts::ARCH.to_owned(),
        connection_mode: ConnectionMode::OutboundTls,
        min_protocol: MIN_MACHINE_PROTOCOL_VERSION,
        max_protocol: MACHINE_PROTOCOL_VERSION,
        min_runtime_protocol: crate::runtime_wire::MIN_PROTOCOL_VERSION,
        max_runtime_protocol: crate::runtime_wire::PROTOCOL_VERSION,
        host_build: env!("CARGO_PKG_VERSION").to_owned(),
        challenge_id: Some(challenge_id.clone()),
        challenge_signature: None,
        components: Vec::<ComponentInventory>::new(),
    };
    let proof =
        crate::machine_protocol::challenge_proof_v1(&challenge_id, &nonce, expires_at_ms, &hello);
    hello.challenge_signature = Some(identity.sign(&proof)?);
    send_frame(&mut socket, &MachineFrame::Hello { hello }).await?;
    let MachineFrame::Welcome {
        heartbeat_interval_ms,
        ..
    } = receive_frame(&mut socket).await?
    else {
        bail!("controller rejected Machine hello");
    };
    tracing::info!(machine = machine_id, "Machine controller authenticated");
    let mut heartbeat =
        tokio::time::interval(Duration::from_millis(heartbeat_interval_ms.max(1_000)));
    heartbeat.tick().await;
    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                send_frame(&mut socket, &MachineFrame::Heartbeat { sent_at_ms: unix_ms() }).await?;
            }
            message = socket.next() => {
                match message {
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => return Ok(()),
                    Some(Ok(Message::Ping(value))) => socket.send(Message::Pong(value)).await?,
                    Some(Ok(_)) => {}
                }
            }
        }
    }
}

fn current_platform() -> Platform {
    if cfg!(target_os = "macos") {
        Platform::Macos
    } else {
        Platform::Linux
    }
}

fn unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |value| value.as_millis().try_into().unwrap_or(i64::MAX))
}

async fn send_frame<S>(socket: &mut S, frame: &MachineFrame) -> anyhow::Result<()>
where
    S: futures::Sink<Message> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    let text = serde_json::to_string(frame).context("serializing Machine frame")?;
    socket
        .send(Message::Text(text.into()))
        .await
        .context("sending Machine frame")
}

async fn receive_frame<S>(socket: &mut S) -> anyhow::Result<MachineFrame>
where
    S: futures::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => {
                return serde_json::from_str(&text).context("parsing Machine frame");
            }
            Some(Ok(Message::Close(_))) | None => bail!("Machine WebSocket closed"),
            Some(Err(error)) => return Err(error).context("reading Machine WebSocket"),
            Some(Ok(_)) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::validate_controller_url;

    #[test]
    fn remote_controller_requires_https() {
        assert!(validate_controller_url("https://cowboy.example").is_ok());
        assert!(validate_controller_url("http://cowboy.example").is_err());
    }

    #[test]
    fn loopback_controller_allows_plaintext_for_hermetic_tests() {
        assert!(validate_controller_url("http://127.0.0.1:43333").is_ok());
        assert!(validate_controller_url("http://localhost:43333").is_ok());
    }
}
