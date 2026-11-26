//! Detached target-owned execution keeper. The control connection is never the
//! owner of a native executor or its jobs. A lost keeper cannot be relaunched
//! under the same incarnation, and admitted effects cannot be retried as new.

pub mod file_helper;
mod identity;
mod ledger;
mod read_only;
pub mod user_mcp;

use crate::execution_protocol::{
    self as wire, Command, LaunchContract, Outcome, Refusal, Request, Response, Scope,
};
use anyhow::{Context as _, Result, ensure};
use ledger::{Admission, EventError, Ledger};
use parking_lot::Mutex;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::process::Command as ProcessCommand;
use tokio::sync::{Semaphore, mpsc, watch};

pub struct Args {
    pub contract: PathBuf,
    pub state_dir: PathBuf,
    pub socket: Option<PathBuf>,
}

/// Validate the private launch identity before initializing target-local logs.
///
/// # Errors
/// Invalid or oversized contracts fail without logging their contents.
pub fn log_context(path: &Path) -> Result<crate::logs::Context> {
    let contract: LaunchContract =
        serde_json::from_slice(&crate::logs::storage::private_read(path, 64 * 1024)?)?;
    validate_contract(&contract)?;
    let mut context = crate::logs::Context::new("cowboy-execution-host");
    context.session = contract.session_id;
    context.machine = contract.binding.environment.machine_id;
    context.environment = contract.binding.environment.id;
    context.generation = contract.executor.version;
    Ok(context)
}

struct Host {
    scope: Scope,
    call_context: Option<String>,
    /// A managed child's environment: every request is constrained read-only.
    read_only: bool,
    capability_digest: [u8; 32],
    initialization: Value,
    cwd: PathBuf,
    source_path: PathBuf,
    state_dir: PathBuf,
    environment: std::collections::BTreeMap<String, String>,
    neutral_user_mcp_cwd: Option<PathBuf>,
    user_mcp_key: [u8; 32],
    /// Strict lock order across the keeper: ALWAYS acquire `user_mcp` BEFORE
    /// `ledger`, never `ledger` before `user_mcp`.
    user_mcp: Mutex<user_mcp::UserMcpState>,
    inode: (u64, u64),
    ledger: Mutex<Ledger>,
    writer: mpsc::Sender<Value>,
    stop: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    stopped: watch::Receiver<bool>,
    close: tokio::sync::Notify,
    /// Last request from an attached worker. Its executor client long-polls
    /// Events for as long as it lives, so silence means no worker remains.
    attended: Mutex<std::time::Instant>,
}

/// A worker that has not polled for this long is gone: its native client
/// cannot reattach to the stdio of processes it started. A local session's
/// worker unit takes its children with it; reap them here the same way.
/// Long enough to ride out Controller restarts and Machine maintenance.
const UNATTENDED_PROCESS_REAP: Duration = Duration::from_secs(30 * 60);

async fn read_json<R: tokio::io::AsyncBufRead + Unpin>(reader: &mut R) -> Result<Value> {
    read_json_bounded(reader, wire::MAX_FRAME_BYTES).await
}

async fn read_json_bounded<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
    limit: usize,
) -> Result<Value> {
    let mut bytes = Vec::new();
    let count = reader
        .take((limit + 1) as u64)
        .read_until(b'\n', &mut bytes)
        .await?;
    ensure!(
        count > 0 && count <= limit && bytes.last() == Some(&b'\n'),
        "invalid execution frame"
    );
    serde_json::from_slice(&bytes).context("invalid execution JSON")
}

async fn write_json<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut W,
    value: &impl serde::Serialize,
) -> Result<()> {
    let mut bytes = serde_json::to_vec(value)?;
    ensure!(
        bytes.len() < wire::MAX_FRAME_BYTES,
        "execution response exceeds limit"
    );
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

fn validate_contract(contract: &LaunchContract) -> Result<()> {
    contract.binding.validate().map_err(anyhow::Error::msg)?;
    ensure!(
        contract.schema == wire::SCHEMA && wire::valid_operation_id(&contract.session_id),
        "invalid execution launch contract"
    );
    ensure!(
        contract.capability.len() == 64
            && contract.capability.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid execution capability"
    );
    ensure!(
        Path::new(&contract.executor.command).is_absolute()
            && contract.executor.sha256.len() == 64
            && contract
                .executor
                .sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit())
            && contract.binding.environment.executor_digest
                == format!("sha256:{}", contract.executor.sha256),
        "invalid executor identity"
    );
    ensure!(
        !contract.executor.version.is_empty() && contract.executor.version.len() <= 64,
        "invalid executor version"
    );
    ensure!(
        contract.environment.len() <= crate::execution_target_environment::MAX
            && contract.environment.iter().all(|(name, value)| {
                crate::execution_target_environment::allowed(name)
                    && value.len() <= 8192
                    && !value.contains('\0')
            }),
        "invalid target environment"
    );
    identity::validate_agent_cli(contract)?;
    ensure!(
        contract.call_context.as_ref().is_none_or(|path| {
            Path::new(path).is_absolute() && path.len() <= 4096 && !path.contains('\0')
        }),
        "invalid call context"
    );
    ensure!(
        contract.managed.is_some() == contract.binding.managed.is_some()
            && contract.managed.as_ref().is_none_or(|managed| {
                // A managed child cannot itself delegate through a call context.
                contract.call_context.is_none()
                    && Path::new(&managed.round_path).is_absolute()
                    && managed.round_path.len() <= 4096
                    && !managed.round_path.contains('\0')
                    && contract
                        .binding
                        .managed
                        .as_ref()
                        .map(|binding| binding.profile)
                        == Some(managed.profile)
            }),
        "invalid managed execution constraint"
    );
    Ok(())
}

impl Host {
    fn current_workspace(&self) -> bool {
        std::fs::metadata(&self.cwd)
            .is_ok_and(|metadata| (metadata.dev(), metadata.ino()) == self.inode)
    }

    async fn request(&self, request: Request) -> Response {
        let digest: [u8; 32] = Sha256::digest(request.capability.as_bytes()).into();
        if request.schema != wire::SCHEMA
            || request.scope != self.scope
            || digest != self.capability_digest
        {
            return Response::Refused {
                reason: Refusal::Unauthorized,
            };
        }
        let refreshes_attended = match &request.command {
            Command::Invoke { invocation, .. } => {
                invocation.method != user_mcp::USER_MCP_STOP_METHOD
            }
            Command::Observe { .. } | Command::Events { .. } => true,
            _ => false,
        };
        if refreshes_attended {
            *self.attended.lock() = std::time::Instant::now();
        }
        let result = match request.command {
            Command::Shutdown => {
                self.ledger.lock().lose();
                if let Some(stop) = self.stop.lock().take() {
                    let _ = stop.send(());
                }
                let mut stopped = self.stopped.clone();
                let finished = tokio::time::timeout(Duration::from_secs(15), async {
                    while !*stopped.borrow_and_update() {
                        stopped
                            .changed()
                            .await
                            .map_err(|_| Refusal::EnvironmentLost)?;
                    }
                    Ok::<_, Refusal>(())
                })
                .await;
                if matches!(finished, Ok(Ok(()))) {
                    Ok(Response::Closed)
                } else {
                    Err(Refusal::EnvironmentLost)
                }
            }
            Command::Describe => {
                if self.ledger.lock().lost {
                    Err(Refusal::EnvironmentLost)
                } else if !self.current_workspace() {
                    Err(Refusal::WorkspaceChanged)
                } else {
                    Ok(Response::Ready {
                        scope: self.scope.clone(),
                        initialization: self.initialization.clone(),
                        event_cursor: self.ledger.lock().event_cursor(),
                    })
                }
            }
            Command::Invoke {
                invocation,
                wait_ms,
            } => {
                if wait_ms > wire::MAX_WAIT_MS {
                    Err(Refusal::InvalidRequest)
                } else if invocation.method == user_mcp::USER_MCP_STOP_METHOD {
                    // Kill switch / teardown stop stays reachable even if the workspace changed.
                    self.handle_stop_user_mcp(&invocation).await
                } else if !self.current_workspace() {
                    Err(Refusal::WorkspaceChanged)
                } else if invocation.method == user_mcp::USER_MCP_LIST_METHOD {
                    Ok(Response::Operation {
                        outcome: Outcome::Completed {
                            reply: self.handle_list_user_mcp(&invocation),
                        },
                    })
                } else if invocation.method == user_mcp::USER_MCP_START_METHOD {
                    self.handle_start_user_mcp(&invocation, wait_ms).await
                } else {
                    // The one native frame this invocation may send. A read-only
                    // environment decides once; a refusal is never forwarded.
                    let native = if self.read_only {
                        read_only::decide(&invocation.method, &invocation.params, &self.cwd)
                    } else {
                        read_only::Decision::Forward(invocation.params.clone())
                    };
                    if let read_only::Decision::Refuse(reply) = native {
                        // Deterministic and effect-free: answered without admission.
                        Ok(Response::Operation {
                            outcome: Outcome::Completed { reply },
                        })
                    } else {
                        let read_only::Decision::Forward(native) = native else {
                            unreachable!("refusals return above")
                        };
                        match self.admit(&invocation, &native) {
                            Ok(Admission::Existing(receiver) | Admission::New { receiver, .. }) => {
                                Ok(Response::Operation {
                                    outcome: wait_outcome(receiver, wait_ms).await,
                                })
                            }
                            Err(reason) => Err(reason),
                        }
                    }
                }
            }
            Command::Observe {
                operation_id,
                wait_ms,
            } => {
                if wait_ms > wire::MAX_WAIT_MS || !wire::valid_operation_id(&operation_id) {
                    Err(Refusal::InvalidRequest)
                } else {
                    let receiver = self.ledger.lock().observe(&operation_id);
                    Ok(Response::Operation {
                        outcome: match receiver {
                            Some(receiver) => wait_outcome(receiver, wait_ms).await,
                            None => Outcome::Missing,
                        },
                    })
                }
            }
            Command::Events { after, wait_ms } => {
                if wait_ms > wire::MAX_WAIT_MS {
                    Err(Refusal::InvalidRequest)
                } else {
                    self.events(after, wait_ms).await
                }
            }
        };
        result.unwrap_or_else(|reason| Response::Refused { reason })
    }

