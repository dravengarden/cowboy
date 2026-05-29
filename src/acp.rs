//! ACP client backend.
//!
//! cowboy is the ACP *client* (design §2): it implements the crate's [`Client`]
//! trait and drives each agent (the ACP *server*) over stdio. This module is
//! the only place that touches the `agent-client-protocol` crate, so a crate
//! bump is contained here.
//!
//! The crate's connection is single-threaded (`?Send`, spawn-local), so each
//! agent runs on its own OS thread inside a current-thread runtime +
//! `LocalSet`. The agent's `Client` callbacks translate every ACP
//! `SessionUpdate` into a normalized [`crate::core::Event`] on the shared
//! [`Hub`]; commands flow in over a `Send` channel ([`AgentCommand`]).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::rc::Rc;

use agent_client_protocol::{
    Agent, CancelNotification, Client, ClientSideConnection, ContentBlock, Error,
    InitializeRequest, NewSessionRequest, PermissionOptionId, PermissionOptionKind, PromptRequest,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse, SessionModeId,
    SessionNotification, SetSessionModeRequest, V1,
};
use anyhow::{Context, Result};
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use crate::core::{Event, Hub, Status};
use crate::provider::LaunchSpec;

/// A command from a client, routed by the supervisor to an agent thread.
#[derive(Debug)]
pub enum AgentCommand {
    /// Send a user turn. The full ACP content array is forwarded to the
    /// upstream agent verbatim — image / audio / resource blocks make it
    /// through (subject to the upstream's own capabilities), not just text.
    Prompt(Vec<ContentBlock>),
    /// Cancel the current turn (ACP `session/cancel`).
    Cancel,
    /// Answer a pending permission request (`None` = cancelled / no choice).
    Permission {
        request_id: String,
        option_id: Option<String>,
    },
}

/// Pending permission requests awaiting a client answer, keyed by request id.
/// `Rc`/`RefCell` is sound here: the connection handler tasks and the command
/// loop all run on the same single-threaded `LocalSet`.
type Pending = Rc<RefCell<HashMap<String, oneshot::Sender<Option<String>>>>>;

/// The client handler that fans agent updates out to all connected frontends
/// via the [`Hub`], and bridges permission requests to client answers.
struct CowboyClient {
    hub: Hub,
    session_id: String,
    pending: Pending,
    next_perm: Rc<Cell<u64>>,
}

#[async_trait::async_trait(?Send)]
impl Client for CowboyClient {
    async fn request_permission(
        &self,
        args: RequestPermissionRequest,
    ) -> Result<RequestPermissionResponse, Error> {
        let n = self.next_perm.get();
        self.next_perm.set(n + 1);
        let request_id = format!("perm-{n}");

        let tool_call = serde_json::to_value(&args.tool_call).unwrap_or(serde_json::Value::Null);
        let options = serde_json::to_value(&args.options).unwrap_or(serde_json::Value::Null);

        let (tx, rx) = oneshot::channel();
        self.pending.borrow_mut().insert(request_id.clone(), tx);
        self.hub.push(
            &self.session_id,
            Event::PermissionRequest {
                request_id,
                tool_call,
                options,
            },
        );

        // Block this tool call until a client answers (first-response-wins is
        // enforced by the command loop, which resolves exactly one sender).
        let chosen = rx.await.unwrap_or(None);
        let outcome = match chosen {
            Some(option_id) => RequestPermissionOutcome::Selected {
                option_id: PermissionOptionId(option_id.into()),
            },
            None => RequestPermissionOutcome::Cancelled,
        };
        Ok(RequestPermissionResponse {
            outcome,
            meta: None,
        })
    }

    async fn session_notification(&self, args: SessionNotification) -> Result<(), Error> {
        // Pass the whole ACP SessionUpdate through as JSON (design §5): message
        // / thought chunks, tool calls + updates, plan, available commands, and
        // mode all reach the UI without per-variant re-modelling.
        match serde_json::to_value(&args.update) {
            Ok(update) => self.hub.push(&self.session_id, Event::Update { update }),
            Err(e) => tracing::warn!(error = %e, "serializing session update"),
        }
        Ok(())
    }
}

/// OS-thread entry point: run one agent session to completion on a
/// current-thread runtime + `LocalSet`. A failure marks the session crashed.
pub fn run_agent(
    spec: &LaunchSpec,
    session_id: &str,
    cwd: PathBuf,
    cmd_rx: mpsc::UnboundedReceiver<AgentCommand>,
    hub: &Hub,
) {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            hub.set_status(session_id, Status::Crashed, Some(format!("runtime: {e}")));
            return;
        }
    };
    let local = tokio::task::LocalSet::new();
    let result = local.block_on(&rt, agent_main(spec, session_id, cwd, cmd_rx, hub));
    match result {
        Ok(()) => hub.set_status(session_id, Status::Exited, None),
        Err(e) => {
            tracing::error!(session = session_id, error = %e, "agent session ended with error");
            hub.set_status(session_id, Status::Crashed, Some(e.to_string()));
        }
    }
}

