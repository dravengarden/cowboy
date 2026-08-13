//! ACP client backend.
//!
//! cowboy is the ACP *client* (design §2): it drives each agent (the ACP
//! *server*) over stdio. This module is the only place that touches the
//! `agent-client-protocol` crate, so a crate bump is contained here.
//!
//! The crate is built around role-typed connections
//! ([`agent_client_protocol::Client`]/[`Agent`] markers + [`ConnectionTo`]).
//! `connect_with` runs the handshake + command loop in `run_session`; incoming
//! `session/update` notifications and permission requests are handled by the
//! `on_receive_*` closures, which translate each ACP `SessionUpdate` into a
//! normalized [`crate::core::Event`] on the shared [`Hub`]. Commands flow in
//! over a `Send` channel ([`AgentCommand`]).
//!
//! Everything here is `Send`: the crate dispatches handlers and `cx.spawn`ed
//! tasks on its own executor (driven by the `connect_with` future), and those
//! require `Send` futures. The shared `Hub` is already `Arc`-backed; the small
//! per-session client state ([`ClientState`]) uses `Arc` + `Mutex`/atomics.

#![warn(clippy::pedantic)]

use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

// ACP's stable wire schema lives under `schema::v1::`; SDK major versions do
// not change that protocol version. `ProtocolVersion` stays at the
// version-agnostic schema root and the `Agent`/`Client` traits at the crate root.
use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    CancelNotification, ClientNotification, ContentBlock, ExtNotification, InitializeRequest,
    LoadSessionRequest, Meta, NewSessionRequest, PermissionOptionId, PermissionOptionKind,
    PromptRequest, RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    ResumeSessionRequest, SelectedPermissionOutcome, SessionConfigKind, SessionConfigOption,
    SessionConfigOptionValue, SessionConfigSelectOption, SessionConfigSelectOptions, SessionId,
    SessionModeId, SessionNotification, SessionUpdate, SetSessionConfigOptionRequest,
    SetSessionModeRequest,
};
use agent_client_protocol::{
    Agent, ByteStreams, Client, ConnectionTo, Error, JsonRpcRequest, JsonRpcResponse,
};
use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt as _, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use crate::agent_model::{
    AUTO_CONTINUE_PREFIX, Event, SCHED_PREFIX, SessionUsage, Status, WAKEUP_PREFIX,
};
use crate::agent_sink::AgentSink;
use crate::cgroup;
use crate::provider::LaunchSpec;

/// Maximum time allowed for each distinct ACP startup phase. The watchdog
/// resets when the agent advances from initialize to session establishment and
/// then startup configuration, so a slow but progressing launch is not charged
/// against one opaque deadline.
pub(crate) const STARTUP_PHASE_TIMEOUT: Duration = Duration::from_mins(1);
const RESUME_PHASE_TIMEOUT: Duration = Duration::from_mins(4);
const CODEX_FULL_ACCESS_CONFIG_ID: &str = "mode";
const CODEX_FULL_ACCESS_CONFIG_VALUE: &str = "agent-full-access";
const CLAUDE_EMPTY_STREAM_MESSAGE: &str = "API Error: Stream ended without receiving any events";
const GROK_SESSION_CONFIG_META: &str = "x.ai/sessionConfig";
const GROK_MODEL_CONFIG_ID: &str = "model";
const GROK_REASONING_CONFIG_ID: &str = "reasoning_effort";
const GROK_SESSION_MODE_CONFIG_ID: &str = "session_mode";
const GROK_PERMISSION_CONFIG_ID: &str = "permission_mode";
const GROK_PERMISSION_NOTIFICATION: &str = "x.ai/yolo_mode_changed";
const GROK_SESSION_INFO_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GrokPermissionMode {
    Default,
    Auto,
    AlwaysApprove,
}

impl GrokPermissionMode {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "default" | "ask" => Some(Self::Default),
            "auto" => Some(Self::Auto),
            "always-approve" => Some(Self::AlwaysApprove),
            _ => None,
        }
    }

    const fn id(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Auto => "auto",
            Self::AlwaysApprove => "always-approve",
        }
    }

    const fn yolo_mode(self) -> bool {
        matches!(self, Self::AlwaysApprove)
    }

    const fn auto_mode(self) -> bool {
        matches!(self, Self::Auto)
    }
}

fn grok_permission_option(current: GrokPermissionMode) -> SessionConfigOption {
    SessionConfigOption::select(
        GROK_PERMISSION_CONFIG_ID,
        "Permission",
        current.id(),
        vec![
            SessionConfigSelectOption::new("default", "Default (Ask)")
                .description("Ask before sensitive tool calls"),
            SessionConfigSelectOption::new("auto", "Auto")
                .description("Automatically approve safe actions"),
            SessionConfigSelectOption::new("always-approve", "Always Approve")
                .description("Run tools without ordinary permission prompts"),
        ],
    )
}

fn grok_permission_notification(mode: GrokPermissionMode) -> ClientNotification {
    let params = serde_json::json!({
        "yolo_mode": mode.yolo_mode(),
        "auto_mode": mode.auto_mode(),
        "permission_mode": mode.id(),
    });
    ClientNotification::ExtNotification(ExtNotification::new(
        GROK_PERMISSION_NOTIFICATION,
        serde_json::value::to_raw_value(&params)
            .expect("serialize Grok permission mode notification")
            .into(),
    ))
}

// Grok Build exposes the unstable ACP model-switch request as
// `session/set_model`. Keep this wire-only compatibility type local so a future
// schema upgrade can delete it without changing Cowboy's provider-independent
// command contract.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, JsonRpcRequest)]
#[request(method = "session/set_model", response = GrokSetSessionModelResponse)]
#[serde(rename_all = "camelCase")]
struct GrokSetSessionModelRequest {
    session_id: SessionId,
    model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "_meta")]
    meta: Option<Meta>,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, JsonRpcResponse)]
#[serde(rename_all = "camelCase")]
struct GrokSetSessionModelResponse {
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "_meta")]
    meta: Option<Meta>,
}

// Grok Build does not emit ACP's standard `usage_update`. Its local
// `x.ai/session/info` extension exposes the authoritative context snapshot
// without running a model turn, so keep the compatibility wire types here.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, JsonRpcRequest)]
#[request(method = "x.ai/session/info", response = GrokSessionInfoResponse)]
#[serde(rename_all = "camelCase")]
struct GrokSessionInfoRequest {
    session_id: SessionId,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, JsonRpcResponse)]
