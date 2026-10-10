//! Cowboy-owned agent tools: defaults per agent kind and per-session
//! overrides. Agent calls are enforced here, at the Controller, before any
//! child is admitted; an agent's instructions can neither enable nor widen
//! them. Settings are UI-edited runtime state in the durable settings table.

use std::collections::BTreeSet;
use std::sync::Arc;

use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{AppState, AuthenticatedProductRequest, session_is_visible};
use crate::core::settings_keys::{AGENT_TOOLS_PREFIX, SESSION_TOOLS_PREFIX};

pub(super) const SCHEMA: u16 = 1;
/// Agents a managed call can start. Each must declare the read-only profile.
pub(super) const CALL_TARGETS: [&str; 2] = ["codex", "claude-code"];
pub(super) const AUTO: &str = "auto";

/// Agents of one family (`claude-code`, `claude-deepseek`; `codex`,
/// `codex-deepseek`) review alike, so an agent never calls its own family.
pub(super) fn same_family(left: &str, right: &str) -> bool {
    fn family(agent: &str) -> &str {
        ["claude", "codex"]
            .into_iter()
            .find(|family| agent == *family || agent.starts_with(&format!("{family}-")))
            .unwrap_or(agent)
    }
    family(left) == family(right)
}

/// The call targets an agent of `caller`'s kind may use at all.
fn callable(caller: &str) -> impl Iterator<Item = &'static str> + '_ {
    CALL_TARGETS
        .into_iter()
        .filter(move |target| !same_family(target, caller))
}
const MAX_TARGETS: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AgentTools {
    pub schema: u16,
    pub matrix: MatrixTools,
    pub calls: CallsPolicy,
}

