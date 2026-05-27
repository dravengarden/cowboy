//! ACP client backend.
//!
//! cowboy is the ACP *client* (design §2): it implements the crate's [`Client`]
//! trait and drives each agent (the ACP *server*) over stdio. This module is
//! the only place that touches the `agent-client-protocol` crate, so a crate
//! bump is contained here.
//!
//! The crate's connection is single-threaded (`?Send`, spawn-local), so each
//! agent runs inside a `tokio::task::LocalSet`.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::Stdio;

use agent_client_protocol::{
    Agent, Client, ClientSideConnection, ContentBlock, Error, InitializeRequest, NewSessionRequest,
    PermissionOptionKind, PromptRequest, RequestPermissionOutcome, RequestPermissionRequest,
    RequestPermissionResponse, SessionNotification, SessionUpdate, V1,
};
use anyhow::{Context, Result};
use tokio::process::Command;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use crate::provider::LaunchSpec;

/// The client handler: receives agent-initiated requests/notifications.
///
/// v0 behaviour is a placeholder for the eventual UI fan-out (design §5):
/// session updates are printed, and permission requests are auto-approved by
/// selecting the first allow-style option. The real client routes these to all
/// connected frontends and applies first-response-wins.
struct CowboyClient;

#[async_trait::async_trait(?Send)]
impl Client for CowboyClient {
    async fn request_permission(
        &self,
        args: RequestPermissionRequest,
    ) -> Result<RequestPermissionResponse, Error> {
        // Placeholder policy: pick the first allow-style option, else cancel.
        let allow = args.options.iter().find(|o| {
            matches!(
                o.kind,
                PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways
            )
        });
        let outcome = match allow {
            Some(opt) => {
                tracing::info!(option = %opt.name, "auto-approving permission (v0 placeholder)");
                RequestPermissionOutcome::Selected {
                    option_id: opt.id.clone(),
                }
            }
            None => RequestPermissionOutcome::Cancelled,
        };
        Ok(RequestPermissionResponse {
            outcome,
            meta: None,
        })
    }

    async fn session_notification(&self, args: SessionNotification) -> Result<(), Error> {
        match args.update {
            SessionUpdate::AgentMessageChunk { content }
            | SessionUpdate::AgentThoughtChunk { content } => {
                if let ContentBlock::Text(t) = content {
                    print!("{}", t.text);
                    let _ = std::io::stdout().flush();
                }
            }
            SessionUpdate::ToolCall(tc) => {
                eprintln!("\n[tool-call] {}", tc.title);
            }
            SessionUpdate::ToolCallUpdate(_) | SessionUpdate::Plan(_) => {}
            other => tracing::debug!(?other, "session update"),
        }
        Ok(())
    }
}

/// Spawn `spec`'s adapter, run the full ACP handshake, send one `prompt` in a
/// fresh session under `cwd`, and stream updates to stdout. Used by the
/// `try-agent` debug command to verify a provider end-to-end.
///
/// Must run inside a `LocalSet` (the connection is `!Send`).
pub async fn run_oneshot(spec: &LaunchSpec, cwd: PathBuf, prompt: String) -> Result<()> {
    let cwd =
        std::path::absolute(&cwd).with_context(|| format!("resolving cwd {}", cwd.display()))?;
    tracing::info!(provider = spec.id, resume = spec.resume, cwd = %cwd.display(), "spawning agent");

    let mut child = Command::new(&spec.command)
        .args(&spec.args)
        .current_dir(&cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("spawning provider {} ({})", spec.id, spec.command))?;

    let outgoing = child.stdin.take().context("child stdin")?.compat_write();
    let incoming = child.stdout.take().context("child stdout")?.compat();

    let (conn, io_task) = ClientSideConnection::new(CowboyClient, outgoing, incoming, |fut| {
        tokio::task::spawn_local(fut);
    });
    tokio::task::spawn_local(async move {
        if let Err(e) = io_task.await {
            tracing::error!(error = %e, "acp io task ended");
        }
    });

    conn.initialize(InitializeRequest {
        protocol_version: V1,
        client_capabilities: agent_client_protocol::ClientCapabilities::default(),
        meta: None,
    })
    .await
    .context("initialize")?;

    let session = conn
        .new_session(NewSessionRequest {
            cwd,
            mcp_servers: vec![],
            meta: None,
        })
        .await
        .context("new_session")?;
    tracing::info!(session_id = %session.session_id.0, "session created");

    let resp = conn
        .prompt(PromptRequest {
            session_id: session.session_id,
            prompt: vec![ContentBlock::from(prompt)],
            meta: None,
        })
        .await
        .context("prompt")?;

    println!("\n--- stop: {:?} ---", resp.stop_reason);
    Ok(())
}