#[serde(rename_all = "camelCase")]
struct GrokSessionInfoResponse {
    result: Option<GrokSessionInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<serde_json::Value>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GrokSessionInfo {
    context: GrokContextInfo,
    #[serde(flatten)]
    extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GrokContextInfo {
    used: u64,
    total: u64,
    #[serde(flatten)]
    extra: HashMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GrokRawConfigOption {
    id: String,
    category: String,
    label: String,
    #[serde(default)]
    description: Option<String>,
    selected: bool,
    #[serde(skip)]
    wire_value: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GrokInitializeModelState {
    current_model_id: String,
    available_models: Vec<GrokInitializeModel>,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GrokInitializeModel {
    model_id: String,
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default, rename = "_meta")]
    meta: Option<serde_json::Value>,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct GrokInitializeEffort {
    id: String,
    #[serde(default)]
    value: Option<String>,
    label: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    default: bool,
}

#[derive(Clone, Debug)]
struct GrokSessionConfig {
    options: Vec<GrokRawConfigOption>,
    efforts_by_model: HashMap<String, Vec<GrokRawConfigOption>>,
}

impl GrokSessionConfig {
    fn from_metadata(session_meta: Option<&Meta>, initialize_meta: Option<&Meta>) -> Option<Self> {
        let mut session_options = session_meta
            .and_then(|meta| meta.get(GROK_SESSION_CONFIG_META))
            .and_then(|config| config.get("options"))
            .cloned()
            .and_then(|options| serde_json::from_value::<Vec<GrokRawConfigOption>>(options).ok())
            .unwrap_or_default();
        let model_state = initialize_meta
            .and_then(|meta| meta.get("modelState"))
            .cloned()
            .and_then(|state| serde_json::from_value::<GrokInitializeModelState>(state).ok());
        let mut initialize_options = Vec::new();
        let mut efforts_by_model = HashMap::new();
        if let Some(state) = model_state {
            for model in state.available_models {
                let selected = model.model_id == state.current_model_id;
                initialize_options.push(GrokRawConfigOption {
                    id: model.model_id.clone(),
                    category: "model".to_owned(),
                    label: model.name,
                    description: model.description,
                    selected,
                    wire_value: None,
                });
                let current_effort = model
                    .meta
                    .as_ref()
                    .and_then(|meta| meta.get("reasoningEffort"))
                    .and_then(serde_json::Value::as_str);
                let efforts = model
                    .meta
                    .as_ref()
                    .and_then(|meta| meta.get("reasoningEfforts"))
                    .cloned()
                    .and_then(|efforts| {
                        serde_json::from_value::<Vec<GrokInitializeEffort>>(efforts).ok()
                    })
                    .unwrap_or_default()
                    .into_iter()
                    .map(|effort| {
                        let wire_value = effort.value.unwrap_or_else(|| effort.id.clone());
                        GrokRawConfigOption {
                            selected: current_effort == Some(wire_value.as_str())
                                || (current_effort.is_none() && effort.default),
                            id: effort.id,
                            category: "mode".to_owned(),
                            label: effort.label,
                            description: effort.description,
                            wire_value: Some(wire_value),
                        }
                    })
                    .collect::<Vec<_>>();
                if !efforts.is_empty() {
                    efforts_by_model.insert(model.model_id, efforts);
                }
            }
        }
        if session_options.is_empty() {
            session_options = initialize_options;
        }
        let selected_model = session_options
            .iter()
            .find(|option| option.category == "model" && option.selected)
            .map(|option| option.id.clone());
        if let Some(efforts) = selected_model
            .as_ref()
            .and_then(|model| efforts_by_model.get(model))
        {
            for option in session_options
                .iter_mut()
                .filter(|option| option.category == "mode")
            {
                option.wire_value = efforts
                    .iter()
                    .find(|effort| effort.id == option.id)
                    .and_then(|effort| effort.wire_value.clone());
            }
        }
        if !session_options
            .iter()
            .any(|option| option.category == "mode")
            && let Some(efforts) = selected_model
                .as_ref()
                .and_then(|model| efforts_by_model.get(model))
        {
            session_options.extend(efforts.clone());
        }
        (!session_options.is_empty()).then_some(Self {
            options: session_options,
            efforts_by_model,
        })
    }

    fn selected(&self, category: &str) -> Option<&str> {
        self.options
            .iter()
            .find(|option| option.category == category && option.selected)
            .map(|option| option.id.as_str())
    }

    fn selected_wire_value(&self, category: &str) -> Option<&str> {
        self.options
            .iter()
            .find(|option| option.category == category && option.selected)
            .map(|option| option.wire_value.as_deref().unwrap_or(&option.id))
    }

    fn select(&mut self, category: &str, id: &str) -> bool {
        if !self
            .options
            .iter()
            .any(|option| option.category == category && option.id == id)
        {
            return false;
        }
        for option in &mut self.options {
            if option.category == category {
                option.selected = option.id == id;
            }
        }
        true
    }

    fn select_model(&mut self, id: &str) -> bool {
        let current_effort = self.selected("mode").map(str::to_owned);
        if !self.select("model", id) {
            return false;
        }
        let Some(mut efforts) = self.efforts_by_model.get(id).cloned() else {
            // Reasoning support is model-specific. Never carry the previous
            // model's menu (or wire value) into a model that advertises none.
            self.options.retain(|option| option.category != "mode");
            return true;
        };
        let selected = current_effort
            .as_ref()
            .filter(|current| efforts.iter().any(|effort| &effort.id == *current))
            .cloned()
            .or_else(|| {
                efforts
                    .iter()
                    .find(|effort| effort.selected)
                    .map(|effort| effort.id.clone())
            })
            .or_else(|| efforts.first().map(|effort| effort.id.clone()));
        for effort in &mut efforts {
            effort.selected = selected.as_deref() == Some(effort.id.as_str());
        }
        self.options.retain(|option| option.category != "mode");
        self.options.extend(efforts);
        true
    }

    fn cowboy_options(&self) -> Vec<SessionConfigOption> {
        [
            ("model", GROK_MODEL_CONFIG_ID, "Model"),
            ("mode", GROK_REASONING_CONFIG_ID, "Reasoning"),
        ]
        .into_iter()
        .filter_map(|(category, config_id, label)| {
            let choices = self
                .options
                .iter()
                .filter(|option| option.category == category)
                .map(|option| {
                    SessionConfigSelectOption::new(option.id.clone(), option.label.clone())
                        .description(option.description.clone())
                })
                .collect::<Vec<_>>();
            if choices.is_empty() {
                return None;
            }
            let selected = self
                .selected(category)
                .map(str::to_owned)
                .or_else(|| choices.first().map(|choice| choice.value.0.to_string()))?;
            Some(SessionConfigOption::select(
                config_id, label, selected, choices,
            ))
        })
        .collect()
    }
}

fn grok_cowboy_options(
    config: Option<&GrokSessionConfig>,
    permission_mode: GrokPermissionMode,
    mode_config_id: Option<&'static str>,
    mode_select: Option<&[SessionConfigSelectOption]>,
    current_session_mode: Option<&str>,
) -> Vec<SessionConfigOption> {
    let mut options = Vec::new();
    if let (Some(config_id), Some(choices), Some(current)) =
        (mode_config_id, mode_select, current_session_mode)
    {
        options.push(SessionConfigOption::select(
            config_id,
            "Mode",
            current.to_owned(),
            choices.to_vec(),
        ));
    }
    options.push(grok_permission_option(permission_mode));
    if let Some(config) = config {
        options.extend(config.cowboy_options());
    }
    options
}

fn grok_model_request(
    session_id: SessionId,
    model_id: String,
    reasoning_effort: Option<&str>,
) -> GrokSetSessionModelRequest {
    let meta = reasoning_effort.map(|effort| {
        let mut meta = Meta::new();
        meta.insert("reasoningEffort".to_owned(), serde_json::json!(effort));
        meta
    });
    GrokSetSessionModelRequest {
        session_id,
        model_id,
        meta,
    }
}

fn observed_at_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
        })
}

fn grok_session_usage(
    response: &GrokSessionInfoResponse,
    observed_at_ms: i64,
) -> Option<SessionUsage> {
    let info = response.result.as_ref()?;
    (info.context.total > 0).then(|| SessionUsage {
        used: info.context.used,
        size: info.context.total,
        raw: serde_json::to_value(response).unwrap_or(serde_json::Value::Null),
        observed_at_ms,
    })
}

async fn run_grok_usage_refresh_queue(
    cx: ConnectionTo<Agent>,
    sink: Arc<dyn AgentSink>,
    session_id: String,
    acp_id: SessionId,
    mut refreshes: mpsc::UnboundedReceiver<()>,
) -> Result<(), Error> {
    // Serialize snapshots so an older request cannot arrive after and replace
    // a post-turn or post-model-change value. Unsupported/older Grok CLIs are
    // intentionally a quiet compatibility fallback, never a user-facing turn
    // or runtime error.
    while refreshes.recv().await.is_some() {
        let request = GrokSessionInfoRequest {
            session_id: acp_id.clone(),
        };
        match tokio::time::timeout(
            GROK_SESSION_INFO_TIMEOUT,
            cx.send_request(request).block_task(),
        )
        .await
        {
            Ok(Ok(response)) => {
                if response.error.is_some() {
                    tracing::debug!(
                        session = %session_id,
                        error = ?response.error,
                        "Grok session info returned an extension error"
                    );
                } else if let Some(usage) = grok_session_usage(&response, observed_at_ms()) {
                    sink.set_session_usage(&session_id, usage);
                } else {
                    tracing::debug!(
                        session = %session_id,
                        "Grok session info did not include a usable context window"
                    );
                }
            }
            Ok(Err(error)) => {
                tracing::debug!(
                    session = %session_id,
                    error = ?error,
                    "Grok session info is unavailable"
                );
            }
            Err(_) => {
                tracing::debug!(
                    session = %session_id,
                    timeout_seconds = GROK_SESSION_INFO_TIMEOUT.as_secs(),
                    "Grok session info timed out"
                );
            }
        }
    }
    Ok(())
}

#[derive(Debug)]
struct GrokConfigChange {
    config_id: String,
    requested: String,
}

#[allow(clippy::too_many_arguments)]
async fn run_grok_config_queue(
    cx: ConnectionTo<Agent>,
    sink: Arc<dyn AgentSink>,
    session_id: String,
    acp_id: SessionId,
    grok_config: Arc<Mutex<Option<GrokSessionConfig>>>,
    current_permission_mode: Arc<Mutex<GrokPermissionMode>>,
    current_session_mode: Arc<Mutex<Option<String>>>,
    mode_config_id: Option<&'static str>,
    mode_select: Option<Vec<SessionConfigSelectOption>>,
    usage_refresh: mpsc::UnboundedSender<()>,
    mut changes: mpsc::UnboundedReceiver<GrokConfigChange>,
) -> Result<(), Error> {
    // One FIFO owns every Grok model/effort mutation. This keeps the state used
    // to build request N+1 authoritative after request N and prevents an older
    // response from overwriting a newer selection in Cowboy's UI.
    while let Some(change) = changes.recv().await {
        let (next_config, model_id, reasoning_effort) = {
            let Some(config) = grok_config.lock().as_ref().cloned() else {
                sink.broadcast_error(
                    Some(session_id.clone()),
                    "Grok did not advertise model configuration for this session".to_owned(),
                );
                continue;
            };
            let mut next = config;
            let selected = if change.config_id == GROK_MODEL_CONFIG_ID {
                next.select_model(&change.requested)
            } else {
                next.select("mode", &change.requested)
            };
            if !selected {
                sink.broadcast_error(
                    Some(session_id.clone()),
                    format!(
                        "Grok did not advertise {} value {:?}",
                        change.config_id, change.requested
                    ),
                );
                continue;
            }
            (
                next.clone(),
                next.selected("model").map(str::to_owned),
                next.selected_wire_value("mode").map(str::to_owned),
            )
        };
        let Some(model_id) = model_id else {
            sink.broadcast_error(
                Some(session_id.clone()),
                "Grok did not report a selected model".to_owned(),
            );
            continue;
        };
        let request = grok_model_request(acp_id.clone(), model_id, reasoning_effort.as_deref());
        match cx.send_request(request).block_task().await {
            Ok(_) => {
                *grok_config.lock() = Some(next_config);
                let config = grok_config.lock().clone();
                let permission_mode = *current_permission_mode.lock();
                let session_mode = current_session_mode.lock().clone();
                let published = grok_cowboy_options(
                    config.as_ref(),
                    permission_mode,
                    mode_config_id,
                    mode_select.as_deref(),
                    session_mode.as_deref(),
                );
                match serde_json::to_value(published) {
                    Ok(options) => sink.set_config_options(&session_id, options),
                    Err(error) => {
                        tracing::warn!(error = %error, "serializing Grok config options");
                    }
                }
                if usage_refresh.send(()).is_err() {
                    tracing::debug!(
                        session = %session_id,
                        "Grok usage refresh queue closed after configuration change"
                    );
                }
            }
            Err(error) => sink.broadcast_error(
                Some(session_id.clone()),
                format!("set {}: {error}", change.config_id),
            ),
        }
    }
    Ok(())
}

/// Attach a stable, content-free session identity to requests sent through the
/// local `DeepSeek` gateways. The gateways HMAC this opaque value for telemetry
/// and deliberately do not forward the Cowboy-only header upstream.
fn deepseek_session_environment(
    provider_id: &str,
    session_id: &str,
    existing_claude_headers: Option<&str>,
    cache_policy: Option<&str>,
) -> Option<(&'static str, String)> {
    let opaque_session_id = crate::deepseek_cache::opaque_session_id(session_id);
    match provider_id {
        "claude-deepseek" => {
            let mut headers = existing_claude_headers
                .unwrap_or_default()
                .trim()
                .to_owned();
            if !headers.is_empty() {
                headers.push('\n');
            }
            headers.push_str("X-Cowboy-Session-Id: ");
            headers.push_str(&opaque_session_id);
            if let Some(policy @ ("auto" | "off")) = cache_policy {
                headers.push_str("\nX-Cowboy-Cache-Protection: ");
                headers.push_str(policy);
            }
            Some(("ANTHROPIC_CUSTOM_HEADERS", headers))
        }
        "codex-deepseek" => Some((crate::provider::DEEPSEEK_SESSION_ID_ENV, opaque_session_id)),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StartupPhase {
    Initialize,
    Resume,
    Load,
    New,
    Configure,
    Ready,
}

impl StartupPhase {
    const fn method(self) -> &'static str {
        match self {
            Self::Initialize => "initialize",
            Self::Resume => "session/resume",
            Self::Load => "session/load",
            Self::New => "session/new",
            Self::Configure => "startup configuration",
            Self::Ready => "ready",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResumeMethod {
    Resume,
    Load,
}

const fn select_resume_method(
    agent_can_resume: bool,
    agent_can_load: bool,
) -> Option<ResumeMethod> {
    if agent_can_resume {
        Some(ResumeMethod::Resume)
    } else if agent_can_load {
        Some(ResumeMethod::Load)
    } else {
        None
    }
}

fn startup_full_access_mode(provider_id: &str) -> Option<&'static str> {
    if crate::provider::is_claude(provider_id) {
        return Some("bypassPermissions");
    }
    match provider_id {
        "gemini" => Some("yolo"),
        _ => None,
    }
}

/// Keep Claude's cacheable system-prefix stable across new and resumed ACP
/// processes. The Agent SDK re-injects the excluded cwd, auto-memory, and Git
/// sections in the first user message, so the model retains the context while
/// the system prompt no longer changes when the process is reconstructed.
fn stable_claude_session_meta(provider_id: &str) -> Option<Meta> {
    if !crate::provider::is_claude(provider_id) {
        return None;
    }

    let mut meta = Meta::new();
    meta.insert(
        "systemPrompt".to_owned(),
        serde_json::json!({
            "type": "preset",
            "preset": "claude_code",
            "excludeDynamicSections": true,
        }),
    );
    Some(meta)
}

fn new_session_request(provider_id: &str, cwd: PathBuf) -> NewSessionRequest {
    NewSessionRequest::new(cwd).meta(stable_claude_session_meta(provider_id))
}

fn resume_session_request(
    provider_id: &str,
    session_id: SessionId,
    cwd: PathBuf,
) -> ResumeSessionRequest {
    ResumeSessionRequest::new(session_id, cwd).meta(stable_claude_session_meta(provider_id))
}

fn load_session_request(
    provider_id: &str,
    session_id: SessionId,
    cwd: PathBuf,
) -> LoadSessionRequest {
    LoadSessionRequest::new(session_id, cwd).meta(stable_claude_session_meta(provider_id))
}

#[cfg(test)]
mod startup_mode_tests {
    use super::{
        ActivePrompt, GrokPermissionMode, GrokSessionConfig, GrokSessionInfoRequest,
        GrokSessionInfoResponse, ResumeMethod, StartupPhase, StartupTimeout,
        codex_full_access_available, codex_full_access_selected, deepseek_session_environment,
        grok_cowboy_options, grok_model_request, grok_permission_notification, grok_session_usage,
        is_empty_stream_message_update, load_session_request, new_session_request,
        resume_session_request, select_resume_method, session_config_value,
        startup_full_access_mode,
    };
    use agent_client_protocol::JsonRpcMessage as _;
    use agent_client_protocol::schema::v1::{
        SessionConfigOption, SessionConfigOptionValue, SessionConfigSelectOption, SessionId,
    };
    use std::path::PathBuf;

    #[test]
    fn providers_use_their_native_full_access_mode() {
        assert_eq!(
            startup_full_access_mode("claude-code"),
            Some("bypassPermissions")
        );
        assert_eq!(
            startup_full_access_mode("claude-deepseek"),
            Some("bypassPermissions")
        );
        assert_eq!(startup_full_access_mode("gemini"), Some("yolo"));
        assert_eq!(startup_full_access_mode("codex"), None);
    }

    #[test]
    fn every_claude_session_setup_path_requests_a_stable_system_prefix() {
        let cwd = PathBuf::from("/tmp/workspace");
        for provider in ["claude-code", "claude-deepseek"] {
            let requests = [
                serde_json::to_value(new_session_request(provider, cwd.clone()))
                    .expect("new request"),
                serde_json::to_value(resume_session_request(
                    provider,
                    SessionId::new("session"),
                    cwd.clone(),
                ))
                .expect("resume request"),
                serde_json::to_value(load_session_request(
                    provider,
                    SessionId::new("session"),
                    cwd.clone(),
                ))
                .expect("load request"),
            ];
            for request in requests {
                assert_eq!(
                    request.pointer("/_meta/systemPrompt"),
                    Some(&serde_json::json!({
                        "type": "preset",
                        "preset": "claude_code",
                        "excludeDynamicSections": true,
                    }))
                );
            }
        }

        let request =
            serde_json::to_value(new_session_request("codex", cwd)).expect("non-Claude request");
        assert!(request.get("_meta").is_none());
    }

    #[test]
    fn deepseek_sessions_receive_stable_content_free_attribution() {
        let (claude_key, claude_value) = deepseek_session_environment(
            "claude-deepseek",
            "sess-private-value",
            Some("X-Existing: retained"),
            Some("auto"),
        )
        .expect("Claude DeepSeek attribution");
        let (codex_key, codex_value) = deepseek_session_environment(
            "codex-deepseek",
            "sess-private-value",
            None,
            Some("auto"),
        )
        .expect("Codex DeepSeek attribution");

        assert_eq!(claude_key, "ANTHROPIC_CUSTOM_HEADERS");
        assert_eq!(codex_key, "COWBOY_DEEPSEEK_SESSION_ID");
        assert_eq!(
            claude_value,
            format!(
                "X-Existing: retained\nX-Cowboy-Session-Id: {codex_value}\nX-Cowboy-Cache-Protection: auto"
            )
        );
        assert_eq!(codex_value.len(), 64);
        assert!(codex_value.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(!claude_value.contains("sess-private-value"));
        assert!(deepseek_session_environment("claude-code", "session", None, None).is_none());
        assert!(deepseek_session_environment("codex", "session", None, None).is_none());
    }

    #[test]
    fn codex_full_access_tracks_the_authoritative_mode() {
        let choices = vec![
            SessionConfigSelectOption::new("agent", "Agent"),
            SessionConfigSelectOption::new("agent-full-access", "Agent (full access)"),
        ];
        let restricted = vec![SessionConfigOption::select(
            "mode",
            "Mode",
            "agent",
            choices.clone(),
        )];
        let full_access = vec![SessionConfigOption::select(
            "mode",
            "Mode",
            "agent-full-access",
            choices,
        )];

        assert!(codex_full_access_available(&restricted));
        assert!(!codex_full_access_selected(&restricted));
        assert!(!codex_full_access_available(&full_access));
        assert!(codex_full_access_selected(&full_access));
    }

    #[test]
    fn config_values_preserve_typed_ids_and_booleans() {
        let selected = session_config_value(&serde_json::json!("agent")).expect("select value");
        assert_eq!(
            selected.as_value_id().map(|value| value.0.as_ref()),
            Some("agent")
        );

        let enabled = session_config_value(&serde_json::json!(true)).expect("boolean value");
        assert_eq!(enabled, SessionConfigOptionValue::boolean(true));
        assert!(session_config_value(&serde_json::json!(42)).is_err());
    }

    #[test]
    fn grok_session_metadata_becomes_standard_model_and_reasoning_options() {
        let mut meta = agent_client_protocol::schema::v1::Meta::new();
        meta.insert(
            "x.ai/sessionConfig".to_owned(),
            serde_json::json!({ "options": [
                { "id": "grok-4.5", "category": "model", "label": "Grok 4.5", "selected": true },
                { "id": "high", "category": "mode", "label": "High Effort", "selected": true },
                { "id": "medium", "category": "mode", "label": "Medium Effort", "selected": false }
            ]}),
        );
        let config = GrokSessionConfig::from_metadata(Some(&meta), None).expect("Grok config");
        let value = serde_json::to_value(config.cowboy_options()).expect("serialize options");
        assert_eq!(value.pointer("/0/id"), Some(&serde_json::json!("model")));
        assert_eq!(
            value.pointer("/0/currentValue"),
            Some(&serde_json::json!("grok-4.5"))
        );
        assert_eq!(
            value.pointer("/1/id"),
            Some(&serde_json::json!("reasoning_effort"))
        );
        assert_eq!(
            value.pointer("/1/currentValue"),
            Some(&serde_json::json!("high"))
        );

        let surfaced = serde_json::to_value(grok_cowboy_options(
            Some(&config),
            GrokPermissionMode::AlwaysApprove,
            None,
            None,
            None,
        ))
        .expect("serialize surfaced Grok options");
        assert_eq!(
            surfaced.pointer("/0/id"),
            Some(&serde_json::json!("permission_mode"))
        );
        assert_eq!(
            surfaced.pointer("/0/currentValue"),
            Some(&serde_json::json!("always-approve"))
        );
        assert_eq!(
            surfaced.pointer("/0/options/0/value"),
            Some(&serde_json::json!("default"))
        );
        assert_eq!(
            surfaced.pointer("/0/options/1/value"),
            Some(&serde_json::json!("auto"))
        );
        assert_eq!(
            surfaced.pointer("/0/options/2/value"),
            Some(&serde_json::json!("always-approve"))
        );
    }

    #[test]
    fn grok_permission_modes_use_the_native_extension_notification() {
        for (mode, expected) in [
            (
                GrokPermissionMode::Default,
                serde_json::json!({
                    "yolo_mode": false,
                    "auto_mode": false,
                    "permission_mode": "default",
                }),
            ),
            (
                GrokPermissionMode::Auto,
                serde_json::json!({
                    "yolo_mode": false,
                    "auto_mode": true,
                    "permission_mode": "auto",
                }),
            ),
            (
                GrokPermissionMode::AlwaysApprove,
                serde_json::json!({
                    "yolo_mode": true,
                    "auto_mode": false,
                    "permission_mode": "always-approve",
                }),
            ),
        ] {
            let notification = grok_permission_notification(mode);
            assert_eq!(notification.method(), "x.ai/yolo_mode_changed");
            assert_eq!(
                serde_json::to_value(notification).expect("serialize notification"),
                expected
            );
        }
        assert_eq!(
            GrokPermissionMode::parse("ask"),
            Some(GrokPermissionMode::Default)
        );
        assert_eq!(GrokPermissionMode::parse("plan"), None);
    }

    #[test]
    fn released_grok_initialize_metadata_supplies_model_and_effort_options() {
        let mut meta = agent_client_protocol::schema::v1::Meta::new();
        meta.insert(
            "modelState".to_owned(),
            serde_json::json!({
                "currentModelId": "grok-4.5",
                "availableModels": [
                    {
                        "modelId": "grok-4.5",
                        "name": "Grok 4.5",
                        "description": "Frontier model",
                        "_meta": {
                            "reasoningEffort": "high",
                            "reasoningEfforts": [
                                { "id": "high", "label": "High Effort", "default": true },
                                { "id": "medium", "label": "Medium Effort", "default": false }
                            ]
                        }
                    },
                    {
                        "modelId": "grok-fast",
                        "name": "Grok Fast",
                        "_meta": {
                            "reasoningEffort": "low",
                            "reasoningEfforts": [
                                { "id": "low", "label": "Low Effort", "default": true }
                            ]
                        }
                    },
                    {
                        "modelId": "grok-direct",
                        "name": "Grok Direct"
                    }
                ]
            }),
        );
        let mut config =
            GrokSessionConfig::from_metadata(None, Some(&meta)).expect("Grok model state");
        let value = serde_json::to_value(config.cowboy_options()).expect("serialize options");
        assert_eq!(
            value.pointer("/0/currentValue"),
            Some(&serde_json::json!("grok-4.5"))
        );
        assert_eq!(
            value.pointer("/1/currentValue"),
            Some(&serde_json::json!("high"))
        );
        assert!(config.select_model("grok-fast"));
        assert_eq!(config.selected("mode"), Some("low"));
        assert!(config.select_model("grok-direct"));
        assert_eq!(config.selected("mode"), None);
        assert_eq!(config.selected_wire_value("mode"), None);
        assert_eq!(config.cowboy_options().len(), 1);
    }

    #[test]
    fn grok_reasoning_menu_id_maps_to_the_canonical_wire_value() {
        let mut initialize_meta = agent_client_protocol::schema::v1::Meta::new();
        initialize_meta.insert(
            "modelState".to_owned(),
            serde_json::json!({
                "currentModelId": "grok-4.5",
                "availableModels": [{
                    "modelId": "grok-4.5",
                    "name": "Grok 4.5",
                    "_meta": {
                        "reasoningEffort": "xhigh",
                        "reasoningEfforts": [{
                            "id": "deep",
                            "value": "xhigh",
                            "label": "Deep",
                            "default": true
                        }]
                    }
                }]
            }),
        );
        let mut session_meta = agent_client_protocol::schema::v1::Meta::new();
        session_meta.insert(
            "x.ai/sessionConfig".to_owned(),
            serde_json::json!({ "options": [
                { "id": "grok-4.5", "category": "model", "label": "Grok 4.5", "selected": true },
                { "id": "deep", "category": "mode", "label": "Deep", "selected": true }
            ]}),
        );

        let config = GrokSessionConfig::from_metadata(Some(&session_meta), Some(&initialize_meta))
            .expect("Grok config");
        assert_eq!(config.selected("mode"), Some("deep"));
        assert_eq!(config.selected_wire_value("mode"), Some("xhigh"));

        let request = grok_model_request(
            SessionId::new("grok-session"),
            config.selected("model").expect("model").to_owned(),
            config.selected_wire_value("mode"),
        );
        let value = serde_json::to_value(request).expect("serialize Grok request");
        assert_eq!(value["_meta"]["reasoningEffort"], "xhigh");
    }

    #[test]
    fn grok_model_switch_uses_upstream_wire_method_and_carries_reasoning_effort() {
        let request = grok_model_request(
            SessionId::new("grok-session"),
            "grok-4.5".to_owned(),
            Some("high"),
        );
        let value = serde_json::to_value(&request).expect("serialize Grok request");
        assert_eq!(request.method(), "session/set_model");
        assert_eq!(value["sessionId"], "grok-session");
        assert_eq!(value["modelId"], "grok-4.5");
        assert_eq!(value["_meta"]["reasoningEffort"], "high");
    }

    #[test]
    fn grok_session_info_becomes_standard_context_usage() {
        let request = GrokSessionInfoRequest {
            session_id: SessionId::new("grok-session"),
        };
        assert_eq!(request.method(), "x.ai/session/info");
        assert_eq!(
            serde_json::to_value(request).expect("serialize Grok session info request"),
            serde_json::json!({ "sessionId": "grok-session" })
        );

        let response: GrokSessionInfoResponse = serde_json::from_value(serde_json::json!({
            "result": {
                "sessionId": "grok-session",
                "cwd": "/workspace",
                "model": "grok-4.6",
                "context": {
                    "used": 12_345,
                    "total": 131_072,
                    "usagePct": 9,
                    "freeTokens": 118_727
                }
            }
        }))
        .expect("deserialize released Grok session info response");
        let usage = grok_session_usage(&response, 1_234).expect("Grok context usage");
        assert_eq!(usage.used, 12_345);
        assert_eq!(usage.size, 131_072);
        assert_eq!(usage.observed_at_ms, 1_234);
        assert_eq!(
            usage.raw.pointer("/result/context/usagePct"),
            Some(&serde_json::json!(9))
        );

        let no_window: GrokSessionInfoResponse = serde_json::from_value(serde_json::json!({
            "result": { "context": { "used": 0, "total": 0 } }
        }))
        .expect("deserialize empty Grok session info response");
        assert!(grok_session_usage(&no_window, 1_234).is_none());
    }

    #[test]
    fn resume_prefers_no_replay_and_keeps_load_as_compatibility_fallback() {
        assert_eq!(select_resume_method(true, true), Some(ResumeMethod::Resume));
        assert_eq!(
            select_resume_method(true, false),
            Some(ResumeMethod::Resume)
        );
        assert_eq!(select_resume_method(false, true), Some(ResumeMethod::Load));
        assert_eq!(select_resume_method(false, false), None);
    }

    #[test]
    fn only_an_initialize_timeout_is_safe_to_retry() {
        let initialize = StartupTimeout::new(StartupPhase::Initialize);
        let resume = StartupTimeout::new(StartupPhase::Resume);

        assert!(initialize.retryable());
        assert!(!resume.retryable());
        assert_eq!(
            resume.to_string(),
            "agent did not complete ACP session/resume within 240s"
        );
    }

    #[test]
    fn only_the_adapter_synthetic_empty_stream_message_is_bufferable() {
        let synthetic = serde_json::json!({
            "sessionUpdate": "agent_message_chunk",
            "messageId": "adapter-error",
            "content": {
                "type": "text",
                "text": "API Error: Stream ended without receiving any events"
            }
        });
        assert!(is_empty_stream_message_update(&synthetic));
        assert!(!is_empty_stream_message_update(&serde_json::json!({
            "sessionUpdate": "agent_thought_chunk",
            "content": {
                "type": "text",
                "text": "API Error: Stream ended without receiving any events"
            }
        })));
        assert!(!is_empty_stream_message_update(&serde_json::json!({
            "sessionUpdate": "agent_message_chunk",
            "content": {"type": "text", "text": "real model output"}
        })));
        assert!(!is_empty_stream_message_update(&serde_json::json!({
            "sessionUpdate": "agent_message_chunk",
            "content": {
                "type": "text",
                "text": "The error API Error: Stream ended without receiving any events means the provider closed early."
            }
        })));
    }

    #[test]
    fn prompt_retry_observation_is_isolated_per_turn() {
        let first = ActivePrompt::new(true);
        let second = ActivePrompt::new(true);

        first
            .visible_update
            .store(true, std::sync::atomic::Ordering::SeqCst);
        first
            .capture
            .lock()
            .as_mut()
            .expect("first capture")
            .push_str("first");
        *first.pending_empty_stream_update.lock() = Some(serde_json::json!({"first": true}));

        assert!(
            !second
                .visible_update
                .load(std::sync::atomic::Ordering::SeqCst)
        );
        assert_eq!(second.capture.lock().as_deref(), Some(""));
        assert!(second.pending_empty_stream_update.lock().is_none());
        assert_eq!(first.capture.lock().as_deref(), Some("first"));
    }

    #[test]
    fn cancel_generation_invalidates_only_already_accepted_prompts() {
        let (cancellation, _) = tokio::sync::watch::channel(0_u64);
        let mut accepted_before_stop = cancellation.subscribe();
        let before = *accepted_before_stop.borrow_and_update();

        cancellation.send_modify(|generation| *generation = generation.wrapping_add(1));

        assert_ne!(*accepted_before_stop.borrow_and_update(), before);
        let mut accepted_after_stop = cancellation.subscribe();
        let after = *accepted_after_stop.borrow_and_update();
        assert_eq!(*cancellation.borrow(), after);
    }
}

/// One ACP startup phase did not complete within [`STARTUP_PHASE_TIMEOUT`].
/// Carried as an `anyhow` cause so [`run_agent_with_sink`] can retry only a
/// pre-initialize adapter stall, never an ambiguous session operation.
#[derive(Debug)]
struct StartupTimeout {
    phase: StartupPhase,
    seconds: u64,
}

impl StartupTimeout {
    const fn new(phase: StartupPhase) -> Self {
        Self {
            phase,
            seconds: startup_phase_timeout(phase).as_secs(),
        }
    }

    const fn retryable(&self) -> bool {
        matches!(self.phase, StartupPhase::Initialize)
    }
}

const fn startup_phase_timeout(phase: StartupPhase) -> Duration {
    if matches!(phase, StartupPhase::Resume) {
        // App Server must scan the native rollout before it can resume. Large
        // image-heavy threads can take longer than a normal ACP handshake even
        // when excludeTurns avoids serializing their history back to the adapter.
        RESUME_PHASE_TIMEOUT
    } else {
        STARTUP_PHASE_TIMEOUT
    }
}

impl std::fmt::Display for StartupTimeout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "agent did not complete ACP {} within {}s",
            self.phase.method(),
            self.seconds
        )
    }
}

impl std::error::Error for StartupTimeout {}

async fn startup_watchdog(mut phases: watch::Receiver<StartupPhase>) -> StartupTimeout {
    loop {
        let phase = *phases.borrow_and_update();
        if phase == StartupPhase::Ready {
            std::future::pending::<()>().await;
        }

        tokio::select! {
            () = tokio::time::sleep(startup_phase_timeout(phase)) => {
                return StartupTimeout::new(phase);
            }
            changed = phases.changed() => {
                if changed.is_err() {
                    std::future::pending::<()>().await;
                }
            }
        }
    }
}

fn codex_full_access_available(options: &[SessionConfigOption]) -> bool {
    options.iter().any(|option| {
        option.id.0.as_ref() == CODEX_FULL_ACCESS_CONFIG_ID
            && match &option.kind {
                SessionConfigKind::Select(select) => {
                    select.current_value.0.as_ref() != CODEX_FULL_ACCESS_CONFIG_VALUE
                        && config_select_options_contain(
                            &select.options,
                            CODEX_FULL_ACCESS_CONFIG_VALUE,
                        )
                }
                #[allow(unreachable_patterns)]
                _ => false,
            }
    })
}

fn codex_full_access_selected(options: &[SessionConfigOption]) -> bool {
    options.iter().any(|option| {
        option.id.0.as_ref() == CODEX_FULL_ACCESS_CONFIG_ID
            && match &option.kind {
                SessionConfigKind::Select(select) => {
                    select.current_value.0.as_ref() == CODEX_FULL_ACCESS_CONFIG_VALUE
                }
                #[allow(unreachable_patterns)]
                _ => false,
            }
    })
}

fn config_select_options_contain(options: &SessionConfigSelectOptions, value: &str) -> bool {
    match options {
        SessionConfigSelectOptions::Ungrouped(options) => options
            .iter()
            .any(|option| option.value.0.as_ref() == value),
        SessionConfigSelectOptions::Grouped(groups) => groups.iter().any(|group| {
            group
                .options
                .iter()
                .any(|option| option.value.0.as_ref() == value)
        }),
        #[allow(unreachable_patterns)]
        _ => false,
    }
}

fn session_config_value(
    value: &serde_json::Value,
) -> std::result::Result<SessionConfigOptionValue, &'static str> {
    match value {
        serde_json::Value::String(value) => Ok(SessionConfigOptionValue::value_id(value.clone())),
        serde_json::Value::Bool(value) => Ok(SessionConfigOptionValue::boolean(*value)),
        _ => Err("configuration values must be a string id or boolean"),
    }
}

async fn set_startup_config_option(
    cx: &ConnectionTo<Agent>,
    session_id: &str,
    acp_id: &SessionId,
    config_id: &str,
    value: &str,
) -> Option<serde_json::Value> {
    let req = SetSessionConfigOptionRequest::new(acp_id.clone(), config_id.to_owned(), value);
    match cx.send_request(req).block_task().await {
        Ok(resp) => match serde_json::to_value(&resp.config_options) {
            Ok(opts) => Some(opts),
            Err(e) => {
                tracing::warn!(
                    session = %session_id,
                    config_id,
                    value,
                    error = %e,
                    "serializing startup config options failed"
                );
                None
            }
        },
        Err(e) => {
            tracing::warn!(
                session = %session_id,
                config_id,
                value,
                error = ?e,
                "setting startup config option failed"
            );
            None
        }
    }
}

/// A command from a client, routed by the supervisor to an agent thread.
#[derive(Debug)]
pub enum AgentCommand {
    /// Send a user turn. The full ACP content array is forwarded to the
    /// upstream agent verbatim — image / audio / resource blocks make it
    /// through (subject to the upstream's own capabilities), not just text.
    /// The `Option<String>` is the originating client's cmid (chat send) used to
    /// tag the user-message echo for optimistic reconcile; None for none.
    Prompt(
        Vec<ContentBlock>,
        Option<String>,
        Option<oneshot::Sender<Result<String, String>>>,
    ),
    /// Cancel the current turn (ACP `session/cancel`).
    Cancel,
    /// Answer a pending permission request (`None` = cancelled / no choice).
    Permission {
        request_id: String,
        option_id: Option<String>,
    },
    /// Set one of the per-session config options the agent advertises
    /// (mode / model / effort / future). Forwarded to the upstream via the
    /// ACP `session/set_config_option` extension method. The agent's
    /// authoritative response carrying the refreshed options is pushed back
    /// into [`Hub`].
    SetConfigOption {
        config_id: String,
        value: serde_json::Value,
    },
}

/// Per-session client state shared by the connection's handler closures and the
/// command loop. All inhabit the crate's single executor, but the crate
/// requires `Send`, so this is `Arc` + `Mutex`/atomics (not `Rc`/`RefCell`).
struct ClientState {
    sink: Arc<dyn AgentSink>,
    session_id: String,
    provider_id: String,
    /// Pending permission requests awaiting a client answer, keyed by request
    /// id. The connection's permission handler inserts a sender; the command
    /// loop resolves exactly one (first-response-wins).
    pending: Mutex<HashMap<String, oneshot::Sender<Option<String>>>>,
    /// Serialize `session/prompt` RPCs while leaving the outer command loop
    /// responsive to Cancel, permission answers, and config changes.
    prompt_lock: tokio::sync::Mutex<()>,
    /// Notification handlers attach progress only to the prompt that currently
    /// owns `prompt_lock`. Queued prompts cannot reset or consume its state.
    active_prompt: Mutex<Option<Arc<ActivePrompt>>>,
    /// Every Cancel advances this generation. A prompt captures the current
    /// value when accepted, so Stop also cancels a retry waiting in backoff (or
    /// a prompt queued before Stop but not yet started).
    prompt_cancellation: watch::Sender<u64>,
    /// The Codex adapter's authoritative session mode is Full Access. Codex has
    /// occasionally emitted permission requests after `session/load` despite
    /// that mode (`approval_policy=never`); keep those upstream regressions
    /// from blocking an explicitly unrestricted Cowboy session. This is never
    /// enabled for a restricted mode or another provider.
    codex_full_access: AtomicBool,
    /// While `true`, incoming `session/update` notifications are dropped rather
    /// than pushed to the Hub. Set only around a `session/load` resume: the
    /// agent replays the whole prior conversation as updates, but cowboy's own
    /// persisted log is the source of truth and already holds that history —
    /// re-pushing it would duplicate every message. `load_session` is used
    /// purely to re-warm the agent's internal context, not to rebuild ours.
    suppress_updates: AtomicBool,
}

/// Retry and completion state belongs to one serialized prompt. Keeping it
/// outside [`ClientState`] prevents a later Prompt command from erasing the
/// progress that makes replaying the current prompt unsafe.
struct ActivePrompt {
    /// Assistant text captured for an internal prompt that requested a direct
    /// completion result. Ordinary UI prompts leave it off.
    capture: Mutex<Option<String>>,
    /// Set by any timeline-visible ACP update during the current prompt. A
    /// Claude empty-stream error is safe to retry only while this remains
    /// false: once text, thinking, or a tool update escaped the adapter, a
    /// replay could duplicate output or side effects.
    visible_update: AtomicBool,
    /// Claude's ACP adapter emits its empty-stream error as an agent message
    /// immediately before returning the same RPC error; buffer that synthetic
    /// message so a successful transparent retry does not leave a false error
    /// in history.
    pending_empty_stream_update: Mutex<Option<serde_json::Value>>,
}

impl ActivePrompt {
    fn new(capture_completion: bool) -> Self {
        Self {
            capture: Mutex::new(capture_completion.then(String::new)),
            visible_update: AtomicBool::new(false),
            pending_empty_stream_update: Mutex::new(None),
        }
    }
}

impl ClientState {
    fn current_prompt(&self) -> Option<Arc<ActivePrompt>> {
        self.active_prompt.lock().clone()
    }

