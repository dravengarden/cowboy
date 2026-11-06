//! Running declared project checks (`check@1`) in a session's execution
//! environment.
//!
//! A check runs project code, so it starts only on an explicit request, only
//! while the owner allows project language servers for the session, and only
//! inside the session's own execution keeper: the process belongs to the
//! session's execution unit and its resource budget, never to the Controller.
//! The keeper is driven through the same session-bound execution route the
//! Agent worker uses. Results are process-local evidence for the inputs the
//! command saw; a Controller restart forgets them and never replays a run.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{Value, json};

use super::*;
use crate::code_review::checks::{self, Check, Parsed};
use crate::execution_environment::BindingV1;
use crate::execution_protocol::{Command, Invocation, Outcome, Response as KeeperResponse};
use crate::machine_protocol::execution::{Action, Request, Response as ExecutionResponse};

mod product;

const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const READ_BYTES: u64 = 1024 * 1024;
const KEEPER_WAIT_MS: u64 = 5_000;
/// Retained runs per session: the latest per check id.
const MAX_SESSIONS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::server) enum RunState {
    Running,
    Passed,
    Failed,
    TimedOut,
    Error,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::server) struct Run {
    pub(in crate::server) run_id: String,
    pub(in crate::server) check: String,
    pub(in crate::server) state: RunState,
    pub(in crate::server) started_at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(in crate::server) finished_at_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(in crate::server) exit_code: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(in crate::server) error: Option<String>,
    pub(in crate::server) output_truncated: bool,
    #[serde(flatten)]
    pub(in crate::server) parsed: Parsed,
}

static RUNS: LazyLock<Mutex<HashMap<String, BTreeMap<String, Run>>>> =
    LazyLock::new(Mutex::default);

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// The latest run of each check of a session.
pub(in crate::server) fn runs(session_id: &str) -> Vec<Run> {
    RUNS.lock()
        .get(session_id)
        .map(|runs| runs.values().cloned().collect())
        .unwrap_or_default()
}

fn record(session_id: &str, run: Run) {
    let mut all = RUNS.lock();
    record_locked(&mut all, session_id, run);
}

fn record_locked(all: &mut HashMap<String, BTreeMap<String, Run>>, session_id: &str, run: Run) {
    if all.len() >= MAX_SESSIONS && !all.contains_key(session_id) {
        // Forget a session with nothing running; never drop a live run.
        if let Some(idle) = all
            .iter()
            .find(|(_, runs)| runs.values().all(|run| run.state != RunState::Running))
            .map(|(id, _)| id.clone())
        {
            all.remove(&idle);
        }
    }
    all.entry(session_id.to_owned())
        .or_default()
        .insert(run.check.clone(), run);
}

/// The session's declared checks, read like any other worktree file.
pub(in crate::server) async fn declared(
    state: &AppState,
    scope: &CodeReadScope,
    cwd: String,
) -> Result<Vec<Check>, String> {
    let operation = CodeOperation::File {
        path: checks::CONFIG_PATH.to_owned(),
        cursor: None,
    };
    let document = match remote_code_request(state, scope, operation).await {
        Ok(Some(crate::code_adapter::CodeAdapterResponse::File(document))) => document,
        Ok(Some(_)) | Err(_) => return Err("checks are unavailable on this Machine".into()),
        Ok(None) => tokio::task::spawn_blocking(move || {
            crate::code_review::LocalCodeProvider::new(cwd).file_page(checks::CONFIG_PATH, None)
        })
        .await
        .map_err(|_| "reading checks failed".to_owned())?
        .map_err(|_| format!("this worktree declares no checks ({})", checks::CONFIG_PATH))?,
    };
    if document.truncated || document.next_cursor.is_some() {
        return Err(format!("{} is too large", checks::CONFIG_PATH));
    }
    Ok(checks::parse_config(document.text.as_bytes())?.checks)
}