#[allow(clippy::too_many_lines)] // one cohesive handshake + command loop
async fn agent_main(
    spec: &LaunchSpec,
    session_id: &str,
    cwd: PathBuf,
    mut cmd_rx: mpsc::UnboundedReceiver<AgentCommand>,
    hub: &Hub,
) -> Result<()> {
    let cwd =
        std::path::absolute(&cwd).with_context(|| format!("resolving cwd {}", cwd.display()))?;
    tracing::info!(provider = spec.id, session = session_id, cwd = %cwd.display(), "spawning agent");

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

    let pending: Pending = Rc::new(RefCell::new(HashMap::new()));
    let client = CowboyClient {
        hub: hub.clone(),
        session_id: session_id.to_owned(),
        pending: pending.clone(),
        next_perm: Rc::new(Cell::new(0)),
    };

    let (conn, io_task) = ClientSideConnection::new(client, outgoing, incoming, |fut| {
        tokio::task::spawn_local(fut);
    });
    let conn = Rc::new(conn);
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
    let acp_id = session.session_id;
    tracing::info!(session = session_id, acp_id = %acp_id.0, "session created");
    hub.set_status(session_id, Status::Running, None);

    // Match Zed's claude-acp default UX: open at `bypassPermissions` if the
    // upstream advertises it. This is what most users want for an agent
    // panel — explicit permission prompts dominate the UX otherwise — and
    // it matches the v0 spec the user is iterating against.
    if let Some(modes) = session.modes.as_ref() {
        let want = "bypassPermissions";
        let has = modes
            .available_modes
            .iter()
            .any(|m| m.id.0.as_ref() == want);
        if has && modes.current_mode_id.0.as_ref() != want {
            let req = SetSessionModeRequest {
                session_id: acp_id.clone(),
                mode_id: SessionModeId(Arc::from(want)),
                meta: None,
            };
            if let Err(e) = conn.set_session_mode(req).await {
                tracing::warn!(error = ?e, "set_session_mode bypassPermissions failed");
            } else {
                tracing::info!(session = session_id, "mode → bypassPermissions");
                // Also echo into the timeline so the UI mode chip is up to
                // date without round-tripping through a session_update.
                hub.push(
                    session_id,
                    Event::Update {
                        update: serde_json::json!({
                            "sessionUpdate": "current_mode_update",
                            "currentModeId": want,
                        }),
                    },
                );
            }
        }
    }

    // Command loop. Prompts run as concurrent local tasks so Cancel and
    // Permission answers are still processed while a turn is in flight.
    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            AgentCommand::Prompt(blocks) => {
                hub.set_status(session_id, Status::Busy, None);
                // Echo each user content block into the timeline so every
                // client (Web UI, phone, Zed via bridge) sees it — the
                // upstream agent may not stream a user_message_chunk back.
                // One Hub event per block so each renders as its own bubble
                // (text + image, today; future: audio etc.).
                for block in &blocks {
                    let content = serde_json::to_value(block).unwrap_or(serde_json::Value::Null);
                    hub.push(
                        session_id,
                        Event::Update {
                            update: serde_json::json!({
                                "sessionUpdate": "user_message_chunk",
                                "content": content,
                            }),
                        },
                    );
                }
                let conn = conn.clone();
                let hub = hub.clone();
                let sid = session_id.to_owned();
                let acp = acp_id.clone();
                tokio::task::spawn_local(async move {
                    let stop_reason = match conn
                        .prompt(PromptRequest {
                            session_id: acp,
                            prompt: blocks,
                            meta: None,
                        })
                        .await
                    {
                        Ok(r) => format!("{:?}", r.stop_reason),
                        Err(e) => format!("error: {e}"),
                    };
                    hub.push(&sid, Event::TurnEnd { stop_reason });
                    hub.set_status(&sid, Status::Running, None);
                });
            }
            AgentCommand::Cancel => {
                let _ = conn
                    .cancel(CancelNotification {
                        session_id: acp_id.clone(),
                        meta: None,
                    })
                    .await;
            }
            AgentCommand::Permission {
                request_id,
                option_id,
            } => {
                if let Some(tx) = pending.borrow_mut().remove(&request_id) {
                    let _ = tx.send(option_id.clone());
                }
                hub.push(
                    session_id,
                    Event::PermissionResolved {
                        request_id,
                        option_id,
                    },
                );
            }
        }
    }
    Ok(())
}

/// The client handler for the one-shot `try-agent` debug command: prints
/// streamed text to stdout and auto-approves the first allow-style permission.
struct OneshotClient;

#[async_trait::async_trait(?Send)]
impl Client for OneshotClient {
    async fn request_permission(
        &self,
        args: RequestPermissionRequest,
    ) -> Result<RequestPermissionResponse, Error> {
        let allow = args.options.iter().find(|o| {
            matches!(
                o.kind,
                PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways
            )
        });
        let outcome = match allow {
            Some(opt) => {
                tracing::info!(option = %opt.name, "auto-approving permission (try-agent)");
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
        use agent_client_protocol::SessionUpdate;
        use std::io::Write as _;
        match args.update {
            SessionUpdate::AgentMessageChunk { content }
            | SessionUpdate::AgentThoughtChunk { content } => {
                if let ContentBlock::Text(t) = content {
                    print!("{}", t.text);
                    let _ = std::io::stdout().flush();
                }
            }
            SessionUpdate::ToolCall(tc) => eprintln!("\n[tool-call] {}", tc.title),
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
    tracing::info!(provider = spec.id, cwd = %cwd.display(), "spawning agent");

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

    let (conn, io_task) = ClientSideConnection::new(OneshotClient, outgoing, incoming, |fut| {
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