    fn clear_prompt(&self, prompt: &Arc<ActivePrompt>) {
        let mut active = self.active_prompt.lock();
        if active
            .as_ref()
            .is_some_and(|candidate| Arc::ptr_eq(candidate, prompt))
        {
            active.take();
        }
    }
}

/// Detached-worker entry point. The ACP connection and all pending request
/// futures remain in this process; output crosses only the [`AgentSink`]
/// boundary, which can survive a Cowboy control-plane restart.
pub fn run_agent_with_sink(
    spec: &LaunchSpec,
    session_id: &str,
    cwd: PathBuf,
    resume: Option<String>,
    mut cmd_rx: mpsc::UnboundedReceiver<AgentCommand>,
    sink: &Arc<dyn AgentSink>,
) {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            sink.set_status(session_id, Status::Crashed, Some(format!("runtime: {e}")));
            return;
        }
    };
    // The whole connection (transport, handlers, `cx.spawn`ed tasks, command
    // loop) runs cooperatively inside the single `connect_with` future, so a
    // plain `block_on` suffices — no `LocalSet` needed now that the crate is
    // `Send`-based.
    //
    // Auto-retry once only when the adapter does not answer ACP initialize.
    // This covers a transient launch/runtime stall without replaying an
    // ambiguous session/resume, session/load, or session/new request. The queued
    // prompts are safe across the retry: they live in the Hub queue (pg) until
    // the session reaches `Running`, never in `cmd_rx`, so `cmd_rx` is empty here
    // — we keep it alive across both attempts only so the supervisor's sender
    // stays valid. A SECOND stall is a persistent failure → `Crashed` + the UI's
    // manual Retry (which routes back through revive).
    let result = rt.block_on(async {
        let mut result = agent_main(
            spec,
            session_id,
            cwd.clone(),
            resume.clone(),
            &mut cmd_rx,
            Arc::clone(sink),
        )
        .await;
        let retryable_startup_stall = result
            .as_ref()
            .err()
            .and_then(|e| e.downcast_ref::<StartupTimeout>())
            .is_some_and(StartupTimeout::retryable);
        if retryable_startup_stall {
            tracing::warn!(
                session = session_id,
                "ACP initialize stalled; auto-retrying spawn once"
            );
            // Stay in `Starting` (a spinner), not `Crashed`: this blip is
            // expected to self-heal, so don't flash an error for it.
            sink.set_status(
                session_id,
                Status::Starting,
                Some("agent slow to start — retrying…".to_owned()),
            );
            result = agent_main(spec, session_id, cwd, resume, &mut cmd_rx, Arc::clone(sink)).await;
        }
        result
    });
    match result {
        Ok(()) => sink.set_status(session_id, Status::Exited, None),
        Err(e) => {
            let raw_error = e.to_string();
            let detail = if spec.id == "gemini" {
                crate::provider::gemini::user_facing_startup_error(&raw_error)
                    .unwrap_or(&raw_error)
                    .to_owned()
            } else {
                raw_error.clone()
            };
            tracing::error!(session = session_id, error = %raw_error, "agent session ended with error");
            // Salvage un-consumed prompts. A cold-start / handshake failure returns
            // BEFORE the command loop (`while let Some(cmd) = cmd_rx.recv()`) drains
            // cmd_rx, so a prompt the dispatcher delivered to this (revived) agent is
            // still sitting here un-logged. Without this it dies with the thread —
            // the user's message vanishes from every surface (see
            // `Hub::requeue_prompt`). Put it back on the durable queue so it's
            // visible and re-drains once the session recovers. Cancel/Permission
            // commands are transient and intentionally dropped.
            while let Ok(cmd) = cmd_rx.try_recv() {
                if let AgentCommand::Prompt(blocks, cmid, completion) = cmd {
                    if let Some(tx) = completion {
                        let _ = tx.send(Err(format!("agent failed before prompt: {detail}")));
                        continue;
                    }
                    let content: Vec<serde_json::Value> = blocks
                        .iter()
                        .map(|b| serde_json::to_value(b).unwrap_or(serde_json::Value::Null))
                        .collect();
                    let text = content
                        .iter()
                        .filter_map(|v| v.get("text").and_then(serde_json::Value::as_str))
                        .collect::<Vec<_>>()
                        .join("\n");
                    sink.requeue_prompt(session_id, text, content, cmid);
                }
            }
            sink.set_status(session_id, Status::Crashed, Some(detail));
        }
    }
}