    fn handle_list_user_mcp(&self, invocation: &wire::Invocation) -> Value {
        if self.read_only || self.neutral_user_mcp_cwd.is_none() {
            return user_mcp::error_reply(
                "user_mcp_unsupported",
                "user MCP servers are not supported in read-only environments",
            );
        }
        let (gate, _) = match user_mcp::extract_manager_gate(&invocation.params) {
            Ok(parsed) => parsed,
            Err(reply) => return reply,
        };
        let catalog = user_mcp::resolve_catalog(
            &self.environment,
            &self.state_dir,
            &self.source_path,
            &self.cwd,
            &gate.allowlist,
            &self.user_mcp_key,
        );
        let servers: Vec<Value> = catalog
            .servers
            .into_iter()
            .map(|server| {
                json!({
                    "name": server.name,
                    "scope": server.scope,
                    "fingerprint": server.fingerprint,
                })
            })
            .collect();
        let omitted: Vec<Value> = catalog
            .omitted
            .into_iter()
            .map(|entry| {
                json!({
                    "name": entry.name,
                    "reason": entry.reason,
                })
            })
            .collect();
        json!({"result": {"servers": servers, "omitted": omitted}})
    }

    async fn handle_start_user_mcp(
        &self,
        invocation: &wire::Invocation,
        wait_ms: u64,
    ) -> Result<Response, Refusal> {
        let (gate, clean_invocation) = match self.prepare_start_user_mcp(invocation) {
            Ok(parsed) => parsed,
            Err(reply) => {
                return Ok(Response::Operation {
                    outcome: Outcome::Completed { reply },
                });
            }
        };
        if let Some(receiver) = self.observe_existing_user_mcp(&clean_invocation)? {
            return Ok(Response::Operation {
                outcome: wait_outcome(receiver, wait_ms).await,
            });
        }
        let Some(configured_cwd) = &self.neutral_user_mcp_cwd else {
            return Ok(Response::Operation {
                outcome: Outcome::Completed {
                    reply: user_mcp::error_reply(
                        "user_mcp_unsupported",
                        "user MCP neutral working directory unavailable",
                    ),
                },
            });
        };
        let neutral_cwd = if configured_cwd.exists() {
            configured_cwd.clone()
        } else {
            match user_mcp::create_neutral_cwd(&self.scope.incarnation) {
                Ok(recreated) => recreated,
                Err(_) => {
                    tracing::warn!(
                        reason = "neutral_cwd_unavailable",
                        "user MCP neutral cwd recreation failed"
                    );
                    return Ok(Response::Operation {
                        outcome: Outcome::Completed {
                            reply: user_mcp::error_reply(
                                "user_mcp_unsupported",
                                "user MCP neutral working directory unavailable",
                            ),
                        },
                    });
                }
            }
        };
        let (name, process_id, fingerprint) =
            match parse_start_user_mcp_args(&clean_invocation.params) {
                Ok(args) => args,
                Err(reply) => {
                    return Ok(Response::Operation {
                        outcome: Outcome::Completed { reply },
                    });
                }
            };
        let epoch = match self.snapshot_user_mcp_epoch(&process_id) {
            Ok(epoch) => epoch,
            Err(reply) => {
                return Ok(Response::Operation {
                    outcome: Outcome::Completed { reply },
                });
            }
        };
        let native_params = match self.resolve_user_mcp_start(
            &gate.allowlist,
            &name,
            &process_id,
            &fingerprint,
            &neutral_cwd,
        ) {
            Ok(params) => params,
            Err(reply) => {
                return Ok(Response::Operation {
                    outcome: Outcome::Completed { reply },
                });
            }
        };
        let receiver = match self.admit_user_mcp_start(
            &clean_invocation,
            epoch,
            gate.config_revision,
            name,
            process_id,
            native_params,
        )? {
            Ok(receiver) => receiver,
            Err(reply) => {
                return Ok(Response::Operation {
                    outcome: Outcome::Completed { reply },
                });
            }
        };
        Ok(Response::Operation {
            outcome: wait_outcome(receiver, wait_ms).await,
        })
    }

    fn prepare_start_user_mcp(
        &self,
        invocation: &wire::Invocation,
    ) -> Result<(user_mcp::ManagerGate, wire::Invocation), Value> {
        if self.read_only {
            return Err(user_mcp::error_reply(
                "user_mcp_unsupported",
                "user MCP servers are not supported in read-only environments",
            ));
        }
        let (gate, clean_params) = user_mcp::extract_manager_gate(&invocation.params)?;
        Ok((
            gate,
            wire::Invocation {
                operation_id: invocation.operation_id.clone(),
                method: invocation.method.clone(),
                params: clean_params,
            },
        ))
    }

    fn observe_existing_user_mcp(
        &self,
        clean_invocation: &wire::Invocation,
    ) -> Result<Option<watch::Receiver<Outcome>>, Refusal> {
        let mut ledger = self.ledger.lock();
        if ledger.observe(&clean_invocation.operation_id).is_some() {
            match ledger.admit(clean_invocation)? {
                Admission::Existing(receiver) | Admission::New { receiver, .. } => {
                    Ok(Some(receiver))
                }
            }
        } else if ledger.lost {
            Err(Refusal::EnvironmentLost)
        } else {
            Ok(None)
        }
    }

    fn snapshot_user_mcp_epoch(&self, process_id: &str) -> Result<u64, Value> {
        let um = self.user_mcp.lock();
        if um.is_tombstoned(process_id) {
            return Err(user_mcp::error_reply(
                "user_mcp_stopped",
                "user MCP process was stopped",
            ));
        }
        Ok(um.stop_epoch)
    }

    fn resolve_user_mcp_start(
        &self,
        allowlist: &[String],
        name: &str,
        process_id: &str,
        fingerprint: &str,
        neutral_cwd: &Path,
    ) -> Result<Value, Value> {
        let catalog = user_mcp::resolve_catalog(
            &self.environment,
            &self.state_dir,
            &self.source_path,
            &self.cwd,
            allowlist,
            &self.user_mcp_key,
        );
        let Some(server) = catalog.servers.iter().find(|entry| entry.name == name) else {
            let code = catalog
                .omitted
                .iter()
                .find(|entry| entry.name == name)
                .map_or("user_mcp_config_drift", |entry| match entry.reason {
                    "not_allowed" => "user_mcp_not_allowed",
                    "unresolved_variable" => "unresolved_variable",
                    "unsupported_transport" => "unsupported_transport",
                    _ => "user_mcp_invalid",
                });
            return Err(user_mcp::error_reply(code, "user MCP server unavailable"));
        };
        if server.fingerprint != fingerprint {
            return Err(user_mcp::error_reply(
                "user_mcp_config_drift",
                "user MCP server configuration changed since discovery",
            ));
        }
        user_mcp::build_native_start_params(process_id, server, neutral_cwd).map_err(|_| {
            user_mcp::error_reply("user_mcp_invalid", "invalid user MCP launch parameters")
        })
    }