/// Matrix memory: its MCP tools and the recall injected before each turn are
/// separate costs, so they are separate switches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MatrixTools {
    pub tools: bool,
    pub recall: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CallsPolicy {
    pub enabled: bool,
    /// Agents this session may call, in Auto's preference order.
    pub targets: Vec<CallTarget>,
    /// `auto` or one of the targets' agents.
    pub default: String,
    pub max_concurrent: u16,
    pub max_per_session: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CallTarget {
    pub agent: String,
    /// A configuration preset of the agent's signed package; none keeps its
    /// own default model and reasoning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
}

/// A session's changes to its agent kind's defaults. Absent means inherited.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SessionTools {
    pub schema: u16,
    #[serde(default)]
    pub matrix: MatrixOverride,
    #[serde(default)]
    pub calls: CallsOverride,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MatrixOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recall: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CallsOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub targets: Option<Vec<CallTarget>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

fn valid_agent(agent: &str) -> bool {
    !agent.is_empty()
        && agent.len() <= 64
        && agent
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn valid_targets(targets: &[CallTarget]) -> bool {
    let mut agents = BTreeSet::new();
    targets.len() <= MAX_TARGETS
        && targets.iter().all(|target| {
            CALL_TARGETS.contains(&target.agent.as_str())
                && agents.insert(target.agent.clone())
                && target
                    .preset
                    .as_ref()
                    .is_none_or(|preset| !preset.is_empty() && preset.len() <= 128)
        })
}

fn valid_default(default: &str, targets: &[CallTarget]) -> bool {
    default == AUTO || targets.iter().any(|target| target.agent == default)
}

impl AgentTools {
    /// Matrix stays on; agent calls are off until a session needs them. Every
    /// agent may call either agent, and Auto prefers the other one.
    pub fn builtin(agent: &str) -> Self {
        let targets = callable(agent)
            .map(|target| CallTarget {
                agent: target.to_owned(),
                preset: None,
            })
            .collect();
        Self {
            schema: SCHEMA,
            matrix: MatrixTools {
                tools: true,
                recall: true,
            },
            calls: CallsPolicy {
                enabled: false,
                targets,
                default: AUTO.to_owned(),
                max_concurrent: 4,
                max_per_session: 64,
            },
        }
    }

    pub fn validate(&self) -> bool {
        self.schema == SCHEMA
            && valid_targets(&self.calls.targets)
            && valid_default(&self.calls.default, &self.calls.targets)
            && (1..=16).contains(&self.calls.max_concurrent)
            && (1..=1000).contains(&self.calls.max_per_session)
    }

    /// Drop targets of the caller's own family (stored before this rule, or
    /// written by hand); a default naming one falls back to Auto.
    pub fn for_caller(mut self, caller: &str) -> Self {
        self.calls
            .targets
            .retain(|target| !same_family(&target.agent, caller));
        if !valid_default(&self.calls.default, &self.calls.targets) {
            AUTO.clone_into(&mut self.calls.default);
        }
        self
    }

    /// Apply a session's override. The result is validated by the caller.
    pub fn overlay(mut self, session: &SessionTools) -> Self {
        if let Some(tools) = session.matrix.tools {
            self.matrix.tools = tools;
        }
        if let Some(recall) = session.matrix.recall {
            self.matrix.recall = recall;
        }
        if let Some(enabled) = session.calls.enabled {
            self.calls.enabled = enabled;
        }
        if let Some(targets) = &session.calls.targets {
            self.calls.targets.clone_from(targets);
        }
        if let Some(default) = &session.calls.default {
            self.calls.default.clone_from(default);
        }
        if !valid_default(&self.calls.default, &self.calls.targets) {
            AUTO.clone_into(&mut self.calls.default);
        }
        self
    }
}

impl SessionTools {
    pub fn validate(&self) -> bool {
        self.schema == SCHEMA
            && self
                .calls
                .targets
                .as_ref()
                .is_none_or(|targets| valid_targets(targets))
            && self
                .calls
                .default
                .as_ref()
                .is_none_or(|default| default == AUTO || CALL_TARGETS.contains(&default.as_str()))
    }

    fn is_empty(&self) -> bool {
        *self
            == Self {
                schema: SCHEMA,
                ..Self::default()
            }
    }
}

/// Stored defaults for one agent kind, else the built-in ones.
pub(super) fn agent_defaults(state: &AppState, agent: &str) -> (AgentTools, bool) {
    state
        .hub
        .setting(&format!("{AGENT_TOOLS_PREFIX}{agent}"))
        .and_then(|value| serde_json::from_value::<AgentTools>(value).ok())
        .filter(AgentTools::validate)
        .map_or_else(
            || (AgentTools::builtin(agent), false),
            |stored| (stored.for_caller(agent), true),
        )
}

pub(super) fn session_override(state: &AppState, session: &str) -> SessionTools {
    state
        .hub
        .setting(&format!("{SESSION_TOOLS_PREFIX}{session}"))
        .and_then(|value| serde_json::from_value::<SessionTools>(value).ok())
        .filter(SessionTools::validate)
        .unwrap_or(SessionTools {
            schema: SCHEMA,
            ..SessionTools::default()
        })
}

/// What a session actually runs with: its agent kind's defaults with its own
/// override applied.
pub(super) fn effective(state: &AppState, session: &crate::core::SessionMeta) -> AgentTools {
    agent_defaults(state, &session.provider)
        .0
        .overlay(&session_override(state, &session.id))
        .for_caller(&session.provider)
}

/// A policy that admits `requested` once, as a person approved it: calls are
/// on and the requested agent is a target. Limits still apply.
pub(super) fn allow_once(mut policy: CallsPolicy, caller: &str, requested: &str) -> CallsPolicy {
    policy.enabled = true;
    widen(&mut policy.targets, caller, requested);
    policy
}

fn widen(targets: &mut Vec<CallTarget>, caller: &str, requested: &str) {
    let missing: Vec<&str> = if requested == AUTO {
        if targets.is_empty() {
            callable(caller).collect()
        } else {
            Vec::new()
        }
    } else if CALL_TARGETS.contains(&requested)
        && !same_family(requested, caller)
        && !targets.iter().any(|target| target.agent == requested)
    {
        vec![requested]
    } else {
        Vec::new()
    };
    targets.extend(missing.into_iter().map(|agent| CallTarget {
        agent: agent.to_owned(),
        preset: None,
    }));
}

/// Turn calls on for one session and allow the agents it just asked for.
/// Only the session's override changes; its kind's defaults stay as they are.
pub(super) fn allow_session(
    state: &AppState,
    meta: &crate::core::SessionMeta,
    requested: &[String],
) {
    let mut session = session_override(state, &meta.id);
    let current = effective(state, meta).calls;
    let mut targets = current.targets.clone();
    for agent in requested {
        widen(&mut targets, &meta.provider, agent);
    }
    session.calls.enabled = Some(true);
    if targets != current.targets {
        session.calls.targets = Some(targets);
    }
    state
        .hub
        .set_setting(format!("{SESSION_TOOLS_PREFIX}{}", meta.id), json!(session));
}

/// A requested agent resolved against the session's policy, before readiness.
pub(super) enum Choice {
    /// Ordered candidates; Auto tries each until one is ready.
    Candidates {
        targets: Vec<CallTarget>,
        selection: &'static str,
    },
    Refused(&'static str),
}

/// Resolve `requested` (an agent id or `auto`) under the parent's policy.
pub(super) fn choose(policy: &CallsPolicy, parent_agent: &str, requested: &str) -> Choice {
    // Not a policy question a person could answer: never offered, never asked.
    if requested != AUTO && same_family(requested, parent_agent) {
        return Choice::Refused("same_agent");
    }
    if !policy.enabled {
        return Choice::Refused("calls_disabled");
    }
    // The caller's choice is what a replay must match, even when the policy
    // default turns `auto` into one fixed agent.
    let selection = if requested == AUTO { AUTO } else { "explicit" };
    let requested = if requested == AUTO {
        policy.default.as_str()
    } else {
        requested
    };
    if requested != AUTO {
        return match policy
            .targets
            .iter()
            .find(|target| target.agent == requested)
        {
            Some(target) => Choice::Candidates {
                targets: vec![target.clone()],
                selection,
            },
            None => Choice::Refused("policy_denied"),
        };
    }
    // Another model reviews more independently; keep the configured order
    // otherwise.
    let mut targets = policy.targets.clone();
    targets.sort_by_key(|target| target.agent == parent_agent);
    Choice::Candidates {
        targets,
        selection: AUTO,
    }
}

fn agents_listing(state: &AppState) -> Vec<String> {
    let mut agents: BTreeSet<String> = CALL_TARGETS
        .iter()
        .map(|agent| (*agent).to_owned())
        .collect();
    for meta in state.hub.session_list() {
        if valid_agent(&meta.provider) {
            agents.insert(meta.provider);
        }
    }
    agents.into_iter().collect()
}

/// Presets of an agent's newest signed package, for choosing a target model.
/// Whether `preset` is one of `agent`'s signed configuration presets.
pub(super) fn preset_exists(state: &AppState, agent: &str, preset: &str) -> bool {
    state
        .provider_catalog
        .latest_package(agent)
        .is_some_and(|(_, _, package)| {
            package
                .manifest
                .configuration
                .presets
                .iter()
                .any(|candidate| candidate.id == preset)
        })
}

pub(super) fn presets(state: &AppState, agent: &str) -> serde_json::Value {
    state
        .provider_catalog
        .latest_package(agent)
        .map(|(_, _, package)| {
            json!(
                package
                    .manifest
                    .configuration
                    .presets
                    .iter()
                    .map(|preset| json!({
                        "id": preset.id, "name": preset.name, "detail": preset.detail,
                        "is_default": preset.is_default,
                    }))
                    .collect::<Vec<_>>()
            )
        })
        .unwrap_or_else(|| json!([]))
}

/// Targets with their presets; `caller` limits them to the ones it may call.
fn catalog(state: &AppState, caller: Option<&str>) -> serde_json::Value {
    json!({
        "call_targets": CALL_TARGETS.iter().filter(|agent| {
            caller.is_none_or(|caller| !same_family(agent, caller))
        }).map(|agent| json!({
            "agent": agent, "presets": presets(state, agent),
        })).collect::<Vec<_>>(),
    })
}

pub(super) async fn list(
    State(state): State<Arc<AppState>>,
    Extension(_authenticated): Extension<AuthenticatedProductRequest>,
) -> Response {
    let agents: Vec<_> = agents_listing(&state)
        .into_iter()
        .map(|agent| {
            let (settings, customized) = agent_defaults(&state, &agent);
            json!({"agent": agent, "settings": settings, "customized": customized})
        })
        .collect();
    Json(json!({"schema": SCHEMA, "agents": agents, "catalog": catalog(&state, None)}))
        .into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AgentUpdate {
    /// `null` restores the built-in defaults.
    settings: Option<AgentTools>,
}

pub(super) async fn configure_agent(
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    Path(agent): Path<String>,
    Json(update): Json<AgentUpdate>,
) -> Response {
    // Defaults apply to every session of this agent kind.
    if authenticated.principal.role != crate::admin::AdminRole::Owner {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !valid_agent(&agent)
        || update
            .settings
            .as_ref()
            .is_some_and(|settings| !settings.validate())
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    state.hub.set_setting(
        format!("{AGENT_TOOLS_PREFIX}{agent}"),
        update
            .settings
            .as_ref()
            .map_or(serde_json::Value::Null, |settings| json!(settings)),
    );
    let (settings, customized) = agent_defaults(&state, &agent);
    Json(json!({"schema": SCHEMA, "agent": agent, "settings": settings, "customized": customized}))
        .into_response()
}

fn session_view(state: &AppState, meta: &crate::core::SessionMeta) -> serde_json::Value {
    let (defaults, customized) = agent_defaults(state, &meta.provider);
    let session = session_override(state, &meta.id);
    json!({
        "schema": SCHEMA,
        "session_id": meta.id,
        "agent": meta.provider,
        "defaults": defaults,
        "defaults_customized": customized,
        "override": session,
        "effective": defaults.clone().overlay(&session).for_caller(&meta.provider),
        "catalog": catalog(state, Some(&meta.provider)),
    })
}

pub(super) async fn session(
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    Path(session): Path<String>,
) -> Response {
    if !session_is_visible(&state.hub, &authenticated.principal, &session) {
        return StatusCode::NOT_FOUND.into_response();
    }
    match state.hub.session_info(&session) {
        Some(info) => Json(session_view(&state, &info.meta)).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

pub(super) async fn configure_session(
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    Path(session): Path<String>,
    Json(update): Json<SessionTools>,
) -> Response {
    if !session_is_visible(&state.hub, &authenticated.principal, &session) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(info) = state.hub.session_info(&session) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !authenticated
        .principal
        .can_mutate(info.meta.owner_user_id.as_deref())
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !update.validate() {
        return StatusCode::BAD_REQUEST.into_response();
    }
    state.hub.set_setting(
        format!("{SESSION_TOOLS_PREFIX}{session}"),
        if update.is_empty() {
            serde_json::Value::Null
        } else {
            json!(update)
        },
    );
    Json(session_view(&state, &info.meta)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_are_off_by_default_and_target_only_other_families() {
        let codex = AgentTools::builtin("codex");
        assert!(codex.validate());
        assert!(codex.matrix.tools && codex.matrix.recall);
        assert!(!codex.calls.enabled);
        let agents = |tools: &AgentTools| -> Vec<String> {
            tools
                .calls
                .targets
                .iter()
                .map(|target| target.agent.clone())
                .collect()
        };
        assert_eq!(agents(&codex), ["claude-code"]);
        assert_eq!(agents(&AgentTools::builtin("claude-deepseek")), ["codex"]);
        assert!(matches!(
            choose(&codex.calls, "codex", AUTO),
            Choice::Refused("calls_disabled")
        ));
        let mut enabled = codex.calls.clone();
        enabled.enabled = true;
        let Choice::Candidates { targets, selection } = choose(&enabled, "codex", AUTO) else {
            panic!("auto must offer candidates");
        };
        assert_eq!(selection, AUTO);
        assert_eq!(targets[0].agent, "claude-code");
        // Its own family is never a call, whatever the policy says.
        assert!(matches!(
            choose(&enabled, "codex-deepseek", "codex"),
            Choice::Refused("same_agent")
        ));
        assert!(matches!(
            choose(&codex.calls, "codex", "codex"),
            Choice::Refused("same_agent")
        ));
        // A fixed default still records the caller's `auto` for replays.
        enabled.default = "claude-code".into();
        let Choice::Candidates { targets, selection } = choose(&enabled, "codex", AUTO) else {
            panic!("a fixed default must offer its target");
        };
        assert_eq!(
            (targets.len(), targets[0].agent.as_str(), selection),
            (1, "claude-code", AUTO)
        );
        // Stored settings naming the own family are cleaned for the caller.
        let mut stored = AgentTools::builtin("claude-code");
        stored.calls.targets.push(CallTarget {
            agent: "claude-code".into(),
            preset: None,
        });
        stored.calls.default = "claude-code".into();
        let cleaned = stored.for_caller("claude-code");
        assert_eq!(agents(&cleaned), ["codex"]);
        assert_eq!(cleaned.calls.default, AUTO);
    }

    #[test]
    fn an_approval_admits_the_requested_agent_once() {
        let mut calls = AgentTools::builtin("claude-code").calls;
        calls.targets.clear();
        assert!(matches!(
            choose(&calls, "claude-code", "codex"),
            Choice::Refused("calls_disabled")
        ));
        let once = allow_once(calls.clone(), "claude-code", "codex");
        assert!(matches!(
            choose(&once, "claude-code", "codex"),
            Choice::Candidates {
                selection: "explicit",
                ..
            }
        ));
        // Approval never widens a policy to the caller's own family.
        let once = allow_once(calls, "claude-code", AUTO);
        let agents: Vec<_> = once
            .targets
            .iter()
            .map(|target| target.agent.as_str())
            .collect();
        assert_eq!(agents, ["codex"]);
    }

    #[test]
    fn sessions_override_only_what_they_name_and_policy_limits_hold() {
        let defaults = AgentTools::builtin("claude-code");
        let session = SessionTools {
            schema: SCHEMA,
            matrix: MatrixOverride {
                tools: None,
                recall: Some(false),
            },
            calls: CallsOverride {
                enabled: Some(true),
                targets: Some(vec![CallTarget {
                    agent: "codex".into(),
                    preset: Some("astra-max".into()),
                }]),
                default: Some("claude-code".into()),
            },
        };
        assert!(session.validate());
        let effective = defaults.overlay(&session);
        assert!(effective.matrix.tools && !effective.matrix.recall);
        assert!(effective.calls.enabled);
        // A default naming a removed target falls back to Auto.
        assert_eq!(effective.calls.default, AUTO);
        assert!(effective.validate());
        assert!(matches!(
            choose(&effective.calls, "codex-deepseek", "claude-code"),
            Choice::Refused("policy_denied")
        ));
        let mut invalid = AgentTools::builtin("codex");
        invalid.calls.targets.push(CallTarget {
            agent: "claude-code".into(),
            preset: None,
        });
        assert!(!invalid.validate());
        invalid = AgentTools::builtin("codex");
        invalid.calls.targets[0].agent = "gemini".into();
        assert!(!invalid.validate());
        invalid = AgentTools::builtin("codex");
        invalid.calls.max_concurrent = 0;
        assert!(!invalid.validate());
        assert!(
            SessionTools {
                schema: SCHEMA,
                ..SessionTools::default()
            }
            .is_empty()
        );
    }
}