#[allow(clippy::too_many_lines)] // one cohesive spawn + connection + watchdog
async fn agent_main(
    spec: &LaunchSpec,
    session_id: &str,
    cwd: PathBuf,
    resume: Option<String>,
    cmd_rx: &mut mpsc::UnboundedReceiver<AgentCommand>,
    sink: Arc<dyn AgentSink>,
) -> Result<()> {
    let cwd =
        std::path::absolute(&cwd).with_context(|| format!("resolving cwd {}", cwd.display()))?;
    tracing::info!(provider = spec.id, session = session_id, cwd = %cwd.display(), "spawning agent");

    let mut command = Command::new(&spec.command);
    command.args(&spec.args);
    for (key, _) in std::env::vars_os() {
        if key
            .to_str()
            .is_some_and(|name| spec.removes_inherited_env(name))
        {
            command.env_remove(key);
        }
    }
    command.envs(&spec.env);
    if let Some((key, value)) = deepseek_session_environment(
        spec.id,
        session_id,
        spec.env.get("ANTHROPIC_CUSTOM_HEADERS").map(String::as_str),
        std::env::var(crate::deepseek_cache::SESSION_POLICY_ENV)
            .ok()
            .as_deref(),
    ) {
        command.env(key, value);
    }
    let mut child = command
        .current_dir(&cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("spawning provider {} ({})", spec.id, spec.command))?;

    // Contain the agent + everything it forks in its own cgroup so a wedged turn's
    // orphaned subprocesses (e.g. a detached `until …; do sleep; done` poll loop)
    // can be reaped wholesale on teardown / recycle. Best-effort: None ⇒ the agent
    // runs uncontained (see crate::cgroup). Done before the agent forks anything.
    let agent_cgroup = child.id().and_then(|pid| {
        let dir = cgroup::create(session_id)?;
        cgroup::add_pid(&dir, pid);
        Some(dir)
    });

    let child_stdin = child.stdin.take().context("child stdin")?;
    let child_stdout = child.stdout.take().context("child stdout")?;
    let child_stderr = child.stderr.take().context("child stderr")?;
    let stderr_session = session_id.to_owned();
    let stderr_provider = spec.id.to_owned();
    let stderr_task = tokio::spawn(async move {
        let mut lines = BufReader::new(child_stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::warn!(
                session = %stderr_session,
                provider = %stderr_provider,
                child = true,
                message = %line,
                "agent stderr"
            );
        }
    });

    // Connect the crate directly to the child's pipes. The 0.4-era custom
    // stdio interceptors are gone: `config_option_update` now decodes natively
    // (handled in the notification closure), and ext methods are sent with
    // their wire name verbatim (no `_`-prefix mangling to undo), so there is
    // nothing left to rewrite on either stream.
    let transport = ByteStreams::new(child_stdin.compat_write(), child_stdout.compat());

    let (prompt_cancellation, _) = watch::channel(0_u64);
    let state = Arc::new(ClientState {
        sink,
        session_id: session_id.to_owned(),
        provider_id: spec.id.to_owned(),
        pending: Mutex::new(HashMap::new()),
        prompt_lock: tokio::sync::Mutex::new(()),
        active_prompt: Mutex::new(None),
        prompt_cancellation,
        codex_full_access: AtomicBool::new(false),
        suppress_updates: AtomicBool::new(false),
    });

    let notif_state = state.clone();
    let perm_state = state.clone();
    let main_state = state.clone();

    // `run_session` advances this marker before every startup request. The
    // watchdog resets its deadline at each transition and pends permanently
    // once the session is Running.
    let (startup_phase, startup_progress) = watch::channel(StartupPhase::Initialize);
    let run_progress = startup_phase.clone();

    let conn = Client
        .builder()
        .name("cowboy")
        .on_receive_notification(
            async move |notif: SessionNotification,
                        _cx: ConnectionTo<Agent>|
                        -> Result<(), Error> {
                handle_session_notification(&notif_state, &notif);
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |req: RequestPermissionRequest,
                        responder,
                        cx: ConnectionTo<Agent>|
                        -> Result<(), Error> {
                // System sessions have no human to answer. Also absorb Codex's
                // known resume regression where it asks despite its
                // authoritative Full Access mode. Restricted user sessions
                // continue through the human permission path below.
                let system_session = perm_state.sink.session_is_system(&perm_state.session_id);
                let codex_full_access = perm_state.codex_full_access.load(Ordering::SeqCst);
                let tool_call =
                    serde_json::to_value(&req.tool_call).unwrap_or(serde_json::Value::Null);
                // A permission request proves the turn reached a tool boundary,
                // even when Full Access auto-answers it without a UI event.
                // Never replay that prompt as an empty-stream recovery.
                if let Some(prompt) = perm_state.current_prompt() {
                    prompt.visible_update.store(true, Ordering::SeqCst);
                }
                if system_session || codex_full_access {
                    let allow = req
                        .options
                        .iter()
                        .find(|o| matches!(o.kind, PermissionOptionKind::AllowAlways))
                        .or_else(|| {
                            req.options
                                .iter()
                                .find(|o| matches!(o.kind, PermissionOptionKind::AllowOnce))
                        });
                    let outcome = match allow {
                        Some(opt) => {
                            tracing::info!(
                                option = %opt.name,
                                session = %perm_state.session_id,
                                system_session,
                                codex_full_access,
                                "auto-approving permission"
                            );
                            RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
                                opt.option_id.clone(),
                            ))
                        }
                        None => RequestPermissionOutcome::Cancelled,
                    };
                    return responder.respond(RequestPermissionResponse::new(outcome));
                }
                // The SDK exposes the actual JSON-RPC request id. Keep its JSON
                // representation as Cowboy's opaque UI key so numeric `1` and
                // string `"1"` cannot collide and cancellation/response
                // correlation remains tied to the wire request.
                let request_id = serde_json::to_string(responder.id())
                    .unwrap_or_else(|_| responder.id().to_string());
                let options = serde_json::to_value(&req.options).unwrap_or(serde_json::Value::Null);

                let (tx, rx) = oneshot::channel::<Option<String>>();
                perm_state.pending.lock().insert(request_id.clone(), tx);
                perm_state.sink.push(
                    &perm_state.session_id,
                    Event::PermissionRequest {
                        request_id: request_id.clone(),
                        tool_call,
                        options,
                    },
                );

                // Defer the actual response: blocking the dispatch loop here
                // would stall every other incoming message (e.g. a concurrent
                // cancel) until the user answers. SDK 2 exposes JSON-RPC
                // request cancellation directly, so an upstream cancellation
                // also clears the pending Cowboy prompt immediately.
                let cancellation = responder.cancellation();
                let cancelled_state = Arc::clone(&perm_state);
                let cancelled_request_id = request_id.clone();
                cx.spawn(async move {
                    let chosen = tokio::select! {
                        chosen = rx => chosen.unwrap_or(None),
                        () = cancellation.cancelled() => {
                            cancelled_state.pending.lock().remove(&cancelled_request_id);
                            cancelled_state.sink.push(
                                &cancelled_state.session_id,
                                Event::PermissionResolved {
                                    request_id: cancelled_request_id,
                                    option_id: None,
                                },
                            );
                            responder.respond_with_error(Error::request_cancelled())?;
                            return Ok(());
                        }
                    };
                    let outcome = match chosen {
                        Some(option_id) => RequestPermissionOutcome::Selected(
                            SelectedPermissionOutcome::new(PermissionOptionId::new(option_id)),
                        ),
                        None => RequestPermissionOutcome::Cancelled,
                    };
                    responder.respond(RequestPermissionResponse::new(outcome))?;
                    Ok(())
                })?;
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(transport, async move |cx: ConnectionTo<Agent>| {
            run_session(&main_state, cx, resume, cwd, cmd_rx, spec.id, &run_progress).await
        });

    // Race the connection against the subprocess's OWN exit. The connection
    // future only resolves when `run_session` returns (its cmd channel closed),
    // so it cannot see the agent dying underneath it: if the agent streams a full
    // reply then exits BEFORE returning the turn's stop_reason, the in-flight
    // `prompt()` awaits a response that will never arrive — and a dead pipe does
    // not reliably surface as a request error — so the future hangs forever and
    // the session latches at `Busy`: a perpetual streaming caret + a queue that
    // never drains (the confirmed sess-stuck bug). `child.wait()` is the ground
    // truth the connection can miss; whichever finishes first ends the session,
    // and the hung turn is torn down with the dropped connection future. The Err
    // here lands as `Status::Crashed` in `run_agent` (queue holds; resend
    // revives). `biased` prefers a clean `run_session` return when both are ready.
    //
    // A THIRD ground truth `child.wait()` cannot catch: a live but wedged
    // adapter. The phase-aware watchdog also reports which request actually
    // stalled instead of attributing every startup timeout to process launch.
    let watchdog = startup_watchdog(startup_progress);
    let result = tokio::select! {
        biased;
        r = conn => r.map_err(|e| anyhow::anyhow!("acp connection: {e}")),
        status = child.wait() => Err(anyhow::anyhow!(
            "agent subprocess exited mid-session ({})",
            match status {
                Ok(s) => s.to_string(),
                Err(e) => format!("wait failed: {e}"),
            }
        )),
        timeout = watchdog => Err(anyhow::Error::new(timeout)),
    };

    // Keep the child alive for the whole connection; dropping it here lets the
    // agent see stdin EOF and exit (a no-op if it already exited above).
    drop(child);
    // Reap the whole agent subtree (the agent + any setsid-detached children it
    // leaked) and remove the leaf. Covers EVERY exit path — clean teardown, a
    // crashed/Exited race, or a watchdog hard-recycle that already SIGKILLed it.
    if let Some(dir) = &agent_cgroup {
        cgroup::kill_and_remove(dir);
    }
    stderr_task.abort();
    result
}

/// Translate one incoming agent `SessionUpdate` into a Hub event.
///
/// `config_option_update` is special-cased: rather than surfacing it as a
/// generic timeline update, its `configOptions` array is pushed to the Hub's
/// dedicated config-options channel (which hydrates the composer dropdowns).
/// Usage is kept as ephemeral session metadata. Every remaining variant is
/// passed through as serialized JSON (design §5), so the UI renders message /
/// thought chunks, tool calls, plans, and modes without per-variant re-modelling.
fn handle_session_notification(state: &ClientState, notif: &SessionNotification) {
    // During a `session/load` resume the agent replays prior turns; drop them
    // — cowboy already has this history persisted (see field docs).
    if state.suppress_updates.load(Ordering::SeqCst) {
        return;
    }
    if let SessionUpdate::ConfigOptionUpdate(ref update) = notif.update {
        match serde_json::to_value(&update.config_options) {
            Ok(opts) => state.sink.set_config_options(&state.session_id, opts),
            Err(e) => tracing::warn!(error = %e, "serializing config options"),
        }
        return;
    }
    let active_prompt = state.current_prompt();
    let synthetic_empty_stream = active_prompt.as_ref().is_some_and(|prompt| {
        crate::provider::is_claude(&state.provider_id)
            && !prompt.visible_update.load(Ordering::SeqCst)
            && matches!(
                &notif.update,
                SessionUpdate::AgentMessageChunk(chunk)
                    if matches!(
                    &chunk.content,
                    ContentBlock::Text(text)
                            if is_adapter_empty_stream_message(&text.text)
                    )
            )
    });
    if !synthetic_empty_stream
        && let SessionUpdate::AgentMessageChunk(ref chunk) = notif.update
        && let ContentBlock::Text(text) = &chunk.content
        && let Some(prompt) = &active_prompt
        && let Some(capture) = prompt.capture.lock().as_mut()
    {
        capture.push_str(&text.text);
    }
    if let SessionUpdate::UsageUpdate(ref usage) = notif.update {
        let raw = serde_json::to_value(&notif.update).unwrap_or(serde_json::Value::Null);
        state.sink.set_session_usage(
            &state.session_id,
            SessionUsage {
                used: usage.used,
                size: usage.size,
                raw,
                observed_at_ms: observed_at_ms(),
            },
        );
        return;
    }
    let update = match serde_json::to_value(&notif.update) {
        Ok(update) => update,
        Err(e) => {
            tracing::warn!(error = %e, "serializing session update");
            return;
        }
    };
    if synthetic_empty_stream && is_empty_stream_message_update(&update) {
        if let Some(prompt) = active_prompt {
            *prompt.pending_empty_stream_update.lock() = Some(update);
        }
        return;
    }
    if let Some(prompt) = active_prompt {
        prompt.visible_update.store(true, Ordering::SeqCst);
    }
    // Honor a ScheduleWakeup BEFORE pushing — the event is still stored
    // verbatim (timeline/UI unchanged); this just adds the side effect of
    // actually firing the wakeup, which the ACP runtime otherwise drops.
    maybe_arm_wakeup(state, &update);
    state.sink.push(&state.session_id, Event::Update { update });
}

fn is_empty_stream_message_update(update: &serde_json::Value) -> bool {
    update
        .get("sessionUpdate")
        .and_then(serde_json::Value::as_str)
        == Some("agent_message_chunk")
        && update
            .pointer("/content/type")
            .and_then(serde_json::Value::as_str)
            == Some("text")
        && update
            .pointer("/content/text")
            .and_then(serde_json::Value::as_str)
            .is_some_and(is_adapter_empty_stream_message)
}

fn is_adapter_empty_stream_message(text: &str) -> bool {
    text.trim() == CLAUDE_EMPTY_STREAM_MESSAGE
}

/// If `update` is a `ScheduleWakeup` tool call carrying `rawInput.{prompt,
/// delaySeconds}`, arm cowboy's scheduler. The agent expects its `/loop` runtime
/// to re-invoke it at the scheduled time; under ACP cowboy IS that runtime, so we
/// honor it here (see [`crate::scheduler`]). Only the event that carries the
/// input arms — the bare `tool_call` and the result `tool_call_update` lack it,
/// so this is effectively once per `ScheduleWakeup` call.
fn maybe_arm_wakeup(state: &ClientState, update: &serde_json::Value) {
    if update
        .pointer("/_meta/claudeCode/toolName")
        .and_then(serde_json::Value::as_str)
        != Some("ScheduleWakeup")
    {
        return;
    }
    let prompt = update
        .pointer("/rawInput/prompt")
        .and_then(serde_json::Value::as_str);
    let delay = update
        .pointer("/rawInput/delaySeconds")
        .and_then(serde_json::Value::as_i64);
    if let (Some(prompt), Some(delay)) = (prompt, delay)
        && !prompt.trim().is_empty()
    {
        tracing::info!(session = %state.session_id, delay_s = delay, "scheduler: arming ScheduleWakeup");
        state
            .sink
            .schedule_wakeup(&state.session_id, delay, prompt.to_owned());
    }
}

/// The connection's `main_fn`: run the ACP handshake, then a command loop that
/// drives prompts/cancels/permissions/config changes until the command channel
/// closes (the supervisor dropped the agent).
#[allow(clippy::too_many_lines)] // one cohesive handshake + command loop
async fn run_session(
    state: &Arc<ClientState>,
    cx: ConnectionTo<Agent>,
    resume: Option<String>,
    cwd: PathBuf,
    cmd_rx: &mut mpsc::UnboundedReceiver<AgentCommand>,
    provider_id: &str,
    startup_phase: &watch::Sender<StartupPhase>,
) -> Result<(), Error> {
    let session_id = state.session_id.clone();

    let init = cx
        .send_request(InitializeRequest::new(ProtocolVersion::V1))
        .block_task()
        .await?;
    let initialize_meta = init.meta.clone();
    let agent_can_load = init.agent_capabilities.load_session;
    let agent_can_resume = init
        .agent_capabilities
        .session_capabilities
        .resume
        .is_some();
    let resume_method = select_resume_method(agent_can_resume, agent_can_load);

    // Establish the agent session. Prefer `session/resume`: unlike
    // `session/load`, ACP defines it to restore native state without replaying
    // prior messages that Cowboy already persists. Retain `session/load` only
    // for older agents. A requested resume is strict: silently falling back to
    // session/new would preserve the Cowboy row while losing the native thread.
    let mut acp_id: Option<SessionId> = None;
    let mut modes = None;
    let mut session_meta = None;
    // Agents may return their initial config options (mode / model / effort) IN the
    // session-creation response (codex does this) rather than only via a later
    // `config_option_update` notification (claude does that). We capture + surface
    // both, so codex's Model / approval chips render like claude's.
    let mut config_options = None;
    if resume.is_some() && resume_method.is_none() {
        return Err(anyhow::anyhow!(
            "agent supports neither session/resume nor session/load; refusing to replace the existing native thread"
        )
        .into());
    }
    if let Some(resume_id) = resume {
        let resume_id = SessionId::new(resume_id.as_str());
        match resume_method.expect("resume support checked above") {
            ResumeMethod::Resume => {
                startup_phase.send_replace(StartupPhase::Resume);
                match cx
                    .send_request(resume_session_request(
                        provider_id,
                        resume_id.clone(),
                        cwd.clone(),
                    ))
                    .block_task()
                    .await
                {
                    Ok(resp) => {
                        tracing::info!(session = %session_id, acp_id = %resume_id.0, "session resumed via session/resume");
                        acp_id = Some(resume_id);
                        modes = resp.modes;
                        config_options = resp.config_options;
                        session_meta = resp.meta;
                    }
                    Err(e) => {
                        tracing::error!(session = %session_id, error = ?e, "session/resume failed; preserving native thread identity");
                        return Err(e);
                    }
                }
            }
            ResumeMethod::Load => {
                startup_phase.send_replace(StartupPhase::Load);
                state.suppress_updates.store(true, Ordering::SeqCst);
                let loaded = cx
                    .send_request(load_session_request(
                        provider_id,
                        resume_id.clone(),
                        cwd.clone(),
                    ))
                    .block_task()
                    .await;
                state.suppress_updates.store(false, Ordering::SeqCst);
                match loaded {
                    Ok(resp) => {
                        tracing::info!(session = %session_id, acp_id = %resume_id.0, "session resumed via session/load compatibility fallback");
                        acp_id = Some(resume_id);
                        modes = resp.modes;
                        config_options = resp.config_options;
                        session_meta = resp.meta;
                    }
                    Err(e) => {
                        tracing::error!(session = %session_id, error = ?e, "session/load failed; preserving native thread identity");
                        return Err(e);
                    }
                }
            }
        }
    }
    let acp_id = if let Some(id) = acp_id {
        id
    } else {
        startup_phase.send_replace(StartupPhase::New);
        let session = cx
            .send_request(new_session_request(provider_id, cwd.clone()))
            .block_task()
            .await?;
        // Persist the agent's own id so a future revive can resume this exact
        // conversation rather than opening a blank one.
        state
            .sink
            .set_agent_session_id(&session_id, session.session_id.0.to_string());
        tracing::info!(session = %session_id, acp_id = %session.session_id.0, "session created");
        modes = session.modes;
        config_options = session.config_options;
        session_meta = session.meta;
        session.session_id
    };
    startup_phase.send_replace(StartupPhase::Configure);
    if crate::provider::is_codex(provider_id) {
        state.codex_full_access.store(
            config_options
                .as_ref()
                .is_some_and(|opts| codex_full_access_selected(opts)),
            Ordering::SeqCst,
        );
    }
    // Codex ACP exposes its approval preset as a config option instead of a
    // session mode. Default new/revived Codex panels to Full Access when the
    // adapter advertises it; a failed set falls back to the adapter default.
    if crate::provider::is_codex(provider_id)
        && config_options
            .as_ref()
            .is_some_and(|opts| codex_full_access_available(opts))
        && let Some(updated_options) = set_startup_config_option(
            &cx,
            &session_id,
            &acp_id,
            CODEX_FULL_ACCESS_CONFIG_ID,
            CODEX_FULL_ACCESS_CONFIG_VALUE,
        )
        .await
    {
        tracing::info!(session = %session_id, "codex approval preset -> full access");
        state.codex_full_access.store(true, Ordering::SeqCst);
        state.sink.set_config_options(&session_id, updated_options);
        config_options = None;
    }

    // Open every provider at its own full-access session mode when advertised.
    // Codex exposes this as the config option handled above; Claude calls it
    // `bypassPermissions`, while Gemini calls the equivalent mode `yolo`.
    if let (Some(modes), Some(want)) = (modes.as_ref(), startup_full_access_mode(provider_id)) {
        let has = modes
            .available_modes
            .iter()
            .any(|m| m.id.0.as_ref() == want);
        if has && modes.current_mode_id.0.as_ref() != want {
            let req = SetSessionModeRequest::new(acp_id.clone(), SessionModeId::new(want));
            match cx.send_request(req).block_task().await {
                Ok(_) => {
                    tracing::info!(session = %session_id, mode = want, "startup mode -> full access");
                    // Echo into the timeline so the UI mode chip is up to date
                    // without round-tripping through a session_update.
                    state.sink.push(
                        &session_id,
                        Event::Update {
                            update: serde_json::json!({
                                "sessionUpdate": "current_mode_update",
                                "currentModeId": want,
                            }),
                        },
                    );
                }
                Err(e) => {
                    tracing::warn!(mode = want, error = ?e, "setting full-access startup mode failed");
                }
            }
        }
    }

    // Do not expose Running (which lets the broker drain queued prompts) until
    // the startup permission mode is authoritative.
    state.sink.set_status(&session_id, Status::Running, None);
    // Startup landed — disarm the phase watchdog (see `agent_main`).
    startup_phase.send_replace(StartupPhase::Ready);

    // Gemini (unlike codex) exposes its APPROVAL options as session MODES
    // (`availableModes` + `session/set_mode`), NOT config_options — so the codex
    // push above renders nothing for it. Translate those modes into a synthetic
    // "mode" select chip (matching Zed, which surfaces `session_modes` as its own
    // selector) so gemini gets an Approval dropdown like the others. Gated to
    // gemini: claude ALSO ships session modes, but advertises its mode as a real
    // `config_option` (via a later notification) that this must not shadow. The
    // chip's SET is routed to `session/set_mode` in the command loop below —
    // gemini implements no `session/set_config_option`. `Some` here also marks the
    // session as mode-via-session-modes for that routing.
    // Grok also has ordinary ACP session modes, but its model + reasoning
    // choices arrive in `x.ai/sessionConfig`; use a distinct id so both menus
    // remain independently selectable.
    let mode_config_id = match provider_id {
        "gemini" => Some("mode"),
        "grok" => Some(GROK_SESSION_MODE_CONFIG_ID),
        _ => None,
    };
    let current_session_mode = Arc::new(Mutex::new(
        modes
            .as_ref()
            .map(|mode| mode.current_mode_id.0.to_string()),
    ));
    // Grok workers launch with `--always-approve`; persisted Cowboy
    // preferences replay after this option is surfaced and can narrow the
    // resident session to Auto or Default without restarting the process.
    let current_permission_mode = Arc::new(Mutex::new(GrokPermissionMode::AlwaysApprove));
    let mode_select: Option<Vec<SessionConfigSelectOption>> = if mode_config_id.is_some() {
        modes
            .as_ref()
            .filter(|m| !m.available_modes.is_empty())
            .map(|m| {
                m.available_modes
                    .iter()
                    .map(|md| SessionConfigSelectOption::new(md.id.0.to_string(), md.name.clone()))
                    .collect()
            })
    } else {
        None
    };
    let grok_config = Arc::new(Mutex::new(
        (provider_id == "grok")
            .then(|| {
                GrokSessionConfig::from_metadata(session_meta.as_ref(), initialize_meta.as_ref())
            })
            .flatten(),
    ));
    let mut surfaced_options = config_options.unwrap_or_default();
    if provider_id == "grok" {
        let config = grok_config.lock().clone();
        let session_mode = current_session_mode.lock().clone();
        surfaced_options.extend(grok_cowboy_options(
            config.as_ref(),
            *current_permission_mode.lock(),
            mode_config_id,
            mode_select.as_deref(),
            session_mode.as_deref(),
        ));
    } else if let (Some(config_id), Some(options), Some(m)) =
        (mode_config_id, mode_select.as_ref(), modes.as_ref())
    {
        surfaced_options.push(SessionConfigOption::select(
            config_id,
            "Mode",
            m.current_mode_id.0.to_string(),
            options.clone(),
        ));
    }
    // Surface every startup option in one authoritative array. This includes
    // standard config options (Codex), synthesized ACP modes (Gemini/Grok), and
    // Grok's pre-standard model/effort metadata.
    if !surfaced_options.is_empty() {
        match serde_json::to_value(&surfaced_options) {
            Ok(v) => state.sink.set_config_options(&session_id, v),
            Err(e) => tracing::warn!(error = %e, "serializing startup config options"),
        }
    }

    let grok_usage_tx = if provider_id == "grok" {
        let (tx, rx) = mpsc::unbounded_channel();
        let usage_cx = cx.clone();
        let usage_sink = Arc::clone(&state.sink);
        let usage_session_id = session_id.clone();
        let usage_acp_id = acp_id.clone();
        cx.clone().spawn(async move {
            run_grok_usage_refresh_queue(usage_cx, usage_sink, usage_session_id, usage_acp_id, rx)
                .await
        })?;
        // Populate a resumed/new session without waiting for another model
        // turn. The request is adapter-local and does not consume quota.
        let _ = tx.send(());
        Some(tx)
    } else {
        None
    };

    let grok_config_tx = if provider_id == "grok" {
        let (tx, rx) = mpsc::unbounded_channel();
        let queue_cx = cx.clone();
        let queue_sink = Arc::clone(&state.sink);
        let queue_session_id = session_id.clone();
        let queue_acp_id = acp_id.clone();
        let queue_config = Arc::clone(&grok_config);
        let queue_permission_mode = Arc::clone(&current_permission_mode);
        let queue_session_mode = Arc::clone(&current_session_mode);
        let queue_mode_select = mode_select.clone();
        let queue_usage_refresh = grok_usage_tx
            .as_ref()
            .expect("Grok usage queue initialized")
            .clone();
        cx.clone().spawn(async move {
            run_grok_config_queue(
                queue_cx,
                queue_sink,
                queue_session_id,
                queue_acp_id,
                queue_config,
                queue_permission_mode,
                queue_session_mode,
                mode_config_id,
                queue_mode_select,
                queue_usage_refresh,
                rx,
            )
            .await
        })?;
        Some(tx)
    } else {
        None
    };

    // Command loop. Prompt work runs in spawned tasks so Cancel and Permission
    // answers remain responsive; `prompt_lock` serializes the actual prompt
    // RPCs. Config changes may still run concurrently with a turn.
    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            AgentCommand::Prompt(blocks, cmid, completion) => {
                state.sink.set_status(&session_id, Status::Busy, None);
                // Echo each user content block into the timeline so every
                // client (Web UI, phone, native shell) sees it — the upstream
                // agent may not stream a user_message_chunk back. One Hub event
                // per block so each renders as its own bubble. The FIRST echo
                // carries the originating client's cmid so that client reconciles
                // its optimistic chat bubble by id (the rest are untagged).
                // A daemon-originated turn — an auto-resume continuation (cmid
                // "__cont__…") or a fired ScheduleWakeup ("__wake__…") — is flagged
                // on the echo (persisted in the payload) so the UI renders it as a
                // distinct "↻ resumed turn" note: it isn't something the user
                // typed, so it must never look like a user bubble (e.g. a wakeup
                // re-issues a self-check prompt the user never sent).
                let auto_resumed = cmid.as_deref().is_some_and(|c| {
                    c.starts_with(AUTO_CONTINUE_PREFIX)
                        || c.starts_with(WAKEUP_PREFIX)
                        || c.starts_with(SCHED_PREFIX)
                });
                for (i, block) in blocks.iter().enumerate() {
                    let content = serde_json::to_value(block).unwrap_or(serde_json::Value::Null);
                    let tag = if i == 0 { cmid.clone() } else { None };
                    let mut update = serde_json::json!({
                        "sessionUpdate": "user_message_chunk",
                        "content": content,
                    });
                    if auto_resumed {
                        update["autoResumed"] = serde_json::Value::Bool(true);
                    }
                    state
                        .sink
                        .push_tagged(&session_id, Event::Update { update }, tag);
                }
                let cx = cx.clone();
                let sink = Arc::clone(&state.sink);
                let sid = session_id.clone();
                let acp = acp_id.clone();
                let state = Arc::clone(state);
                let provider = provider_id.to_owned();
                let usage_refresh = grok_usage_tx.clone();
                let capture_completion = completion.is_some();
                let mut cancellation = state.prompt_cancellation.subscribe();
                let cancellation_generation = *cancellation.borrow_and_update();
                cx.clone().spawn(async move {
                    let _prompt_guard = state.prompt_lock.lock().await;
                    if *cancellation.borrow_and_update() != cancellation_generation {
                        if let Some(tx) = completion {
                            let _ = tx.send(Err("prompt cancelled before it started".to_owned()));
                        }
                        sink.push(
                            &sid,
                            Event::TurnEnd {
                                stop_reason: "Cancelled".to_owned(),
                            },
                        );
                        sink.set_status(&sid, Status::Running, None);
                        return Ok(());
                    }
                    // A queued prompt may acquire the lock just after the prior
                    // turn reported Running. Reassert Busy before its RPC.
                    sink.set_status(&sid, Status::Busy, None);
                    let prompt = Arc::new(ActivePrompt::new(capture_completion));
                    *state.active_prompt.lock() = Some(Arc::clone(&prompt));
                    let mut retries = 0;
                    let mut cancelled_during_retry = false;
                    let response = loop {
                        let request =
                            cx.send_request(PromptRequest::new(acp.clone(), blocks.clone()));
                        // `send_request` synchronously queues the JSON-RPC
                        // request. Recheck immediately afterwards: if Stop won
                        // the tiny check/send race, queue both cancellations
                        // behind this exact request instead of letting an idle
                        // session/cancel get consumed before the retry exists.
                        if *cancellation.borrow_and_update() != cancellation_generation {
                            cancelled_during_retry = true;
                            let _ = request.cancel();
                            let _ = cx.send_notification(CancelNotification::new(acp.clone()));
                        }
                        let response = request.block_task().await;
                        if *cancellation.borrow_and_update() != cancellation_generation {
                            cancelled_during_retry = true;
                            break response;
                        }
                        let Err(error) = &response else {
                            break response;
                        };
                        let detail = error.to_string();
                        let visible_update = prompt.visible_update.load(Ordering::SeqCst);
                        if !crate::provider::claude_code::should_retry_empty_stream(
                            &provider,
                            &detail,
                            visible_update,
                            retries,
                        ) {
                            break response;
                        }
                        retries += 1;
                        tracing::warn!(
                            session = %sid,
                            provider = %provider,
                            attempt = retries + 1,
                            "Claude stream ended before its first event; retrying prompt once"
                        );
                        tokio::select! {
                            () = tokio::time::sleep(Duration::from_millis(400)) => {}
                            changed = cancellation.changed() => {
                                if changed.is_ok() {
                                    cancelled_during_retry = true;
                                }
                            }
                        }
                        if cancelled_during_retry
                            || *cancellation.borrow_and_update() != cancellation_generation
                        {
                            cancelled_during_retry = true;
                            break response;
                        }
                        // A notification queued just before the failed response
                        // may be dispatched during the backoff. Abort the replay
                        // if it made any user-visible progress in that window.
                        if prompt.visible_update.load(Ordering::SeqCst) {
                            break response;
                        }
                    };
                    state.clear_prompt(&prompt);
                    if cancelled_during_retry {
                        prompt.pending_empty_stream_update.lock().take();
                        if let Some(tx) = completion {
                            let _ = tx.send(Err("prompt cancelled".to_owned()));
                        }
                        sink.push(
                            &sid,
                            Event::TurnEnd {
                                stop_reason: "Cancelled".to_owned(),
                            },
                        );
                        sink.set_status(&sid, Status::Running, None);
                        return Ok(());
                    }
                    match response {
                        Ok(r) => {
                            let pending_update = prompt.pending_empty_stream_update.lock().take();
                            if retries == 0
                                && let Some(update) = pending_update
                            {
                                // A successful first attempt means this was
                                // genuine model text that happened to match the
                                // diagnostic, not the adapter's error prelude.
                                sink.push(&sid, Event::Update { update });
                            }
                            if let Some(tx) = completion {
                                let text = prompt.capture.lock().take().unwrap_or_default();
                                let _ = tx.send(Ok(text));
                            }
                            // Turn completed — including a `Cancelled` from the user's manual
                            // Stop or a force-push (an Ok we WANT to drain). Going Running lets
                            // the auto-drain send the next queued prompt.
                            sink.push(
                                &sid,
                                Event::TurnEnd {
                                    stop_reason: format!("{:?}", r.stop_reason),
                                },
                            );
                            sink.set_status(&sid, Status::Running, None);
                            if usage_refresh
                                .as_ref()
                                .is_some_and(|refresh| refresh.send(()).is_err())
                            {
                                tracing::debug!(
                                    session = %sid,
                                    "Grok usage refresh queue closed after prompt"
                                );
                            }
                        }
                        Err(e) => {
                            let detail = e.to_string();
                            let pending_update = prompt.pending_empty_stream_update.lock().take();
                            if let Some(update) = pending_update {
                                sink.push(&sid, Event::Update { update });
                            }
                            if let Some(tx) = completion {
                                prompt.capture.lock().take();
                                let _ = tx.send(Err(detail.clone()));
                            }
                            // Context rejection and a zero-event stream failure
                            // are failed TURNS, not failed workers. The adapter
                            // remains connected, so retain it while the controller
                            // keeps the session in an actionable error state.
                            let worker_alive = crate::provider::claude_code::keeps_worker_alive(
                                &provider, &detail,
                            );
                            if worker_alive {
                                tracing::warn!(
                                    session = %sid,
                                    provider = %provider,
                                    "provider rejected a recoverable turn; keeping ACP worker alive"
                                );
                            }
                            // Every other prompt failure can include an agent/connection
                            // failure, including the subprocess dying mid-turn (surfaced by
                            // agent_main's child.wait() race). Mark those Crashed so a
                            // resend/open replaces the dead worker.
                            //
                            // We deliberately do NOT auto-detect a live-but-silent wedge: idle
                            // time can't tell a slow turn from a stuck one (Zed, the ACP author,
                            // reaches the same conclusion). The UI surfaces silence as a
                            // "waiting Xm" indicator and the user recovers MANUALLY via Stop
                            // (→ Cancel → the agent yields here as an Ok). No auto-kill.
                            sink.push(
                                &sid,
                                Event::TurnEnd {
                                    stop_reason: format!("error: {detail}"),
                                },
                            );
                            sink.set_status(
                                &sid,
                                if worker_alive {
                                    Status::Running
                                } else {
                                    Status::Crashed
                                },
                                Some(detail),
                            );
                        }
                    }
                    Ok(())
                })?;
            }
            AgentCommand::Cancel => {
                state
                    .prompt_cancellation
                    .send_modify(|generation| *generation = generation.wrapping_add(1));
                let _ = cx.send_notification(CancelNotification::new(acp_id.clone()));
            }
            AgentCommand::Permission {
                request_id,
                option_id,
            } => {
                if let Some(tx) = state.pending.lock().remove(&request_id) {
                    let _ = tx.send(option_id.clone());
                }
                state.sink.push(
                    &session_id,
                    Event::PermissionResolved {
                        request_id,
                        option_id,
                    },
                );
            }
            AgentCommand::SetConfigOption { config_id, value }
                if mode_config_id == Some(config_id.as_str()) && mode_select.is_some() =>
            {
                // gemini's synthesized "mode" chip maps to ACP `session/set_mode` —
                // it implements no `session/set_config_option` (it never advertised
                // config options; cowboy built this chip from its session modes).
                let Some(mode_id) = value.as_str().map(str::to_owned) else {
                    tracing::warn!(?value, "set mode: non-string value");
                    continue;
                };
                let cx = cx.clone();
                let sink = Arc::clone(&state.sink);
                let sid = session_id.clone();
                let acp = acp_id.clone();
                let options = mode_select.clone().unwrap_or_default();
                let grok_config = Arc::clone(&grok_config);
                let current_permission_mode = Arc::clone(&current_permission_mode);
                let current_session_mode = Arc::clone(&current_session_mode);
                let mode_config_id = mode_config_id.expect("synthesized mode id");
                let is_grok = provider_id == "grok";
                cx.clone().spawn(async move {
                    let req = SetSessionModeRequest::new(acp, SessionModeId::new(mode_id.clone()));
                    match cx.send_request(req).block_task().await {
                        Ok(_) => {
                            *current_session_mode.lock() = Some(mode_id.clone());
                            // Re-push the chip with the new current so the dropdown
                            // sticks (gemini emits no current_mode_update for an
                            // explicit set).
                            let config = grok_config.lock().clone();
                            let published = if is_grok {
                                grok_cowboy_options(
                                    config.as_ref(),
                                    *current_permission_mode.lock(),
                                    Some(mode_config_id),
                                    Some(&options),
                                    Some(&mode_id),
                                )
                            } else {
                                vec![SessionConfigOption::select(
                                    mode_config_id,
                                    "Mode",
                                    mode_id,
                                    options,
                                )]
                            };
                            match serde_json::to_value(published) {
                                Ok(v) => sink.set_config_options(&sid, v),
                                Err(e) => {
                                    tracing::warn!(error = %e, "re-serializing synthesized mode options");
                                }
                            }
                        }
                        Err(e) => sink.broadcast_error(Some(sid.clone()), format!("set mode: {e}")),
                    }
                    Ok(())
                })?;
            }
            AgentCommand::SetConfigOption { config_id, value }
                if provider_id == "grok" && config_id == GROK_PERMISSION_CONFIG_ID =>
            {
                let Some(requested) = value.as_str() else {
                    state.sink.broadcast_error(
                        Some(session_id.clone()),
                        format!("set {config_id}: configuration value must be a string id"),
                    );
                    continue;
                };
                let Some(permission_mode) = GrokPermissionMode::parse(requested) else {
                    state.sink.broadcast_error(
                        Some(session_id.clone()),
                        format!("set {config_id}: unsupported Grok permission mode {requested:?}"),
                    );
                    continue;
                };
                match cx.send_notification(grok_permission_notification(permission_mode)) {
                    Ok(()) => {
                        *current_permission_mode.lock() = permission_mode;
                        let config = grok_config.lock().clone();
                        let session_mode = current_session_mode.lock().clone();
                        let published = grok_cowboy_options(
                            config.as_ref(),
                            permission_mode,
                            mode_config_id,
                            mode_select.as_deref(),
                            session_mode.as_deref(),
                        );
                        match serde_json::to_value(published) {
                            Ok(options) => state.sink.set_config_options(&session_id, options),
                            Err(error) => {
                                tracing::warn!(error = %error, "serializing Grok permission options");
                            }
                        }
                    }
                    Err(error) => state.sink.broadcast_error(
                        Some(session_id.clone()),
                        format!("set {config_id}: {error}"),
                    ),
                }
            }
            AgentCommand::SetConfigOption { config_id, value }
                if provider_id == "grok"
                    && matches!(
                        config_id.as_str(),
                        GROK_MODEL_CONFIG_ID | GROK_REASONING_CONFIG_ID
                    ) =>
            {
                let Some(requested) = value.as_str().map(str::to_owned) else {
                    state.sink.broadcast_error(
                        Some(session_id.clone()),
                        format!("set {config_id}: configuration value must be a string id"),
                    );
                    continue;
                };
                let Some(tx) = grok_config_tx.as_ref() else {
                    state.sink.broadcast_error(
                        Some(session_id.clone()),
                        "Grok configuration queue is unavailable".to_owned(),
                    );
                    continue;
                };
                if tx
                    .send(GrokConfigChange {
                        config_id: config_id.clone(),
                        requested,
                    })
                    .is_err()
                {
                    state.sink.broadcast_error(
                        Some(session_id.clone()),
                        format!("set {config_id}: Grok configuration queue closed"),
                    );
                }
            }
            AgentCommand::SetConfigOption { config_id, value } => {
                // claude-agent-acp ≥ 0.31 handles mode / model / effort all
                // through the same `session/set_config_option` request. The
                // agent acks with the refreshed `configOptions` array; pushing
                // it back into Hub keeps the composer dropdowns in sync even
                // when the upstream chose a different value than we asked for
                // (e.g. `model=default` resets effort to its model's default).
                let cx = cx.clone();
                let sink = Arc::clone(&state.sink);
                let sid = session_id.clone();
                let acp = acp_id.clone();
                let state = Arc::clone(state);
                cx.clone().spawn(async move {
                    let config_value = match session_config_value(&value) {
                        Ok(value) => value,
                        Err(e) => {
                            sink.broadcast_error(
                                Some(sid.clone()),
                                format!("set {config_id}: {e}"),
                            );
                            return Ok(());
                        }
                    };
                    let req =
                        SetSessionConfigOptionRequest::new(acp, config_id.clone(), config_value);
                    match cx.send_request(req).block_task().await {
                        Ok(response) => {
                            if config_id == CODEX_FULL_ACCESS_CONFIG_ID {
                                let selected = codex_full_access_selected(&response.config_options);
                                state.codex_full_access.store(selected, Ordering::SeqCst);
                            }
                            match serde_json::to_value(response.config_options) {
                                Ok(options) => sink.set_config_options(&sid, options),
                                Err(e) => {
                                    tracing::warn!(
                                        error = %e,
                                        "serializing set config response"
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            sink.broadcast_error(
                                Some(sid.clone()),
                                format!("set {config_id}: {e}"),
                            );
                        }
                    }
                    Ok(())
                })?;
            }
        }
    }
    Ok(())
}

/// Spawn `spec`'s adapter, run the full ACP handshake, send one `prompt` in a
/// fresh session under `cwd`, and stream updates to stdout. Used by the
/// `try-agent` debug command to verify a provider end-to-end. Auto-approves the
/// first allow-style permission option.
pub async fn run_oneshot(spec: &LaunchSpec, cwd: PathBuf, prompt: String) -> Result<()> {
    let cwd =
        std::path::absolute(&cwd).with_context(|| format!("resolving cwd {}", cwd.display()))?;
    tracing::info!(provider = spec.id, cwd = %cwd.display(), "spawning agent");

    let mut command = Command::new(&spec.command);
    command.args(&spec.args);
    for (key, _) in std::env::vars_os() {
        if key
            .to_str()
            .is_some_and(|name| spec.removes_inherited_env(name))
        {
            command.env_remove(key);
        }
    }
    command.envs(&spec.env);
    let mut child = command
        .current_dir(&cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("spawning provider {} ({})", spec.id, spec.command))?;

    let child_stdin = child.stdin.take().context("child stdin")?;
    let child_stdout = child.stdout.take().context("child stdout")?;
    let transport = ByteStreams::new(child_stdin.compat_write(), child_stdout.compat());
    let provider_id = spec.id;

    let result = Client
        .builder()
        .name("cowboy-oneshot")
        .on_receive_notification(
            async move |notif: SessionNotification,
                        _cx: ConnectionTo<Agent>|
                        -> Result<(), Error> {
                use std::io::Write as _;
                match notif.update {
                    SessionUpdate::AgentMessageChunk(chunk)
                    | SessionUpdate::AgentThoughtChunk(chunk) => {
                        if let ContentBlock::Text(t) = chunk.content {
                            print!("{}", t.text);
                            let _ = std::io::stdout().flush();
                        }
                    }
                    SessionUpdate::ToolCall(tc) => eprintln!("\n[tool-call] {}", tc.title),
                    other => tracing::debug!(?other, "session update"),
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |req: RequestPermissionRequest,
                        responder,
                        _cx: ConnectionTo<Agent>|
                        -> Result<(), Error> {
                let allow = req.options.iter().find(|o| {
                    matches!(
                        o.kind,
                        PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways
                    )
                });
                let outcome = match allow {
                    Some(opt) => {
                        tracing::info!(option = %opt.name, "auto-approving permission (try-agent)");
                        RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
                            opt.option_id.clone(),
                        ))
                    }
                    None => RequestPermissionOutcome::Cancelled,
                };
                responder.respond(RequestPermissionResponse::new(outcome))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(transport, async move |cx: ConnectionTo<Agent>| {
            cx.send_request(InitializeRequest::new(ProtocolVersion::V1))
                .block_task()
                .await?;

            let session = cx
                .send_request(new_session_request(provider_id, cwd.clone()))
                .block_task()
                .await?;
            tracing::info!(session_id = %session.session_id.0, "session created");

            let resp = cx
                .send_request(PromptRequest::new(
                    session.session_id,
                    vec![ContentBlock::from(prompt)],
                ))
                .block_task()
                .await?;

            println!("\n--- stop: {:?} ---", resp.stop_reason);
            Ok(())
        })
        .await;

    drop(child);
    result.map_err(|e| anyhow::anyhow!("acp connection: {e}"))
}