    /// Single atomic critical section (strict lock order: `user_mcp` -> `ledger`).
    /// Idempotency (`ledger.observe`) is checked FIRST so a concurrent duplicate
    /// invocation never hits `active_processes.contains_key` and returns a
    /// misleading deterministic error while the process is actually starting.
    fn admit_user_mcp_start(
        &self,
        clean_invocation: &wire::Invocation,
        epoch: u64,
        gate_config_revision: Option<u64>,
        name: String,
        process_id: String,
        native_params: Value,
    ) -> Result<Result<watch::Receiver<Outcome>, Value>, Refusal> {
        let mut um = self.user_mcp.lock();
        let mut ledger = self.ledger.lock();
        if ledger.observe(&clean_invocation.operation_id).is_some() || ledger.lost {
            return match ledger.admit(clean_invocation)? {
                Admission::Existing(receiver) | Admission::New { receiver, .. } => Ok(Ok(receiver)),
            };
        }
        if um.stop_epoch != epoch
            || um.is_tombstoned(&process_id)
            || gate_config_revision.is_some_and(|rev| rev < um.last_stop_config_revision)
        {
            return Ok(Err(user_mcp::error_reply(
                "user_mcp_stopped",
                "user MCP start fenced by concurrent stop",
            )));
        }
        if let Some(latest) = &um.latest_allowlist
            && !latest.iter().any(|allowed| allowed == &name)
        {
            return Ok(Err(user_mcp::error_reply(
                "user_mcp_not_allowed",
                "user MCP server excluded by updated allowlist",
            )));
        }
        if um.active_processes.len() >= user_mcp::MAX_ACTIVE_USER_SERVERS
            || um.active_processes.contains_key(&process_id)
        {
            return Ok(Err(user_mcp::error_reply(
                "user_mcp_capacity",
                "user MCP server capacity reached",
            )));
        }
        let permit = self.writer.try_reserve().map_err(|_| Refusal::Capacity)?;
        let admission = ledger.admit(clean_invocation)?;
        let receiver = match admission {
            Admission::Existing(receiver) => receiver,
            Admission::New { id, receiver } => {
                um.register_pending(process_id, name, id);
                permit.send(json!({
                    "id": id,
                    "method": "process/start",
                    "params": native_params,
                }));
                receiver
            }
        };
        Ok(Ok(receiver))
    }

    async fn handle_stop_user_mcp(
        &self,
        invocation: &wire::Invocation,
    ) -> Result<Response, Refusal> {
        if self.read_only {
            return Ok(Response::Operation {
                outcome: Outcome::Completed {
                    reply: user_mcp::error_reply(
                        "user_mcp_unsupported",
                        "user MCP servers are not supported in read-only environments",
                    ),
                },
            });
        }
        let args = match parse_stop_user_mcp_args(&invocation.params) {
            Ok(parsed) => parsed,
            Err(reply) => {
                return Ok(Response::Operation {
                    outcome: Outcome::Completed { reply },
                });
            }
        };
        let targets = self.select_user_mcp_stop_targets(args);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut stopped = 0_u32;
        let mut failed = 0_u32;
        for target in targets {
            if self
                .stop_one_user_mcp_target(&target.pid, target.pending_start, target.ended, deadline)
                .await
            {
                stopped += 1;
            } else {
                failed += 1;
            }
        }
        Ok(Response::Operation {
            outcome: Outcome::Completed {
                reply: json!({"result": {"stopped": stopped, "failed": failed}}),
            },
        })
    }

    fn select_user_mcp_stop_targets(&self, args: StopUserMcpArgs) -> Vec<StopUserMcpTarget> {
        let mut um = self.user_mcp.lock();
        if let Some(pid) = args.target_pid {
            um.record_tombstone(&pid);
            return um
                .active_processes
                .get(&pid)
                .map(|entry| {
                    vec![StopUserMcpTarget {
                        pid,
                        pending_start: pending_rx(&entry.start_state),
                        ended: entry.ended.subscribe(),
                    }]
                })
                .unwrap_or_default();
        }
        um.stop_epoch = um.stop_epoch.saturating_add(1);
        if let Some(rev) = args.config_revision {
            um.last_stop_config_revision = um.last_stop_config_revision.max(rev);
        }
        um.latest_allowlist = args.keep_allowlist.clone();
        let selected: Vec<StopUserMcpTarget> = um
            .active_processes
            .iter()
            .filter(|(_, entry)| {
                args.keep_allowlist.as_ref().is_none_or(|allowlist| {
                    !allowlist
                        .iter()
                        .any(|allowed| allowed == &entry.server_name)
                })
            })
            .map(|(pid, entry)| StopUserMcpTarget {
                pid: pid.clone(),
                pending_start: pending_rx(&entry.start_state),
                ended: entry.ended.subscribe(),
            })
            .collect();
        for target in &selected {
            um.record_tombstone(&target.pid);
        }
        selected
    }

    /// Rule E1 confirmation: wait for a `Pending` start reply before terminating;
    /// accept `unknown process id` ONLY after a confirmed start; confirm real
    /// process exit (`running == false` or `process/exited`/`process/closed`)
    /// before removing the entry or reporting clean.
    async fn stop_one_user_mcp_target(
        &self,
        pid: &str,
        maybe_pending: Option<watch::Receiver<Option<bool>>>,
        mut ended_rx: watch::Receiver<bool>,
        deadline: tokio::time::Instant,
    ) -> bool {
        if let Some(mut rx) = maybe_pending {
            let outcome = tokio::time::timeout_at(deadline, async {
                loop {
                    if let Some(started) = *rx.borrow_and_update() {
                        return Some(started);
                    }
                    if rx.changed().await.is_err() {
                        return *rx.borrow_and_update();
                    }
                }
            })
            .await;
            match outcome {
                Ok(Some(false)) => {
                    self.user_mcp.lock().remove_confirmed_or_failed(pid);
                    return true;
                }
                Ok(Some(true)) => {}
                _ => return false,
            }
        }
        if *ended_rx.borrow_and_update() {
            self.user_mcp.lock().remove_confirmed_or_failed(pid);
            return true;
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return false;
        }
        let op_id = self.user_mcp.lock().next_stop_operation_id();
        let term_invocation = wire::Invocation {
            operation_id: op_id,
            method: "process/terminate".into(),
            params: json!({"processId": pid}),
        };
        let Ok(Admission::Existing(receiver) | Admission::New { receiver, .. }) =
            self.admit(&term_invocation, &term_invocation.params)
        else {
            return false;
        };
        let wait_ms = u64::try_from(remaining.as_millis())
            .unwrap_or(5_000)
            .min(wire::MAX_WAIT_MS);
        let Outcome::Completed { reply } = wait_outcome(receiver, wait_ms).await else {
            return false;
        };
        let unknown_msg = format!("unknown process id {pid}");
        let immediately_gone = reply.pointer("/result/running") == Some(&Value::Bool(false))
            || reply.pointer("/error/message").and_then(Value::as_str)
                == Some(unknown_msg.as_str());
        if immediately_gone {
            self.user_mcp.lock().remove_confirmed_or_failed(pid);
            return true;
        }
        if reply.get("result").is_none() {
            return false;
        }
        // `process/terminate` returned `result` (e.g. `{"running": true}`):
        // wait within the remaining budget for `process/exited` / `process/closed`
        // (via `on_process_ended`) before marking clean.
        let exited = tokio::time::timeout_at(deadline, async {
            loop {
                if *ended_rx.borrow_and_update() {
                    return true;
                }
                if ended_rx.changed().await.is_err() {
                    return *ended_rx.borrow_and_update();
                }
            }
        })
        .await
        .unwrap_or(false);
        if exited {
            self.user_mcp.lock().remove_confirmed_or_failed(pid);
        }
        exited
    }