pub(in crate::server) fn routes() -> Router<Arc<AppState>> {
    Router::new().route(
        "/api/code/sessions/{id}/checks",
        get(api_checks).post(product::start_product),
    )
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeclaredCheck {
    id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    format: checks::Format,
    timeout_seconds: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ChecksResponse {
    api_version: u8,
    epoch: &'static str,
    allowed: bool,
    runnable: bool,
    declared: Vec<DeclaredCheck>,
    #[serde(skip_serializing_if = "Option::is_none")]
    declaration_error: Option<String>,
    runs: Vec<Run>,
}

/// Declared checks and the latest run of each, for Review annotations. Runs
/// only exist for sessions; a browse-only context reports none.
async fn api_checks(
    State(state): State<Arc<AppState>>,
    authority: code_reads::Authority,
    Path(session_id): Path<String>,
) -> Response {
    let context_id = session_id.clone();
    code_reads::scoped(authority, &context_id, |context| async move {
        let session = matches!(context.scope, CodeReadScope::Session(_));
        let declared = if session {
            declared(&state, &context.scope, context.cwd).await
        } else {
            Ok(Vec::new())
        };
        let (declared, declaration_error) = match declared {
            Ok(declared) => (declared, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        Json(ChecksResponse {
            api_version: 1,
            epoch: &product::EPOCH,
            allowed: session && state.hub.language_servers_allowed(&session_id),
            runnable: session
                && state.hub.language_servers_allowed(&session_id)
                && state.hub.session_info(&session_id).is_some_and(|info| {
                    info.meta.execution_binding.as_ref().is_some_and(|binding| {
                        binding
                            .for_runtime(&info.meta.machine_id, &info.meta.cwd)
                            .is_ok()
                    })
                }),
            declared: declared
                .into_iter()
                .map(|check| DeclaredCheck {
                    timeout_seconds: check.timeout().as_secs(),
                    id: check.id,
                    title: check.title,
                    format: check.format,
                })
                .collect(),
            declaration_error,
            runs: if session {
                runs(&session_id)
            } else {
                Vec::new()
            },
        })
        .into_response()
    })
    .await
}

pub(in crate::server) enum Started {
    New(Run),
    Existing(Run),
}

/// Start one declared check, or observe the run this operation already started.
pub(in crate::server) async fn start(
    state: &Arc<AppState>,
    session_id: &str,
    check_id: &str,
    operation: &str,
) -> Result<Started, String> {
    start_authorized(state, session_id, check_id, operation, None).await
}

async fn start_authorized(
    state: &Arc<AppState>,
    session_id: &str,
    check_id: &str,
    operation: &str,
    approval: Option<&crate::server::operator_approval::OperatorApproval>,
) -> Result<Started, String> {
    if let Some(approval) = approval {
        product::authorize(state, session_id, approval).await?;
    }
    if let Some(run) = runs(session_id).into_iter().find(|run| {
        run.check == check_id && (run.run_id == operation || run.state == RunState::Running)
    }) {
        return Ok(Started::Existing(run));
    }
    if !state.hub.language_servers_allowed(session_id) {
        return Err("restricted: project code is off for this session".into());
    }
    let scope = code_reads::session::resolve(
        &state.hub,
        &state.machine_control,
        &state.service_id,
        session_id,
    )
    .ok_or("unavailable")?;
    let CodeReadScope::Session(observed) = &scope else {
        return Err("unavailable".into());
    };
    let info = state.hub.session_info(session_id).ok_or("unavailable")?;
    let binding = info
        .meta
        .execution_binding
        .as_ref()
        .and_then(|binding| {
            binding
                .for_runtime(&info.meta.machine_id, &info.meta.cwd)
                .ok()
        })
        .ok_or("checks need a session with an execution environment")?;
    let check = declared(state, &scope, observed.session().cwd().to_owned())
        .await?
        .into_iter()
        .find(|check| check.id == check_id)
        .ok_or_else(|| format!("no declared check named {check_id}"))?;
    let run = Run {
        run_id: operation.to_owned(),
        check: check.id.clone(),
        state: RunState::Running,
        started_at_ms: now_ms(),
        finished_at_ms: None,
        exit_code: None,
        error: None,
        output_truncated: false,
        parsed: Parsed::default(),
    };
    if let Some(approval) = approval {
        product::authorize(state, session_id, approval).await?;
    }
    if !state.hub.code_scope_is_current(observed.session())
        || !state.hub.language_servers_allowed(session_id)
    {
        return Err("restricted: session scope changed".into());
    }
    {
        let mut all = RUNS.lock();
        if let Some(existing) = all
            .get(session_id)
            .and_then(|runs| runs.get(check_id))
            .filter(|run| run.run_id == operation || run.state == RunState::Running)
        {
            return Ok(Started::Existing(existing.clone()));
        }
        record_locked(&mut all, session_id, run.clone());
    }
    let state = Arc::clone(state);
    let session = session_id.to_owned();
    let initial = run.clone();
    tokio::spawn(async move {
        let finished = execute(&state, &session, &binding, &check, initial).await;
        record(&session, finished);
    });
    Ok(Started::New(run))
}

struct Keeper<'a> {
    state: &'a AppState,
    session: &'a str,
    binding: &'a BindingV1,
    prefix: String,
    sequence: u32,
}

impl Keeper<'_> {
    async fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.sequence += 1;
        let operation_id = format!("{}-{}", self.prefix, self.sequence);
        let mut command = Command::Invoke {
            invocation: Invocation {
                operation_id: operation_id.clone(),
                method: method.to_owned(),
                params,
            },
            wait_ms: KEEPER_WAIT_MS,
        };
        for _ in 0..120 {
            let machine = &self.binding.environment.machine_id;
            let response = self
                .state
                .machine_control
                .execution_request(
                    machine,
                    Request {
                        service_id: self.state.service_id.clone(),
                        machine_id: machine.clone(),
                        action: Action::Call {
                            session_id: self.session.to_owned(),
                            binding: Box::new(self.binding.clone()),
                            command: command.clone(),
                        },
                    },
                )
                .await?;
            let ExecutionResponse::Call { response } = response else {
                return Err("the execution environment refused the check".into());
            };
            match response {
                KeeperResponse::Operation {
                    outcome: Outcome::Completed { reply },
                } => {
                    if let Some(error) = reply.get("error") {
                        return Err(format!("{method} failed: {error}"));
                    }
                    return Ok(reply.get("result").cloned().unwrap_or(reply));
                }
                KeeperResponse::Operation {
                    outcome: Outcome::Pending,
                } => {
                    command = Command::Observe {
                        operation_id: operation_id.clone(),
                        wait_ms: KEEPER_WAIT_MS,
                    };
                }
                other => return Err(format!("{method} did not complete: {other:?}")),
            }
        }
        Err(format!("{method} did not complete"))
    }
}

async fn execute(
    state: &AppState,
    session: &str,
    binding: &BindingV1,
    check: &Check,
    mut run: Run,
) -> Run {
    let suffix: String = run
        .run_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(40)
        .collect();
    let process = format!("cowboy-check-{suffix}");
    let mut keeper = Keeper {
        state,
        session,
        binding,
        prefix: process.clone(),
        sequence: 0,
    };
    let root = binding.workspace.cwd.clone();
    let outcome = drive(&mut keeper, &process, &root, check).await;
    run.finished_at_ms = Some(now_ms());
    match outcome {
        Ok((exit, stdout, stderr, truncated, timed_out)) => {
            run.exit_code = exit;
            run.output_truncated = truncated;
            run.parsed = checks::parse(
                check.format,
                &String::from_utf8_lossy(&stdout),
                &String::from_utf8_lossy(&stderr),
                &root,
            );
            run.state = if timed_out {
                RunState::TimedOut
            } else if exit == Some(0) {
                RunState::Passed
            } else {
                RunState::Failed
            };
        }
        Err(error) => {
            run.state = RunState::Error;
            run.error = Some(error.chars().take(500).collect());
        }
    }
    run
}

type Finished = (Option<i64>, Vec<u8>, Vec<u8>, bool, bool);

async fn drive(
    keeper: &mut Keeper<'_>,
    process: &str,
    root: &str,
    check: &Check,
) -> Result<Finished, String> {
    let cwd = url::Url::from_directory_path(root)
        .map_err(|()| "the execution worktree is not an absolute path".to_owned())?;
    let started = keeper
        .call(
            "process/start",
            json!({
                "processId": process,
                "argv": check.argv,
                "cwd": cwd.as_str(),
                "env": {},
                "tty": false,
                "pipeStdin": false,
                "arg0": null,
                "envPolicy": {"inherit":"all","ignoreDefaultExcludes":false,"exclude":[],"set":{},"includeOnly":[]},
            }),
        )
        .await?;
    if started.get("processId").and_then(Value::as_str) != Some(process) {
        return Err("the execution environment changed the process identity".into());
    }
    let deadline = Instant::now() + check.timeout();
    let (mut stdout, mut stderr, mut truncated) = (Vec::new(), Vec::new(), false);
    let mut after: u64 = 0;
    loop {
        if Instant::now() >= deadline {
            let _ = keeper
                .call("process/terminate", json!({"processId": process}))
                .await;
            return Ok((None, stdout, stderr, truncated, true));
        }
        let wait = deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_secs(1));
        let read = keeper
            .call(
                "process/read",
                json!({"processId": process, "afterSeq": after, "maxBytes": READ_BYTES,
                    "waitMs": wait.as_millis().max(1)}),
            )
            .await?;
        for chunk in read
            .get("chunks")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            after = chunk.get("seq").and_then(Value::as_u64).unwrap_or(after);
            let bytes = chunk
                .get("chunk")
                .and_then(Value::as_str)
                .and_then(|text| {
                    use base64::Engine as _;
                    base64::engine::general_purpose::STANDARD.decode(text).ok()
                })
                .unwrap_or_default();
            let target = if chunk.get("stream").and_then(Value::as_str) == Some("stderr") {
                &mut stderr
            } else {
                &mut stdout
            };
            if target.len() + bytes.len() > MAX_OUTPUT_BYTES {
                truncated = true;
            } else {
                target.extend_from_slice(&bytes);
            }
        }
        let has_chunks = read
            .get("chunks")
            .and_then(Value::as_array)
            .is_some_and(|chunks| !chunks.is_empty());
        if read.get("closed").and_then(Value::as_bool) == Some(true) && !has_chunks {
            let exit = read.get("exitCode").and_then(Value::as_i64);
            return Ok((exit, stdout, stderr, truncated, false));
        }
    }
}