    /// Reserve before admitting so a full local queue proves non-admission.
    /// Once admitted, the writer owns the call.
    fn admit(&self, invocation: &wire::Invocation, native: &Value) -> Result<Admission, Refusal> {
        let mut ledger = self.ledger.lock();
        if ledger.observe(&invocation.operation_id).is_some() || ledger.lost {
            return ledger.admit(invocation);
        }
        let permit = self.writer.try_reserve().map_err(|_| Refusal::Capacity)?;
        let result = ledger.admit(invocation);
        if let Ok(Admission::New { id, .. }) = &result {
            // The ledger retains the admitted params; only the native frame
            // carries context.
            let params =
                wire::with_call_context(&invocation.method, native, self.call_context.as_deref());
            permit.send(json!({"id": id, "method": invocation.method, "params": params}));
        }
        result
    }

    /// Terminate the native processes of a worker that is gone. Each request
    /// is an ordinary admitted operation, so its reply keeps the ledger whole.
    fn reap_unattended(&self, sequence: &mut u64) -> usize {
        if self.attended.lock().elapsed() < UNATTENDED_PROCESS_REAP || self.ledger.lock().lost {
            return 0;
        }
        let processes = self.ledger.lock().take_processes();
        let mut reaped = 0;
        for process in processes {
            *sequence += 1;
            let invocation = wire::Invocation {
                operation_id: format!("cowboy-unattended-reap-{sequence}"),
                method: "process/terminate".into(),
                params: json!({"processId": process}),
            };
            if self.admit(&invocation, &invocation.params).is_ok() {
                reaped += 1;
            }
        }
        reaped
    }

    async fn events(&self, after: u64, wait_ms: u64) -> Result<Response, Refusal> {
        let mut changed = self.ledger.lock().event_changed.subscribe();
        let (events, through) = self.ledger.lock().events(after)?;
        if events.is_empty() && self.ledger.lock().lost {
            return Err(Refusal::EnvironmentLost);
        }
        if events.is_empty() && wait_ms > 0 && !self.ledger.lock().lost {
            let _ = tokio::time::timeout(Duration::from_millis(wait_ms), changed.changed()).await;
            let (events, through) = self.ledger.lock().events(after)?;
            return Ok(Response::Events { events, through });
        }
        Ok(Response::Events { events, through })
    }
}

async fn wait_outcome(mut receiver: watch::Receiver<Outcome>, wait_ms: u64) -> Outcome {
    if *receiver.borrow() == Outcome::Pending && wait_ms > 0 {
        let _ = tokio::time::timeout(Duration::from_millis(wait_ms), receiver.changed()).await;
    }
    receiver.borrow().clone()
}

async fn connection(host: Arc<Host>, stream: UnixStream) -> Result<()> {
    let (read, mut write) = stream.into_split();
    let mut read = BufReader::new(read);
    // A connection may issue successive bounded requests; waiting and polling
    // is transport work and never enters the model conversation.
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(30), read_json(&mut read)).await??;
        let request: Request =
            serde_json::from_value(frame).context("invalid execution request")?;
        let response = host.request(request).await;
        tokio::time::timeout(Duration::from_secs(10), write_json(&mut write, &response)).await??;
        if response == Response::Closed {
            host.close.notify_one();
            return Ok(());
        }
    }
}

/// Start exactly one target executor. The Machine must supply an owned pinned
/// component; using another Plugin's installed private CLI is not a launch path.
fn pending_rx(state: &user_mcp::StartState) -> Option<watch::Receiver<Option<bool>>> {
    match state {
        user_mcp::StartState::Pending(tx) => Some(tx.subscribe()),
        user_mcp::StartState::Confirmed => None,
    }
}

fn parse_start_user_mcp_args(params: &Value) -> Result<(String, String, String), Value> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .filter(|name| user_mcp::valid_server_name(name))
        .map(str::to_owned)
        .ok_or_else(|| user_mcp::error_reply("user_mcp_invalid", "invalid user MCP server name"))?;
    let process_id = params
        .get("processId")
        .and_then(Value::as_str)
        .filter(|id| user_mcp::valid_user_mcp_process_id(id))
        .map(str::to_owned)
        .ok_or_else(|| user_mcp::error_reply("user_mcp_invalid", "invalid user MCP processId"))?;
    let fingerprint = params
        .get("fingerprint")
        .and_then(Value::as_str)
        .filter(|fp| fp.len() == 64 && fp.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
        .map(str::to_owned)
        .ok_or_else(|| user_mcp::error_reply("user_mcp_invalid", "invalid user MCP fingerprint"))?;
    Ok((name, process_id, fingerprint))
}

struct StopUserMcpArgs {
    target_pid: Option<String>,
    keep_allowlist: Option<Vec<String>>,
    config_revision: Option<u64>,
}

struct StopUserMcpTarget {
    pid: String,
    pending_start: Option<watch::Receiver<Option<bool>>>,
    ended: watch::Receiver<bool>,
}

fn parse_stop_user_mcp_args(params: &Value) -> Result<StopUserMcpArgs, Value> {
    let target_pid = match params.get("processId") {
        None | Some(Value::Null) => None,
        Some(Value::String(id)) if user_mcp::valid_user_mcp_process_id(id) => Some(id.clone()),
        _ => {
            return Err(user_mcp::error_reply(
                "user_mcp_invalid",
                "invalid processId in stopUserMcp",
            ));
        }
    };
    let keep_allowlist = match params.get("keepAllowlist") {
        None | Some(Value::Null) => None,
        Some(Value::Array(items)) if items.len() <= 32 => {
            let mut list = Vec::with_capacity(items.len());
            for item in items {
                let Some(name) = item
                    .as_str()
                    .filter(|s| crate::config::valid_mcp_server_name(s))
                else {
                    return Err(user_mcp::error_reply(
                        "user_mcp_invalid",
                        "invalid server name in keepAllowlist",
                    ));
                };
                list.push(name.to_owned());
            }
            Some(list)
        }
        _ => {
            return Err(user_mcp::error_reply(
                "user_mcp_invalid",
                "invalid keepAllowlist in stopUserMcp",
            ));
        }
    };
    let config_revision = match params.get("configRevision") {
        None | Some(Value::Null) => None,
        Some(value) => match value.as_u64() {
            Some(rev) => Some(rev),
            None => {
                return Err(user_mcp::error_reply(
                    "user_mcp_invalid",
                    "invalid configRevision in stopUserMcp",
                ));
            }
        },
    };
    Ok(StopUserMcpArgs {
        target_pid,
        keep_allowlist,
        config_revision,
    })
}

fn handle_native_message(
    backend_host: &Host,
    mut message: Value,
    pending_event: &mut Option<Value>,
) -> Result<(), ()> {
    let mut reply_notify: Option<(u64, bool)> = None;
    let mut ended_process: Option<String> = None;
    let result = {
        let mut ledger = backend_host.ledger.lock();
        if let Some(id) = message.get("id").and_then(Value::as_u64) {
            if message.get("method").is_some()
                || message.get("result").is_some() == message.get("error").is_some()
            {
                return Err(());
            }
            message
                .as_object_mut()
                .expect("response object")
                .remove("id");
            let succeeded = message.get("result").is_some();
            if serde_json::to_vec(&message)
                .map_or(true, |bytes| bytes.len() > wire::MAX_FRAME_BYTES - 4096)
            {
                message = json!({"error":{"code":-32000,"message":"Execution finished, but its reply exceeds the retained transport limit; do not repeat a write or command. Use a bounded read to inspect the result."}});
            }
            let res = ledger.finish(id, message);
            if res.is_ok() {
                reply_notify = Some((id, succeeded));
            }
            res
        } else if let Some(method) = message.get("method").and_then(Value::as_str) {
            if matches!(method, "process/exited" | "process/closed")
                && let Some(pid) = message.pointer("/params/processId").and_then(Value::as_str)
                && pid.starts_with(user_mcp::USER_MCP_PROCESS_PREFIX)
            {
                ended_process = Some(pid.to_owned());
            }
            *pending_event = Some(message);
            Ok(())
        } else {
            Err(())
        }
    };
    // Strictly AFTER dropping `ledger` lock (user_mcp -> ledger lock order).
    if let Some((id, succeeded)) = reply_notify {
        backend_host.user_mcp.lock().on_native_reply(id, succeeded);
    }
    if let Some(pid) = ended_process {
        backend_host.user_mcp.lock().on_process_ended(&pid);
    }
    result
}

fn annotate_execution_capabilities(
    initialization: &mut Value,
    contract: &LaunchContract,
) -> Result<Option<PathBuf>> {
    let obj = initialization
        .as_object_mut()
        .context("executor initialization is not an object")?;
    if let Some(managed) = &contract.managed {
        obj.insert(
            wire::MANAGED_INITIALIZATION_KEY.to_owned(),
            json!({"schema": 1, "profile": managed.profile, "roundPath": managed.round_path}),
        );
        return Ok(None);
    }
    match user_mcp::create_neutral_cwd(&contract.binding.environment.incarnation) {
        Ok(cwd) => {
            obj.insert(
                user_mcp::USER_MCP_INITIALIZATION_KEY.to_owned(),
                json!({"schema": 1}),
            );
            Ok(Some(cwd))
        }
        Err(_) => {
            tracing::warn!(
                reason = "neutral_cwd_unavailable",
                "user MCP neutral cwd unavailable; capability disabled for this keeper"
            );
            Ok(None)
        }
    }
}

pub async fn run(args: Args) -> Result<()> {
    let contract_metadata = std::fs::symlink_metadata(&args.contract)?;
    ensure!(
        contract_metadata.is_file() && contract_metadata.mode() & 0o077 == 0,
        "execution contract must be a private regular file"
    );
    ensure!(
        contract_metadata.len() <= 64 * 1024,
        "execution contract exceeds limit"
    );
    let contract: LaunchContract = serde_json::from_slice(&std::fs::read(&args.contract)?)?;
    validate_contract(&contract)?;
    tracing::info!(event_name = "cowboy.execution.starting", session = %contract.session_id, environment_id = %contract.binding.environment.id, incarnation = %contract.binding.environment.incarnation, "starting owned execution environment");
    ensure!(
        args.state_dir.is_absolute(),
        "execution state path must be absolute"
    );
    let metadata = std::fs::symlink_metadata(&args.state_dir)?;
    ensure!(
        metadata.is_dir()
            && metadata.mode() & 0o077 == 0
            && metadata.uid() == contract_metadata.uid(),
        "execution state directory must be private"
    );
    identity::verify(&contract.executor).await?;
    if let Some(cli) = &contract.agent_cli {
        identity::verify(cli).await?;
    }
    let cwd = PathBuf::from(&contract.binding.workspace.cwd);
    let physical_cwd = std::fs::canonicalize(&cwd).context("resolving execution workspace")?;
    let metadata = std::fs::metadata(&physical_cwd)?;
    ensure!(metadata.is_dir(), "execution workspace unavailable");
    // An existing marker is never treated as a stale PID/socket to remove.
    // Lost keepers require explicit new incarnation admission by the Machine.
    let marker = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(args.state_dir.join("started.json"))
        .context("execution incarnation has already started")?;
    serde_json::to_writer(&marker, &Scope::from_binding(&contract.binding))?;
    marker.sync_all()?;
    std::fs::File::open(&args.state_dir)?.sync_all()?;
    let socket = args
        .socket
        .unwrap_or_else(|| args.state_dir.join("keeper.sock"));
    ensure!(
        socket.is_absolute()
            && socket.file_name().is_some_and(|name| name == "keeper.sock")
            && socket
                .parent()
                .is_some_and(|parent| std::fs::canonicalize(parent).ok()
                    == std::fs::canonicalize(&args.state_dir).ok()),
        "control socket must belong to the private execution directory"
    );
    let listener = UnixListener::bind(&socket)?;
    std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
    let private_home = args.state_dir.join("executor-home");
    std::fs::create_dir(&private_home)?;
    std::fs::set_permissions(&private_home, std::fs::Permissions::from_mode(0o700))?;
    let mut child = ProcessCommand::new(&contract.executor.command)
        .args(["exec-server", "--listen", "stdio"])
        .current_dir(&physical_cwd)
        .env_clear()
        .envs(&contract.environment)
        .env("CODEX_HOME", &private_home)
        .env("COWBOY_EXECUTION_FILE_HELPER", std::env::current_exe()?)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .context("starting owned executor")?;
    let mut input = child.stdin.take().context("executor input unavailable")?;
    let mut native_errors = child
        .stderr
        .take()
        .context("executor diagnostics unavailable")?;
    let stderr_reader = tokio::spawn(async move {
        // Native stderr is unstructured and may contain command data. Drain it
        // without blocking the executor, retaining only bounded metadata.
        let mut buffer = [0_u8; 4096];
        let mut bytes = 0_u64;
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                result = native_errors.read(&mut buffer) => match result {
                    Ok(0) | Err(_) => break,
                    Ok(count) => { bytes = bytes.saturating_add(count as u64); }
                },
                _ = interval.tick(), if bytes > 0 => {
                    tracing::warn!(event_name = "cowboy.execution.native_stderr", reason = "native_diagnostic_output", bytes, "native executor wrote diagnostics; content excluded");
                    bytes = 0;
                }
            }
        }
        if bytes > 0 {
            tracing::warn!(
                event_name = "cowboy.execution.native_stderr",
                reason = "native_diagnostic_output",
                bytes,
                "native executor wrote diagnostics before exit"
            );
        }
    });
    let mut output = BufReader::new(child.stdout.take().context("executor output unavailable")?);
    let initialization = tokio::time::timeout(Duration::from_secs(15), async {
        write_json(&mut input, &json!({"id": 0, "method": "initialize", "params": {"clientName": "cowboy-machine-executor"}})).await?;
        let reply = read_json(&mut output).await?;
        ensure!(reply.get("id") == Some(&json!(0)) && reply.get("error").is_none(), "executor initialization refused");
        let mut result = reply.get("result").context("executor initialization missing")?.clone();
        ensure!(result.pointer("/environmentInfo/executorVersion").and_then(Value::as_str) == Some(contract.executor.version.as_str()), "executor protocol version differs");
        let actual_cwd = result.pointer("/environmentInfo/cwd").and_then(Value::as_str).and_then(|path| url::Url::parse(path).ok()).and_then(|path| path.to_file_path().ok());
        ensure!(actual_cwd.as_ref() == Some(&physical_cwd), "executor working directory differs");
        // A storage move may retain the admitted path as a directory symlink.
        // Verify the native physical cwd, then keep the binding's logical path
        // on the wire for existing Provider identities. Host::current_workspace
        // still fences changes to the directory reached through that alias.
        *result.pointer_mut("/environmentInfo/cwd").context("executor cwd missing")? =
            json!(url::Url::from_file_path(&cwd).map_err(|()| anyhow::anyhow!("invalid execution workspace URL"))?.as_str());
        write_json(&mut input, &json!({"method": "initialized", "params": {}})).await?;
        Ok::<_, anyhow::Error>(result)
    }).await??;
    let mut initialization = initialization;
    identity::annotate(&mut initialization, &contract)?;
    let neutral_user_mcp_cwd = annotate_execution_capabilities(&mut initialization, &contract)?;
    let (writer, mut requests) = mpsc::channel(64);
    tracing::info!(
        event_name = "cowboy.execution.ready",
        "native executor initialized"
    );
    let (stop, mut stopped) = tokio::sync::oneshot::channel();
    let (finished, finished_rx) = watch::channel(false);
    let mut user_mcp_key = [0_u8; 32];
    rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut user_mcp_key);
    let host = Arc::new(Host {
        scope: Scope::from_binding(&contract.binding),
        call_context: contract.call_context.clone(),
        read_only: contract.managed.is_some(),
        capability_digest: Sha256::digest(contract.capability.as_bytes()).into(),
        initialization,
        cwd,
        source_path: PathBuf::from(&contract.binding.workspace.source_path),
        state_dir: args.state_dir.clone(),
        environment: contract.environment.clone(),
        neutral_user_mcp_cwd,
        user_mcp_key,
        user_mcp: Mutex::new(user_mcp::UserMcpState::default()),
        inode: (metadata.dev(), metadata.ino()),
        ledger: Mutex::new(Ledger::new()),
        writer,
        stop: Mutex::new(Some(stop)),
        stopped: finished_rx,
        close: tokio::sync::Notify::new(),
        attended: Mutex::new(std::time::Instant::now()),
    });
    let reaper_host = Arc::clone(&host);
    let reaper = tokio::spawn(async move {
        let mut sequence = 0;
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            let reaped = reaper_host.reap_unattended(&mut sequence);
            if reaped > 0 {
                tracing::info!(
                    event_name = "cowboy.execution.unattended_reaped",
                    processes = reaped,
                    "terminated native processes of a worker that is gone"
                );
            }
        }
    });
    let (messages, mut native_messages) = mpsc::channel(4);
    let reader = tokio::spawn(async move {
        loop {
            let message = read_json_bounded(&mut output, 32 * 1024 * 1024).await;
            let ended = message.is_err();
            if messages.send(message).await.is_err() || ended {
                break;
            }
        }
    });
    let backend_host = Arc::clone(&host);
    let backend = tokio::spawn(async move {
        // Poll both directions independently. A full executor output pipe must
        // not prevent us from draining it while a large input frame is being
        // written. Bounded admission owns the queued effects; backpressure is
        // not evidence that the executor died and grants no replay authority.
        let mut native_writer = Box::pin(async {
            while let Some(request) = requests.recv().await {
                write_json(&mut input, &request).await?;
            }
            Ok::<_, anyhow::Error>(())
        });
        let mut event_space = backend_host.ledger.lock().event_space.subscribe();
        let mut pending_event = None;
        let mut pressure_started: Option<std::time::Instant> = None;
        loop {
            if let Some(message) = pending_event.take() {
                match backend_host.ledger.lock().push_event(message) {
                    Ok(()) => {
                        if let Some(started) = pressure_started.take() {
                            tracing::info!(
                                event_name = "cowboy.execution.backpressure_cleared",
                                duration_ms = started.elapsed().as_secs_f64() * 1000.0,
                                "execution event consumer caught up"
                            );
                        }
                    }
                    Err(EventError::Full(message)) => {
                        if pressure_started.is_none() {
                            pressure_started = Some(std::time::Instant::now());
                            tracing::warn!(
                                event_name = "cowboy.execution.backpressure",
                                reason = "unacknowledged_event_capacity",
                                "execution event consumer is behind"
                            );
                        }
                        pending_event = Some(message);
                    }
                    Err(EventError::Invalid) => {
                        tracing::warn!(
                            event_name = "cowboy.execution.backend_stopped",
                            reason = "native_event_limit",
                            "execution backend stopped"
                        );
                        break;
                    }
                }
            }
            tokio::select! {
                _ = &mut native_writer => {
                    tracing::warn!(event_name = "cowboy.execution.backend_stopped", reason = "native_input_closed", "execution backend stopped");
                    break;
                }
                message = native_messages.recv(), if pending_event.is_none() => {
                    let Some(Ok(message)) = message else {
                        tracing::warn!(event_name = "cowboy.execution.backend_stopped", reason = "native_stream_closed", "execution backend stopped");
                        break;
                    };
                    if handle_native_message(&backend_host, message, &mut pending_event).is_err() {
                        break;
                    }
                }
                _ = event_space.changed(), if pending_event.is_some() => {},
                status = child.wait() => {
                    tracing::warn!(event_name = "cowboy.execution.native_exited", exit_code = status.ok().and_then(|s| s.code()).unwrap_or(-1), "native executor exited");
                    break;
                },
                _ = &mut stopped => {
                    tracing::info!(event_name = "cowboy.execution.shutdown_requested", "execution shutdown requested");
                    break;
                },
            }
        }
        backend_host.ledger.lock().lose();
        backend_host.user_mcp.lock().on_backend_lost();
        // EOF gives the executor a chance to reap its owned process groups.
        // The Machine service additionally owns the complete cgroup.
        drop(native_writer);
        drop(input);
        if tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .is_err()
        {
            let _ = child.kill().await;
        }
        reader.abort();
        let _ = reader.await;
        stderr_reader.abort();
        let _ = stderr_reader.await;
        let _ = finished.send(true);
    });
    let connections = Arc::new(Semaphore::new(32));
    let mut shutdown = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    loop {
        tokio::select! {
            _ = shutdown.recv() => break,
            _ = host.close.notified() => break,
            _ = tokio::signal::ctrl_c() => break,
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                let Ok(permit) = Arc::clone(&connections).try_acquire_owned() else { drop(stream); continue; };
                let host = Arc::clone(&host);
                tokio::spawn(async move {
                    let _permit = permit;
                    let _ = connection(host, stream).await;
                });
            }
        }
    }
    reaper.abort();
    host.ledger.lock().lose();
    if let Some(stop) = host.stop.lock().take() {
        let _ = stop.send(());
    }
    let _ = backend.await;
    if let Some(cwd) = &host.neutral_user_mcp_cwd {
        user_mcp::remove_neutral_cwd(cwd);
    }
    tracing::info!(
        event_name = "cowboy.execution.stopped",
        "execution environment stopped"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(attended: std::time::Instant) -> (Host, mpsc::Receiver<Value>) {
        let (writer, frames) = mpsc::channel(8);
        let (stop, _) = tokio::sync::oneshot::channel();
        let scope = Scope {
            binding_id: "binding".into(),
            revision: 1,
            incarnation: "incarnation".into(),
        };
        let host = Host {
            scope,
            call_context: None,
            read_only: false,
            capability_digest: [0; 32],
            initialization: json!({}),
            cwd: PathBuf::from("/"),
            source_path: PathBuf::from("/"),
            state_dir: PathBuf::from("/tmp"),
            environment: std::collections::BTreeMap::new(),
            neutral_user_mcp_cwd: None,
            user_mcp_key: [0x11; 32],
            user_mcp: Mutex::new(user_mcp::UserMcpState::default()),
            inode: (0, 0),
            ledger: Mutex::new(Ledger::new()),
            writer,
            stop: Mutex::new(Some(stop)),
            stopped: watch::channel(false).1,
            close: tokio::sync::Notify::new(),
            attended: Mutex::new(attended),
        };
        (host, frames)
    }

    fn start(host: &Host, process: &str) {
        let invocation = wire::Invocation {
            operation_id: format!("start-{process}"),
            method: "process/start".into(),
            params: json!({"processId": process, "argv": ["true"]}),
        };
        assert!(matches!(
            host.admit(&invocation, &invocation.params),
            Ok(Admission::New { .. })
        ));
    }

    #[test]
    fn an_unattended_worker_has_its_processes_terminated_once() {
        let gone = std::time::Instant::now()
            .checked_sub(UNATTENDED_PROCESS_REAP + Duration::from_secs(1))
            .unwrap();
        let (host, mut frames) = host(gone);
        start(&host, "mcp");
        start(&host, "closed");
        host.ledger
            .lock()
            .push_event(json!({"method": "process/closed", "params": {"processId": "closed"}}))
            .unwrap();
        let mut sequence = 0;
        assert_eq!(host.reap_unattended(&mut sequence), 1);
        let mut methods = Vec::new();
        while let Ok(frame) = frames.try_recv() {
            methods.push((
                frame["method"].clone(),
                frame["params"]["processId"].clone(),
            ));
        }
        assert_eq!(
            methods.last().unwrap(),
            &(json!("process/terminate"), json!("mcp"))
        );
        assert_eq!(methods.len(), 3);
        assert_eq!(host.reap_unattended(&mut sequence), 0);
    }

    #[test]
    fn an_attended_worker_keeps_its_processes() {
        let (host, mut frames) = host(std::time::Instant::now());
        start(&host, "mcp");
        let mut sequence = 0;
        assert_eq!(host.reap_unattended(&mut sequence), 0);
        assert_eq!(frames.try_recv().unwrap()["method"], "process/start");
        assert!(frames.try_recv().is_err());
    }

    fn test_host_with_user_mcp(
        dir: &Path,
        home: &Path,
        bin: &Path,
        source: &Path,
        worktree: &Path,
        neutral_cwd: PathBuf,
    ) -> (Host, mpsc::Receiver<Value>) {
        let (writer, frames) = mpsc::channel(16);
        let (stop, _) = tokio::sync::oneshot::channel();
        let meta = std::fs::metadata(worktree).unwrap();
        let mut environment = std::collections::BTreeMap::new();
        environment.insert("HOME".into(), home.to_string_lossy().into_owned());
        environment.insert("PATH".into(), bin.to_string_lossy().into_owned());
        let host = Host {
            scope: Scope {
                binding_id: "binding".into(),
                revision: 1,
                incarnation: "incarnation".into(),
            },
            call_context: Some("/tmp/secret-call-context.json".into()),
            read_only: false,
            capability_digest: Sha256::digest(b"cap").into(),
            initialization: json!({ user_mcp::USER_MCP_INITIALIZATION_KEY: {"schema": 1} }),
            cwd: worktree.to_path_buf(),
            source_path: source.to_path_buf(),
            state_dir: dir.join("keeper-state"),
            environment,
            neutral_user_mcp_cwd: Some(neutral_cwd),
            user_mcp_key: [0x22; 32],
            user_mcp: Mutex::new(user_mcp::UserMcpState::default()),
            inode: (meta.dev(), meta.ino()),
            ledger: Mutex::new(Ledger::new()),
            writer,
            stop: Mutex::new(Some(stop)),
            stopped: watch::channel(false).1,
            close: tokio::sync::Notify::new(),
            attended: Mutex::new(std::time::Instant::now()),
        };
        (host, frames)
    }

    fn req(host: &Host, command: Command) -> Request {
        Request {
            schema: wire::SCHEMA,
            scope: host.scope.clone(),
            capability: "cap".into(),
            command,
        }
    }

    #[tokio::test]
    async fn user_mcp_requires_manager_gate_and_start_observes_before_config_drift() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let bin = dir.path().join("bin");
        let source = dir.path().join("source");
        let worktree = dir.path().join("worktree");
        let neutral = dir.path().join("neutral");
        for d in [&home, &bin, &source, &worktree, &neutral] {
            std::fs::create_dir_all(d).unwrap();
        }
        let tool = bin.join("mcp-bin");
        std::fs::write(&tool, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(
            home.join(".claude.json"),
            json!({"mcpServers": {"demo": {"command": "mcp-bin", "args": ["--v1"]}}}).to_string(),
        )
        .unwrap();

        let (host, mut frames) =
            test_host_with_user_mcp(dir.path(), &home, &bin, &source, &worktree, neutral);

        // Missing managerGate fails closed (old Machine rollback protection).
        let no_gate = host
            .request(req(
                &host,
                Command::Invoke {
                    invocation: wire::Invocation {
                        operation_id: "op-no-gate".into(),
                        method: user_mcp::USER_MCP_LIST_METHOD.into(),
                        params: json!({}),
                    },
                    wait_ms: 0,
                },
            ))
            .await;
        let Response::Operation {
            outcome: Outcome::Completed { reply },
        } = no_gate
        else {
            panic!("expected Completed error");
        };
        assert_eq!(
            reply.pointer("/error/data/code").and_then(Value::as_str),
            Some("user_mcp_manager_gate_required")
        );

        // List with managerGate succeeds and returns only name/scope/fingerprint.
        let listed = host
            .request(req(
                &host,
                Command::Invoke {
                    invocation: wire::Invocation {
                        operation_id: "op-list".into(),
                        method: user_mcp::USER_MCP_LIST_METHOD.into(),
                        params: json!({"managerGate": {"enabled": true, "allowlist": []}}),
                    },
                    wait_ms: 0,
                },
            ))
            .await;
        let Response::Operation {
            outcome: Outcome::Completed { reply },
        } = listed
        else {
            panic!("expected list Completed");
        };
        let fp = reply["result"]["servers"][0]["fingerprint"]
            .as_str()
            .unwrap()
            .to_owned();
        let pid = format!(
            "{}0123456789abcdef0123456789abcdef",
            user_mcp::USER_MCP_PROCESS_PREFIX
        );

        // Start demo server.
        let start_resp = host
            .request(req(
                &host,
                Command::Invoke {
                    invocation: wire::Invocation {
                        operation_id: "op-start-1".into(),
                        method: user_mcp::USER_MCP_START_METHOD.into(),
                        params: json!({
                            "name": "demo",
                            "processId": pid,
                            "fingerprint": fp,
                            "managerGate": {"enabled": true, "allowlist": []}
                        }),
                    },
                    wait_ms: 0,
                },
            ))
            .await;
        assert!(matches!(
            start_resp,
            Response::Operation {
                outcome: Outcome::Pending
            }
        ));
        let frame = frames.try_recv().unwrap();
        assert_eq!(frame["method"], "process/start");
        assert_eq!(frame["params"]["envPolicy"]["inherit"], "none");
        assert!(
            frame["params"]["envPolicy"]["set"]
                .get("COWBOY_CALL_CONTEXT")
                .is_none()
        );
        let native_id = frame["id"].as_u64().unwrap();
        // Complete native start.
        host.ledger
            .lock()
            .finish(native_id, json!({"result": {"processId": pid}}))
            .unwrap();
        host.user_mcp.lock().on_native_reply(native_id, true);

        // Mutate ~/.claude.json (config drift) AND change allowlist in managerGate:
        // retrying the SAME operation_id MUST replay cached Completed result!
        std::fs::write(
            home.join(".claude.json"),
            json!({"mcpServers": {"demo": {"command": "mcp-bin", "args": ["--v2"]}}}).to_string(),
        )
        .unwrap();
        let retry = host
            .request(req(
                &host,
                Command::Invoke {
                    invocation: wire::Invocation {
                        operation_id: "op-start-1".into(),
                        method: user_mcp::USER_MCP_START_METHOD.into(),
                        params: json!({
                            "name": "demo",
                            "processId": pid,
                            "fingerprint": fp,
                            "managerGate": {"enabled": true, "allowlist": ["other"]}
                        }),
                    },
                    wait_ms: 0,
                },
            ))
            .await;
        let Response::Operation {
            outcome: Outcome::Completed { reply },
        } = retry
        else {
            panic!("expected idempotent Completed replay, got {retry:?}");
        };
        assert_eq!(reply["result"]["processId"], pid);

        // Verify keeper ledger tracked processId for unattended reaping.
        let tracked = host.ledger.lock().take_processes();
        assert!(tracked.contains(&pid));
    }

    #[tokio::test]
    async fn stop_user_mcp_respects_e1_pending_start_and_fences_tombstoned_ids() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let bin = dir.path().join("bin");
        let source = dir.path().join("source");
        let worktree = dir.path().join("worktree");
        let neutral = dir.path().join("neutral");
        for d in [&home, &bin, &source, &worktree, &neutral] {
            std::fs::create_dir_all(d).unwrap();
        }
        let tool = bin.join("mcp-bin");
        std::fs::write(&tool, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(
            home.join(".claude.json"),
            json!({"mcpServers": {"demo": {"command": "mcp-bin"}}}).to_string(),
        )
        .unwrap();

        let (host, mut frames) =
            test_host_with_user_mcp(dir.path(), &home, &bin, &source, &worktree, neutral);
        let host = Arc::new(host);

        // Stop an unseen processId first -> tombstones it so later startUserMcp is rejected.
        let pid_tomb = format!(
            "{}aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            user_mcp::USER_MCP_PROCESS_PREFIX
        );
        let stop_unseen = host
            .request(req(
                &host,
                Command::Invoke {
                    invocation: wire::Invocation {
                        operation_id: "op-stop-unseen".into(),
                        method: user_mcp::USER_MCP_STOP_METHOD.into(),
                        params: json!({"processId": pid_tomb}),
                    },
                    wait_ms: 0,
                },
            ))
            .await;
        let Response::Operation {
            outcome: Outcome::Completed { reply },
        } = stop_unseen
        else {
            panic!("expected stop Completed");
        };
        assert_eq!(reply["result"]["stopped"], 0);
        assert_eq!(reply["result"]["failed"], 0);

        let listed = host
            .request(req(
                &host,
                Command::Invoke {
                    invocation: wire::Invocation {
                        operation_id: "op-list-2".into(),
                        method: user_mcp::USER_MCP_LIST_METHOD.into(),
                        params: json!({"managerGate": {"enabled": true, "allowlist": []}}),
                    },
                    wait_ms: 0,
                },
            ))
            .await;
        let Response::Operation {
            outcome: Outcome::Completed { reply },
        } = listed
        else {
            panic!();
        };
        let fp = reply["result"]["servers"][0]["fingerprint"]
            .as_str()
            .unwrap()
            .to_owned();

        let late_start = host
            .request(req(
                &host,
                Command::Invoke {
                    invocation: wire::Invocation {
                        operation_id: "op-start-tomb".into(),
                        method: user_mcp::USER_MCP_START_METHOD.into(),
                        params: json!({
                            "name": "demo",
                            "processId": pid_tomb,
                            "fingerprint": fp,
                            "managerGate": {"enabled": true, "allowlist": []}
                        }),
                    },
                    wait_ms: 0,
                },
            ))
            .await;
        let Response::Operation {
            outcome: Outcome::Completed { reply },
        } = late_start
        else {
            panic!();
        };
        assert_eq!(
            reply.pointer("/error/data/code").and_then(Value::as_str),
            Some("user_mcp_stopped")
        );

        // Start a real pending process, then issue stopUserMcp concurrently:
        // stop MUST wait for start confirmation before sending process/terminate!
        let pid_live = format!(
            "{}bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            user_mcp::USER_MCP_PROCESS_PREFIX
        );
        let _ = host
            .request(req(
                &host,
                Command::Invoke {
                    invocation: wire::Invocation {
                        operation_id: "op-start-live".into(),
                        method: user_mcp::USER_MCP_START_METHOD.into(),
                        params: json!({
                            "name": "demo",
                            "processId": pid_live,
                            "fingerprint": fp,
                            "managerGate": {"enabled": true, "allowlist": []}
                        }),
                    },
                    wait_ms: 0,
                },
            ))
            .await;
        let start_frame = frames.try_recv().unwrap();
        let start_id = start_frame["id"].as_u64().unwrap();

        let host_stop = Arc::clone(&host);
        let stop_task = tokio::spawn(async move {
            host_stop
                .request(req(
                    &host_stop,
                    Command::Invoke {
                        invocation: wire::Invocation {
                            operation_id: "op-stop-all".into(),
                            method: user_mcp::USER_MCP_STOP_METHOD.into(),
                            params: json!({}),
                        },
                        wait_ms: 0,
                    },
                ))
                .await
        });

        // Give stop_task a tick: no process/terminate should be sent yet while start is Pending!
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            frames.try_recv().is_err(),
            "terminate must wait for start confirmation"
        );

        // Confirm start -> stop_task now sends process/terminate.
        host.ledger
            .lock()
            .finish(start_id, json!({"result": {"processId": pid_live}}))
            .unwrap();
        host.user_mcp.lock().on_native_reply(start_id, true);

        let term_frame = tokio::time::timeout(Duration::from_secs(2), frames.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(term_frame["method"], "process/terminate");
        assert_eq!(term_frame["params"]["processId"], pid_live);
        let term_id = term_frame["id"].as_u64().unwrap();
        // Now `unknown process id` IS accepted (E1: start was confirmed).
        host.ledger
            .lock()
            .finish(
                term_id,
                json!({"error": {"code": -32001, "message": format!("unknown process id {pid_live}")}}),
            )
            .unwrap();

        let stop_resp = stop_task.await.unwrap();
        let Response::Operation {
            outcome: Outcome::Completed { reply },
        } = stop_resp
        else {
            panic!();
        };
        assert_eq!(reply["result"]["stopped"], 1);
        assert_eq!(reply["result"]["failed"], 0);
        assert!(host.user_mcp.lock().active_processes.is_empty());
    }

    #[tokio::test]
    async fn duplicate_start_returns_existing_stop_waits_for_exit_and_revision_fences_stale_gate() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let bin = dir.path().join("bin");
        let source = dir.path().join("source");
        let worktree = dir.path().join("worktree");
        let neutral = dir.path().join("neutral");
        for d in [&home, &bin, &source, &worktree, &neutral] {
            std::fs::create_dir_all(d).unwrap();
        }
        let srv = bin.join("echo-mcp");
        std::fs::write(&srv, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&srv, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(
            home.join(".claude.json"),
            json!({ "mcpServers": { "demo": { "command": srv.to_string_lossy() } } }).to_string(),
        )
        .unwrap();

        let (host, mut frames) =
            test_host_with_user_mcp(dir.path(), &home, &bin, &source, &worktree, neutral);
        let host = Arc::new(host);

        let list = host
            .request(req(
                &host,
                Command::Invoke {
                    invocation: wire::Invocation {
                        operation_id: "op-list-dup".into(),
                        method: user_mcp::USER_MCP_LIST_METHOD.into(),
                        params: json!({"managerGate": {"enabled": true, "allowlist": [], "configRevision": 2}}),
                    },
                    wait_ms: 1000,
                },
            ))
            .await;
        let Response::Operation {
            outcome: Outcome::Completed { reply },
        } = list
        else {
            panic!();
        };
        let fp = reply["result"]["servers"][0]["fingerprint"]
            .as_str()
            .unwrap()
            .to_owned();
        let pid = format!(
            "{}aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            user_mcp::USER_MCP_PROCESS_PREFIX
        );
        let clean_inv = wire::Invocation {
            operation_id: "op-dup-start".into(),
            method: user_mcp::USER_MCP_START_METHOD.into(),
            params: json!({"name": "demo", "processId": pid, "fingerprint": fp}),
        };
        let native = json!({"processId": pid, "argv": [srv.to_string_lossy()]});
        // Finding 4: two calls reaching admit_user_mcp_start with same operation_id + processId
        // must both return Ok(Ok(receiver)), NOT Completed{user_mcp_capacity}.
        let rx1 = host
            .admit_user_mcp_start(
                &clean_inv,
                0,
                Some(2),
                "demo".into(),
                pid.clone(),
                native.clone(),
            )
            .unwrap()
            .expect("first admit");
        let rx2 = host
            .admit_user_mcp_start(&clean_inv, 0, Some(2), "demo".into(), pid.clone(), native)
            .unwrap()
            .expect("duplicate admit must return Existing receiver, not capacity error");
        let frame = frames.recv().await.unwrap();
        let start_id = frame["id"].as_u64().unwrap();
        host.ledger
            .lock()
            .finish(start_id, json!({"result": {"processId": pid}}))
            .unwrap();
        host.user_mcp.lock().on_native_reply(start_id, true);
        assert!(matches!(
            wait_outcome(rx1, 100).await,
            Outcome::Completed { .. }
        ));
        assert!(matches!(
            wait_outcome(rx2, 100).await,
            Outcome::Completed { .. }
        ));

        // Finding 9: stopUserMcp MUST NOT refresh `attended` and MUST work even if workspace changed.
        let old_attended = std::time::Instant::now() - Duration::from_secs(600);
        *host.attended.lock() = old_attended;
        std::fs::remove_dir(&worktree).unwrap();
        assert!(!host.current_workspace());

        // Finding 8a: `process/terminate` returning `{"running": true}` must wait for `on_process_ended`.
        let host_stop = Arc::clone(&host);
        let stop_task = tokio::spawn(async move {
            host_stop
                .request(req(
                    &host_stop,
                    Command::Invoke {
                        invocation: wire::Invocation {
                            operation_id: "op-stop-rev3".into(),
                            method: user_mcp::USER_MCP_STOP_METHOD.into(),
                            params: json!({"configRevision": 3}),
                        },
                        wait_ms: 5000,
                    },
                ))
                .await
        });
        let term = tokio::time::timeout(Duration::from_secs(2), frames.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(term["method"], "process/terminate");
        let term_id = term["id"].as_u64().unwrap();
        host.ledger
            .lock()
            .finish(term_id, json!({"result": {"running": true}}))
            .unwrap();
        tokio::time::sleep(Duration::from_millis(40)).await;
        assert!(
            !stop_task.is_finished(),
            "stop must not report clean before process exit is confirmed"
        );
        host.user_mcp.lock().on_process_ended(&pid);
        let stop_resp = stop_task.await.unwrap();
        let Response::Operation {
            outcome: Outcome::Completed { reply },
        } = stop_resp
        else {
            panic!("stopUserMcp must succeed even after WorkspaceChanged");
        };
        assert_eq!(reply["result"]["stopped"], 1);
        assert_eq!(reply["result"]["failed"], 0);
        assert_eq!(
            *host.attended.lock(),
            old_attended,
            "stopUserMcp must not refresh attended timestamp"
        );

        // Finding 8b: stale managerGate with configRevision < 3 is fenced by last_stop_config_revision.
        let stale_pid = format!(
            "{}bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            user_mcp::USER_MCP_PROCESS_PREFIX
        );
        let stale_inv = wire::Invocation {
            operation_id: "op-stale-rev2".into(),
            method: user_mcp::USER_MCP_START_METHOD.into(),
            params: json!({"name": "demo", "processId": stale_pid, "fingerprint": fp}),
        };
        let current_epoch = host.user_mcp.lock().stop_epoch;
        let fenced = host
            .admit_user_mcp_start(
                &stale_inv,
                current_epoch,
                Some(2),
                "demo".into(),
                stale_pid,
                json!({}),
            )
            .unwrap();
        let Err(err_reply) = fenced else {
            panic!("stale configRevision gate must be fenced");
        };
        assert_eq!(
            err_reply
                .pointer("/error/data/code")
                .and_then(Value::as_str),
            Some("user_mcp_stopped")
        );
    }
}
