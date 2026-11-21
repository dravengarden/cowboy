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
mod effort_store;
pub(super) use effort_store::store_session;

pub(super) const SCHEMA: u16 = 1;
/// Agents a managed call can start. Each must declare the read-only profile.
pub(super) const CALL_TARGETS: [&str; 2] = ["codex", "claude-code"];
pub(super) const AUTO: &str = "auto";
/// Agents a managed call may start only when a caller names them explicitly,
/// for one fresh read-only round: an analysis (a context ask over recorded
/// history included), a review or a design review. They are never Tools
/// policy targets, Auto choices or approvals: Antigravity accounts are
/// personal, so only an explicit request spends one.
pub(super) const CONTEXT_ONLY_TARGETS: [&str; 1] = ["antigravity"];

pub(super) fn context_only(agent: &str) -> bool {
    CONTEXT_ONLY_TARGETS.contains(&agent)
}

/// The single candidate for an explicit request to a context-only agent, or
/// why the request cannot use that agent. A context ask (an analysis over
/// recorded history) and a workspace analysis answer in text; a review or
/// design review may carry Cowboy's own review schema and nothing else. A
/// review is a second opinion, so it never comes from the caller's own agent.
pub(super) fn choose_context_only(
    policy: &CallsPolicy,
    caller: &str,
    requested: &str,
    request: &crate::managed_calls::Request,
    continued: bool,
) -> Result<CallTarget, &'static str> {
    use crate::managed_calls::Purpose;
    let context_ask = !request.context.sessions.is_empty()
        && request.purpose == Purpose::Analysis
        && request.output.is_text();
    let workspace = request.context.sessions.is_empty()
        && matches!(
            request.purpose,
            Purpose::Analysis | Purpose::Review | Purpose::DesignReview
        );
    // `agy` has no native schema constraint: its proxy checks only Cowboy's
    // own review schema, so any other structured output is refused here,
    // before a child could be launched on the owner's account.
    // A review is a verdict, so it always carries that checked schema; an
    // analysis answers in text.
    let review = matches!(request.purpose, Purpose::Review | Purpose::DesignReview);
    let output = match &request.output {
        crate::managed_calls::OutputFormat::Text {} => !review,
        crate::managed_calls::OutputFormat::JsonSchema { schema } => {
            review && *schema == crate::managed_calls::review::output_schema()
        }
    };
    // The proxy refuses a prompt `agy` would read as a slash command; refusing
    // it here keeps that refusal from reaching the owner's account.
    // Unicode whitespace and a byte-order mark: a superset of what the
    // proxy's `\s` strips, so admission refuses at least what it would.
    let slash = request
        .prompt()
        .trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}')
        .starts_with('/');
    if continued || !output || slash || !(context_ask || workspace) {
        return Err("unsupported_capability");
    }
    if matches!(request.purpose, Purpose::Review | Purpose::DesignReview)
        && same_family(requested, caller)
    {
        return Err("same_agent");
    }
    if !policy.enabled {
        return Err("calls_disabled");
    }
    Ok(own_target(requested))
}

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
    /// Schema 1 keeps the `matrix` wire and storage key, so every deployed
    /// Web and stored setting reads it; `memory` is accepted on input.
    #[serde(rename = "matrix", alias = "memory")]
    pub memory: MemoryTools,
    pub calls: CallsPolicy,
}

/// The session's memory backend: its MCP tools and the recall injected before
/// each turn are separate costs, so they are separate switches. Which backend
/// serves them is the runtime Machine's installed `memory_backend` Plugin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MemoryTools {
    pub tools: bool,
    pub recall: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CallsPolicy {
    #[serde(default)]
    pub review: ReviewPreferences,
    pub enabled: bool,
    /// Agents this session may call, in Auto's preference order.
    pub targets: Vec<CallTarget>,
    /// `auto` or one of the targets' agents.
    pub default: String,
    pub max_concurrent: u16,
    pub max_per_session: u16,
    /// Image calls may run at all (docs/call-capabilities.md).
    #[serde(default = "enabled_by_default")]
    pub images: bool,
    /// Image calls may also use the caller's own family, and its account.
    #[serde(default)]
    pub self_images: bool,
}

fn enabled_by_default() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CallTarget {
    pub agent: String,
    /// A configuration preset of the agent's signed package; none keeps its
    /// own default model and reasoning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// A custom choice instead of a preset: values for the option ids its
    /// presets set (model, reasoning). Never both.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<std::collections::BTreeMap<String, String>>,
}

/// A record names a custom choice in its `preset` field as `custom:k=v;k=v`,
/// so durable call records keep their schema.
pub(super) const CUSTOM_PREFIX: &str = "custom:";

fn valid_option_part(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|c| !c.is_control() && !matches!(c, ';' | '=' | ':'))
}

impl CallTarget {
    /// What an admitted call records for this choice.
    pub(super) fn recorded_preset(&self) -> Option<String> {
        if let Some(options) = self.options.as_ref().filter(|options| !options.is_empty()) {
            let pairs: Vec<String> = options.iter().map(|(k, v)| format!("{k}={v}")).collect();
            return Some(format!("{CUSTOM_PREFIX}{}", pairs.join(";")));
        }
        self.preset.clone()
    }
}

/// The values of a recorded `custom:` choice, or `None` for a preset id.
pub(super) fn custom_values(recorded: &str) -> Option<std::collections::BTreeMap<String, String>> {
    let body = recorded.strip_prefix(CUSTOM_PREFIX)?;
    body.split(';')
        .map(|pair| {
            let (key, value) = pair.split_once('=')?;
            (valid_option_part(key) && valid_option_part(value))
                .then(|| (key.to_owned(), value.to_owned()))
        })
        .collect()
}

/// A session's changes to its agent kind's defaults. Absent means inherited.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SessionTools {
    pub schema: u16,
    #[serde(default, rename = "matrix", alias = "memory")]
    pub memory: MemoryOverride,
    #[serde(default)]
    pub calls: CallsOverride,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MemoryOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recall: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CallsOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<ReviewOverride>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub targets: Option<Vec<CallTarget>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub self_images: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReviewPreferences {
    pub claude: Vec<String>,
    pub gpt: Vec<String>,
    /// Effort order within a family for reviews not declared high risk.
    /// High-risk reviews always try Max first. Absent in older stores.
    #[serde(default)]
    pub claude_effort: ReviewEffort,
    #[serde(default)]
    pub gpt_effort: ReviewEffort,
}

/// `max` keeps the historical strongest-first order; `standard` starts at
/// medium/default effort and escalates only through the remaining presets.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ReviewEffort {
    #[default]
    Max,
    Standard,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReviewOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpt: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_effort: Option<ReviewEffort>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpt_effort: Option<ReviewEffort>,
}

impl ReviewOverride {
    fn apply(&self, mut defaults: ReviewPreferences) -> ReviewPreferences {
        if let Some(order) = &self.claude {
            defaults.claude = order.clone();
        }
        if let Some(order) = &self.gpt {
            defaults.gpt = order.clone();
        }
        if let Some(effort) = self.claude_effort {
            defaults.claude_effort = effort;
        }
        if let Some(effort) = self.gpt_effort {
            defaults.gpt_effort = effort;
        }
        defaults
    }
}

impl Default for ReviewPreferences {
    fn default() -> Self {
        Self {
            claude: vec!["opus".into(), "fable".into()],
            gpt: vec!["astra".into(), "sol".into()],
            claude_effort: ReviewEffort::Max,
            gpt_effort: ReviewEffort::Max,
        }
    }
}

impl ReviewPreferences {
    pub(super) fn valid(&self) -> bool {
        fn order(values: &[String], allowed: &[&str]) -> bool {
            !values.is_empty()
                && values.len() <= allowed.len()
                && values.iter().all(|v| allowed.contains(&v.as_str()))
                && values.iter().collect::<BTreeSet<_>>().len() == values.len()
        }
        order(&self.claude, &["opus", "fable"]) && order(&self.gpt, &["astra", "sol"])
    }
}

#[cfg(test)]
mod review_preferences_tests {
    use super::*;

    #[test]
    fn legacy_defaults_inherit_and_session_override_is_validated() {
        let base = AgentTools::builtin("codex");
        let mut old = serde_json::to_value(&base).unwrap();
        old["calls"].as_object_mut().unwrap().remove("review");
        let restored: AgentTools = serde_json::from_value(old).unwrap();
        assert_eq!(restored.calls.review, ReviewPreferences::default());
        let sparse: SessionTools = serde_json::from_value(json!({
            "schema":1,"calls":{"review":{"gpt":["sol"]}}
        }))
        .unwrap();
        let mut changed_defaults = restored;
        changed_defaults.calls.review.claude = vec!["fable".into()];
        let inherited = changed_defaults.overlay(&sparse).calls.review;
        assert_eq!(inherited.claude, ["fable"]);
        assert_eq!(inherited.gpt, ["sol"]);
        assert_eq!(inherited.gpt_effort, ReviewEffort::Max);
        let effort: SessionTools = serde_json::from_value(json!({
            "schema":1,"calls":{"review":{"gpt_effort":"standard"}}
        }))
        .unwrap();
        let tuned = AgentTools::builtin("codex").overlay(&effort).calls.review;
        assert_eq!(
            (tuned.gpt_effort, tuned.claude_effort, tuned.gpt),
            (
                ReviewEffort::Standard,
                ReviewEffort::Max,
                ReviewPreferences::default().gpt
            )
        );
        assert!(serde_json::from_value::<ReviewOverride>(json!({"gpt_effort":"min"})).is_err());
        let mut session: SessionTools = serde_json::from_value(json!({
            "schema":1,"calls":{"review":{"claude":["fable","opus"],"gpt":["sol"]}}
        }))
        .unwrap();
        assert!(session.validate());
        assert_eq!(
            base.overlay(&session).calls.review.claude,
            ["fable", "opus"]
        );
        for invalid in [
            json!({"claude":[],"gpt":["astra"]}),
            json!({"claude":["opus","opus"],"gpt":["astra"]}),
            json!({"claude":["sonnet"],"gpt":["astra"]}),
            json!({"claude":["opus"],"gpt":["luna"]}),
        ] {
            session.calls.review = Some(serde_json::from_value(invalid).unwrap());
            assert!(!session.validate());
        }
    }
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
                && target.options.as_ref().is_none_or(|options| {
                    target.preset.is_none()
                        && !options.is_empty()
                        && options.len() <= 4
                        && options
                            .iter()
                            .all(|(k, v)| valid_option_part(k) && valid_option_part(v))
                        && target
                            .recorded_preset()
                            .is_some_and(|recorded| recorded.len() <= 128)
                })
        })
}

fn valid_default(default: &str, targets: &[CallTarget]) -> bool {
    default == AUTO || targets.iter().any(|target| target.agent == default)
}

impl AgentTools {
    /// Memory and agent calls are on. Calls cost tokens only when an agent
    /// decides a change needs a second opinion, which its prompt keeps rare.
    /// Every agent may call the other family, and Auto prefers it.
    pub fn builtin(agent: &str) -> Self {
        let targets = callable(agent)
            .map(|target| CallTarget {
                agent: target.to_owned(),
                preset: None,
                options: None,
            })
            .collect();
        Self {
            schema: SCHEMA,
            memory: MemoryTools {
                tools: true,
                recall: true,
            },
            calls: CallsPolicy {
                review: ReviewPreferences::default(),
                enabled: true,
                targets,
                default: AUTO.to_owned(),
                max_concurrent: 4,
                max_per_session: 64,
                images: true,
                self_images: false,
            },
        }
    }

    pub fn validate(&self) -> bool {
        self.schema == SCHEMA
            && self.calls.review.valid()
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
        if let Some(review) = &session.calls.review {
            self.calls.review = review.apply(self.calls.review);
        }
        if let Some(tools) = session.memory.tools {
            self.memory.tools = tools;
        }
        if let Some(recall) = session.memory.recall {
            self.memory.recall = recall;
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
        if let Some(images) = session.calls.images {
            self.calls.images = images;
        }
        if let Some(self_images) = session.calls.self_images {
            self.calls.self_images = self_images;
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
                .review
                .as_ref()
                .is_none_or(|review| review.apply(ReviewPreferences::default()).valid())
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
    hub_agent_defaults(&state.hub, agent)
}

fn hub_agent_defaults(hub: &crate::core::Hub, agent: &str) -> (AgentTools, bool) {
    effort_store::agent_document(hub, agent)
        .and_then(|value| serde_json::from_value::<AgentTools>(value).ok())
        .filter(AgentTools::validate)
        .map_or_else(
            || (AgentTools::builtin(agent), false),
            |stored| (stored.for_caller(agent), true),
        )
}

pub(super) fn session_override(state: &AppState, session: &str) -> SessionTools {
    hub_session_override(&state.hub, session)
}

fn hub_session_override(hub: &crate::core::Hub, session: &str) -> SessionTools {
    effort_store::session_document(hub, session)
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
    hub_effective(&state.hub, session)
}

/// Copy explicit preferences while preserving links to the destination's defaults.
/// Explicit target restrictions remain restrictive across a family change.
pub(super) fn continuation_settings(mut current: SessionTools, provider: &str) -> SessionTools {
    if let Some(targets) = &mut current.calls.targets {
        let had_targets = !targets.is_empty();
        targets.retain(|target| !same_family(&target.agent, provider));
        if had_targets && targets.is_empty() {
            current.calls.enabled = Some(false);
        }
    }
    if current.calls.default.as_ref().is_some_and(|default| {
        same_family(default, provider)
            || current
                .calls
                .targets
                .as_ref()
                .is_some_and(|targets| !valid_default(default, targets))
    }) {
        current.calls.default = None;
    }
    current
}

fn hub_effective(hub: &crate::core::Hub, session: &crate::core::SessionMeta) -> AgentTools {
    hub_agent_defaults(hub, &session.provider)
        .0
        .overlay(&hub_session_override(hub, &session.id))
        .for_caller(&session.provider)
}

/// Whether replacing `session`'s override grants anything its current policy
/// refuses: a memory switch, the calls switch, or a call target. Order, review
/// preference and narrowing are not widening.
pub(super) fn override_widens(
    hub: &crate::core::Hub,
    session: &crate::core::SessionMeta,
    update: &SessionTools,
) -> bool {
    let before = hub_effective(hub, session);
    let after = hub_agent_defaults(hub, &session.provider)
        .0
        .overlay(update)
        .for_caller(&session.provider);
    (after.memory.tools && !before.memory.tools)
        || (after.memory.recall && !before.memory.recall)
        || (after.calls.enabled && !before.calls.enabled)
        || image_widens(&before.calls, &after.calls)
        || after
            .calls
            .targets
            .iter()
            .any(|target| !before.calls.targets.contains(target))
}

/// Whether a new session under `provider`'s defaults holds a switch `parent`
/// lacks. Targets are skipped: they legitimately differ between providers.
pub(super) fn fresh_session_widens(
    hub: &crate::core::Hub,
    parent: &crate::core::SessionMeta,
    provider: &str,
) -> bool {
    let before = hub_effective(hub, parent);
    let after = hub_agent_defaults(hub, provider).0.for_caller(provider);
    (after.memory.tools && !before.memory.tools)
        || (after.memory.recall && !before.memory.recall)
        || (after.calls.enabled && !before.calls.enabled)
        || image_widens(&before.calls, &after.calls)
}

fn image_widens(before: &CallsPolicy, after: &CallsPolicy) -> bool {
    (after.images && !before.images) || (after.self_images && !before.self_images)
}

/// Copy a source's explicit preferences for a continuation. A denial it only
/// inherits from its own kind's defaults is written down, so a destination with
/// other defaults does not lose it.
pub(super) fn inherited_settings(
    state: &AppState,
    source: &crate::core::SessionMeta,
    provider: &str,
) -> SessionTools {
    let mut settings = continuation_settings(session_override(state, &source.id), provider);
    let policy = effective(state, source).calls;
    if !policy.images {
        settings.calls.images = Some(false);
    }
    if !policy.self_images {
        settings.calls.self_images = Some(false);
    }
    settings
}

/// Whether a call may run in the caller's own agent family: an image call
/// with `self_images` on, or a context ask, which reads recorded history for
/// the caller rather than giving it a second opinion.
/// Whether a request may run on the caller's own Provider family. Images
/// follow `self_images`; every other call may, after other vendors (`choose`
/// orders them first), and a review never runs the caller's own model
/// (`selection::exclude_caller_model`).
pub(super) fn own_family(policy: &CallsPolicy, request: &crate::managed_calls::Request) -> bool {
    if request.purpose.is_image() {
        policy.self_images
    } else {
        // Context asks always could; other calls follow the session's
        // targets, so switching every target off still calls nothing.
        !request.context.sessions.is_empty() || !policy.targets.is_empty()
    }
}

/// A refusal no approval can answer, checked before any is asked for.
pub(super) fn capability_refusal(policy: &CallsPolicy, image: bool) -> Option<&'static str> {
    (image && !policy.images).then_some("capability_disabled")
}

/// Why `provider` cannot run an image call for `caller`, or `None`. Admission
/// (through `choose`) and both forms of discovery agree through this.
pub(super) fn image_refusal(
    policy: &CallsPolicy,
    caller: &str,
    provider: &str,
) -> Option<&'static str> {
    if !policy.images {
        return Some("capability_disabled");
    }
    if same_family(provider, caller) && !policy.self_images {
        return Some("same_agent");
    }
    if !policy.enabled {
        return Some("calls_disabled");
    }
    if !same_family(provider, caller) && !policy.targets.iter().any(|t| t.agent == provider) {
        return Some("policy_denied");
    }
    None
}

/// The session-extension switches a worker launch carries. A change applies
/// at the session's next worker start.
pub(crate) fn launch_extensions(
    hub: &crate::core::Hub,
    session: &crate::core::SessionMeta,
) -> crate::runtime_wire::SessionExtensions {
    let memory = hub_effective(hub, session).memory;
    crate::runtime_wire::SessionExtensions {
        schema: crate::runtime_wire::SESSION_EXTENSIONS_SCHEMA,
        memory_tools: memory.tools,
        memory_recall: memory.recall,
    }
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
        options: None,
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
    effort_store::store_session(&state.hub, &meta.id, Some(json!(session)));
}

/// A requested agent resolved against the session's policy, before readiness.
pub(super) enum Choice {
    /// Ordered candidates; Auto tries each until one is ready.
    Candidates {
        targets: Vec<CallTarget>,
        selection: &'static str,
    },
    Refused(&'static str),
    /// Refused with no question to ask: approval cannot grant a family target.
    Closed(&'static str),
}

/// Resolve `requested` (an agent id or `auto`) under the parent's policy.
/// `own_family` admits the caller's family for this request (an image call
/// with `self_images` on); everything else is a second opinion from another.
pub(super) fn choose(
    policy: &CallsPolicy,
    parent_agent: &str,
    requested: &str,
    own_family: bool,
) -> Choice {
    // Not a policy question a person could answer: never offered, never asked.
    if requested != AUTO && same_family(requested, parent_agent) {
        if !own_family {
            return Choice::Refused("same_agent");
        }
        if !policy.enabled {
            return Choice::Closed("calls_disabled");
        }
        return Choice::Candidates {
            targets: vec![own_target(requested)],
            selection: "explicit",
        };
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
    if own_family {
        targets.extend(
            CALL_TARGETS
                .into_iter()
                .filter(|agent| same_family(agent, parent_agent))
                .map(own_target),
        );
    }
    Choice::Candidates {
        targets,
        selection: AUTO,
    }
}

fn own_target(agent: &str) -> CallTarget {
    CallTarget {
        agent: agent.to_owned(),
        preset: None,
        options: None,
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
/// The option ids an agent's presets set, in their first order.
fn preset_option_ids(state: &AppState, agent: &str) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    if let Some((_, _, package)) = state.provider_catalog.latest_package(agent) {
        for preset in &package.manifest.configuration.presets {
            for id in preset.values.keys() {
                if !ids.contains(id) {
                    ids.push(id.clone());
                }
            }
        }
    }
    ids
}

/// Custom choices for the option ids presets set. Labels and choices come
/// from the newest live session of that agent, which reports them as its
/// configuration options; values only presets name are added as they are.
fn option_choices(state: &AppState, agent: &str) -> serde_json::Value {
    let ids = preset_option_ids(state, agent);
    let live: Vec<serde_json::Value> = state
        .hub
        .session_list()
        .into_iter()
        .rev()
        .filter(|meta| {
            meta.provider == agent
                && meta
                    .execution_binding
                    .as_ref()
                    .is_none_or(|b| b.managed_child().is_none())
        })
        .find_map(|meta| state.hub.reported_config_options(&meta.id))
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    let mut preset_values: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    if let Some((_, _, package)) = state.provider_catalog.latest_package(agent) {
        for preset in &package.manifest.configuration.presets {
            for (id, value) in &preset.values {
                let text = value.clone();
                let list = preset_values.entry(id.clone()).or_default();
                if !list.contains(&text) {
                    list.push(text);
                }
            }
        }
    }
    json!(ids.iter().map(|id| {
        let option = live.iter().find(|option| option.get("id").and_then(serde_json::Value::as_str) == Some(id));
        let mut choices: Vec<serde_json::Value> = option
            .and_then(|option| option.get("options"))
            .and_then(serde_json::Value::as_array)
            .map(|values| values.iter().filter_map(|choice| {
                let value = choice.get("value")?;
                let value = value.as_str().map_or_else(|| value.to_string(), str::to_owned);
                Some(json!({"value": value, "name": choice.get("name").and_then(serde_json::Value::as_str).unwrap_or(&value)}))
            }).collect())
            .unwrap_or_default();
        for value in preset_values.get(id).into_iter().flatten() {
            if !choices.iter().any(|choice| choice["value"] == json!(value)) {
                choices.push(json!({"value": value, "name": value}));
            }
        }
        json!({
            "id": id,
            "name": option.and_then(|option| option.get("name")).and_then(serde_json::Value::as_str).unwrap_or(id),
            "choices": choices,
        })
    }).collect::<Vec<_>>())
}

pub(super) fn presets(state: &AppState, agent: &str) -> serde_json::Value {
    presets_on(state, agent, None)
}

/// The signed presets of one exact generation (`(version, digest)`), or of the
/// latest package without one. Review selection must rank the generation a
/// child will actually run.
pub(super) fn presets_on(
    state: &AppState,
    agent: &str,
    generation: Option<(&str, &str)>,
) -> serde_json::Value {
    let package = match generation {
        Some((version, digest)) => state.provider_catalog.package(agent, version, digest),
        None => state
            .provider_catalog
            .latest_package(agent)
            .map(|(_, _, package)| package),
    };
    package
        .map(|package| {
            json!(
                package
                    .manifest
                    .configuration
                    .presets
                    .iter()
                    .map(|preset| json!({
                        "id": preset.id, "name": preset.name, "detail": preset.detail,
                        "is_default": preset.is_default, "values": preset.values,
                    }))
                    .collect::<Vec<_>>()
            )
        })
        .unwrap_or_else(|| json!([]))
}

/// Targets with their presets; `caller` limits them to the ones it may call.
/// The embedded review routing table (docs/review-routing.md), shown
/// read-only in Settings.
static REVIEW_ROUTING: std::sync::LazyLock<serde_json::Value> = std::sync::LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../../models/routing.json")).unwrap_or_default()
});

fn catalog(state: &AppState, caller: Option<&str>) -> serde_json::Value {
    json!({
        "review_routing": &*REVIEW_ROUTING,
        "call_targets": CALL_TARGETS.iter().filter(|agent| {
            caller.is_none_or(|caller| !same_family(agent, caller))
        }).map(|agent| json!({
            "agent": agent,
            "presets": presets(state, agent),
            "options": option_choices(state, agent),
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
    effort_store::store_agent(
        &state.hub,
        &agent,
        update.settings.as_ref().map(|settings| json!(settings)),
    );
    let (settings, customized) = agent_defaults(&state, &agent);
    Json(json!({"schema": SCHEMA, "agent": agent, "settings": settings, "customized": customized}))
        .into_response()
}

pub(super) fn session_view(state: &AppState, meta: &crate::core::SessionMeta) -> serde_json::Value {
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
    effort_store::store_session(
        &state.hub,
        &session,
        (!update.is_empty()).then(|| json!(update)),
    );
    Json(session_view(&state, &info.meta)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuation_keeps_memory_and_review_choices_without_widening_call_targets() {
        let current: SessionTools = serde_json::from_value(json!({"schema":1,"matrix":{"tools":false,"recall":false},"calls":{"targets":[{"agent":"codex","preset":"astra-max"}],"review":{"claude":["opus"]}}})).unwrap();
        let same = continuation_settings(current.clone(), "claude-code");
        assert!(same.validate());
        assert_eq!(same, current);
        let cross = continuation_settings(current, "codex");
        assert!(cross.validate());
        let effective = AgentTools::builtin("codex")
            .overlay(&cross)
            .for_caller("codex");
        assert!(!effective.memory.tools && !effective.memory.recall);
        assert!(!effective.calls.enabled);
        assert!(effective.calls.targets.is_empty());
        assert_eq!(effective.calls.review.claude, vec!["opus"]);
    }

    #[test]
    fn continuation_keeps_default_policy_links_live() {
        let current = SessionTools {
            schema: SCHEMA,
            ..SessionTools::default()
        };
        for provider in ["claude-code", "codex"] {
            let copied = continuation_settings(current.clone(), provider);
            assert_eq!(copied, current);
            let mut defaults = AgentTools::builtin(provider);
            assert!(!defaults.clone().overlay(&copied).calls.targets.is_empty());
            defaults.calls.enabled = false;
            defaults.memory.recall = false;
            assert_eq!(defaults.clone().overlay(&copied), defaults);
        }
    }

    #[test]
    fn memory_switches_keep_the_matrix_wire_key_and_accept_memory() {
        let mut stored = serde_json::to_value(AgentTools::builtin("codex")).unwrap();
        // Deployed Web clients read and write `matrix`.
        assert_eq!(stored["matrix"]["tools"], true);
        assert!(stored.get("memory").is_none());
        stored["matrix"] = serde_json::json!({"tools": false, "recall": true});
        let loaded: AgentTools = serde_json::from_value(stored.clone()).unwrap();
        assert!(!loaded.memory.tools && loaded.memory.recall);
        let renamed = stored.as_object_mut().unwrap().remove("matrix").unwrap();
        stored["memory"] = renamed;
        let loaded: AgentTools = serde_json::from_value(stored).unwrap();
        assert!(!loaded.memory.tools && loaded.memory.recall);
        let session: SessionTools =
            serde_json::from_value(serde_json::json!({"schema": 1, "memory": {"recall": false}}))
                .unwrap();
        assert_eq!(session.memory.recall, Some(false));
        let effective = AgentTools::builtin("claude-code").overlay(&session);
        assert!(effective.memory.tools && !effective.memory.recall);
    }
    #[test]
    fn calls_are_on_by_default_and_target_only_other_families() {
        let codex = AgentTools::builtin("codex");
        assert!(codex.validate());
        assert!(codex.memory.tools && codex.memory.recall);
        assert!(codex.calls.enabled);
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
        let mut disabled = codex.calls.clone();
        disabled.enabled = false;
        assert!(matches!(
            choose(&disabled, "codex", AUTO, false),
            Choice::Refused("calls_disabled")
        ));
        let mut enabled = codex.calls.clone();
        let Choice::Candidates { targets, selection } = choose(&enabled, "codex", AUTO, false)
        else {
            panic!("auto must offer candidates");
        };
        assert_eq!(selection, AUTO);
        assert_eq!(targets[0].agent, "claude-code");
        // Its own family is never a call, whatever the policy says.
        assert!(matches!(
            choose(&enabled, "codex-deepseek", "codex", false),
            Choice::Refused("same_agent")
        ));
        assert!(matches!(
            choose(&codex.calls, "codex", "codex", false),
            Choice::Refused("same_agent")
        ));
        // A fixed default still records the caller's `auto` for replays.
        enabled.default = "claude-code".into();
        let Choice::Candidates { targets, selection } = choose(&enabled, "codex", AUTO, false)
        else {
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
            options: None,
        });
        stored.calls.default = "claude-code".into();
        let cleaned = stored.for_caller("claude-code");
        assert_eq!(agents(&cleaned), ["codex"]);
        assert_eq!(cleaned.calls.default, AUTO);
    }

    #[test]
    fn a_custom_choice_records_its_values_in_the_preset_field() {
        let custom = CallTarget {
            agent: "codex".into(),
            preset: None,
            options: Some(
                [("model", "gpt-6-astra"), ("reasoning_effort", "high")]
                    .map(|(k, v)| (k.to_owned(), v.to_owned()))
                    .into(),
            ),
        };
        assert!(valid_targets(std::slice::from_ref(&custom)));
        let recorded = custom.recorded_preset().unwrap();
        assert_eq!(recorded, "custom:model=gpt-6-astra;reasoning_effort=high");
        assert_eq!(custom_values(&recorded), custom.options);
        assert_eq!(custom_values("astra-max"), None);
        // Never both, and never a separator inside a value.
        let mut both = custom.clone();
        both.preset = Some("astra-max".into());
        assert!(!valid_targets(&[both]));
        let mut forged = custom;
        forged.options = Some([("model".to_owned(), "a;b=c".to_owned())].into());
        assert!(!valid_targets(&[forged]));
    }

    #[test]
    fn an_approval_admits_the_requested_agent_once() {
        let mut calls = AgentTools::builtin("claude-code").calls;
        calls.enabled = false;
        calls.targets.clear();
        assert!(matches!(
            choose(&calls, "claude-code", "codex", false),
            Choice::Refused("calls_disabled")
        ));
        let once = allow_once(calls.clone(), "claude-code", "codex");
        assert!(matches!(
            choose(&once, "claude-code", "codex", false),
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
            memory: MemoryOverride {
                tools: None,
                recall: Some(false),
            },
            calls: CallsOverride {
                review: None,
                enabled: Some(true),
                targets: Some(vec![CallTarget {
                    agent: "codex".into(),
                    preset: Some("astra-max".into()),
                    options: None,
                }]),
                default: Some("claude-code".into()),
                images: None,
                self_images: None,
            },
        };
        assert!(session.validate());
        let effective = defaults.overlay(&session);
        assert!(effective.memory.tools && !effective.memory.recall);
        assert!(effective.calls.enabled);
        // A default naming a removed target falls back to Auto.
        assert_eq!(effective.calls.default, AUTO);
        assert!(effective.validate());
        assert!(matches!(
            choose(&effective.calls, "codex-deepseek", "claude-code", false),
            Choice::Refused("policy_denied")
        ));
        let mut invalid = AgentTools::builtin("codex");
        invalid.calls.targets.push(CallTarget {
            agent: "claude-code".into(),
            preset: None,
            options: None,
        });
        assert!(!invalid.validate());
        invalid = AgentTools::builtin("codex");
        invalid.calls.targets[0].agent = "grok".into();
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

    fn policy(images: bool, self_images: bool) -> CallsPolicy {
        let mut calls = AgentTools::builtin("codex").calls;
        calls.images = images;
        calls.self_images = self_images;
        calls
    }

    fn agents(choice: Choice) -> Vec<String> {
        match choice {
            Choice::Candidates { targets, .. } => targets.into_iter().map(|t| t.agent).collect(),
            Choice::Refused(code) | Choice::Closed(code) => vec![code.to_owned()],
        }
    }

    fn ask(document: &str) -> crate::managed_calls::Request {
        crate::managed_calls::Request::parse(document.as_bytes()).unwrap()
    }

    #[test]
    fn antigravity_serves_explicit_fresh_read_only_rounds() {
        let context = ask(
            r#"{"schema":1,"request_id":"a","purpose":"analysis","instruction":"what did we decide?","context":{"scope":"current-worktree","sessions":[{"session_id":"sess-1","before":9}]},"access":"read-only","conversation":{"mode":"fresh"}}"#,
        );
        let plain = ask(
            r#"{"schema":1,"request_id":"b","purpose":"analysis","instruction":"Summarize","context":{"scope":"current-worktree"},"access":"read-only","conversation":{"mode":"fresh"}}"#,
        );
        let schema = crate::managed_calls::review::output_schema();
        let review = ask(&format!(
            r#"{{"schema":1,"request_id":"c","purpose":"review","instruction":"Review","context":{{"scope":"current-worktree"}},"access":"read-only","conversation":{{"mode":"fresh"}},"output":{{"format":"json_schema","schema":{schema}}}}}"#
        ));
        // Any schema but Cowboy's own review schema is refused.
        let custom = ask(
            r#"{"schema":1,"request_id":"f","purpose":"review","instruction":"Review","context":{"scope":"current-worktree"},"access":"read-only","conversation":{"mode":"fresh"},"output":{"format":"json_schema","schema":{"type":"object"}}}"#,
        );
        // A prompt agy would read as a slash command never reaches it.
        let slash = ask(
            r#"{"schema":1,"request_id":"h","purpose":"analysis","instruction":"/tmp/notes.md: summarize","context":{"scope":"current-worktree"},"access":"read-only","conversation":{"mode":"fresh"}}"#,
        );
        let bom = ask(
            r#"{"schema":1,"request_id":"i","purpose":"analysis","instruction":"\ufeff\u0085 /tmp/notes.md","context":{"scope":"current-worktree"},"access":"read-only","conversation":{"mode":"fresh"}}"#,
        );
        let structured_analysis = ask(&format!(
            r#"{{"schema":1,"request_id":"g","purpose":"analysis","instruction":"Summarize","context":{{"scope":"current-worktree"}},"access":"read-only","conversation":{{"mode":"fresh"}},"output":{{"format":"json_schema","schema":{schema}}}}}"#
        ));
        let design = ask(&format!(
            r#"{{"schema":1,"request_id":"d","purpose":"design_review","instruction":"Review the plan","context":{{"scope":"current-worktree"}},"access":"read-only","conversation":{{"mode":"fresh"}},"output":{{"format":"json_schema","schema":{}}}}}"#,
            crate::managed_calls::review::output_schema()
        ));
        // A review without the checked schema would be an unchecked verdict.
        let text_review = ask(
            r#"{"schema":1,"request_id":"j","purpose":"review","instruction":"Review","context":{"scope":"current-worktree"},"access":"read-only","conversation":{"mode":"fresh"}}"#,
        );
        let structured_history = ask(
            r#"{"schema":1,"request_id":"e","purpose":"analysis","instruction":"what?","context":{"scope":"current-worktree","sessions":[{"session_id":"sess-1"}]},"access":"read-only","conversation":{"mode":"fresh"},"output":{"format":"json_schema","schema":{"type":"object"}}}"#,
        );
        let on = policy(true, false);
        for request in [&context, &plain, &review, &design] {
            assert_eq!(
                choose_context_only(&on, "codex", "antigravity", request, false)
                    .unwrap()
                    .agent,
                "antigravity"
            );
        }
        // A context ask answers in text; nothing is continued.
        for (request, continued) in [
            (&structured_history, false),
            (&custom, false),
            (&text_review, false),
            (&slash, false),
            (&bom, false),
            (&structured_analysis, false),
            (&plain, true),
            (&review, true),
        ] {
            assert_eq!(
                choose_context_only(&on, "codex", "antigravity", request, continued).err(),
                Some("unsupported_capability")
            );
        }
        // A review is a second opinion: never Antigravity's own.
        for request in [&review, &design] {
            assert_eq!(
                choose_context_only(&on, "antigravity", "antigravity", request, false).err(),
                Some("same_agent")
            );
        }
        assert!(choose_context_only(&on, "antigravity", "antigravity", &context, false).is_ok());
        let mut off = on.clone();
        off.enabled = false;
        assert_eq!(
            choose_context_only(&off, "codex", "antigravity", &context, false).err(),
            Some("calls_disabled")
        );
        // Never a policy target, approval widening or Auto candidate.
        for caller in ["codex", "claude-code", "antigravity"] {
            assert!(callable(caller).all(|agent| !context_only(agent)));
            assert!(
                AgentTools::builtin(caller)
                    .calls
                    .targets
                    .iter()
                    .all(|target| !context_only(&target.agent))
            );
        }
    }

    #[test]
    fn stored_policies_without_the_image_fields_keep_todays_behaviour() {
        let mut stored = json!(AgentTools::builtin("codex"));
        stored["calls"].as_object_mut().unwrap().remove("images");
        stored["calls"]
            .as_object_mut()
            .unwrap()
            .remove("self_images");
        let stored: AgentTools = serde_json::from_value(stored).unwrap();
        assert!(stored.calls.images);
        assert!(!stored.calls.self_images);
    }

    #[test]
    fn only_an_image_call_with_self_images_reaches_the_callers_family() {
        let on = policy(true, true);
        assert_eq!(agents(choose(&on, "codex", "codex", true)), ["codex"]);
        assert_eq!(
            agents(choose(&on, "codex", "codex", false)),
            ["same_agent"],
            "a second opinion stays with another family"
        );
        assert_eq!(
            agents(choose(&on, "codex", AUTO, true)),
            ["claude-code", "codex"],
            "Auto keeps its order and tries the own family last"
        );
        let mut fixed = on.clone();
        fixed.default = "claude-code".to_owned();
        assert_eq!(
            agents(choose(&fixed, "codex", AUTO, true)),
            ["claude-code"],
            "a fixed default stays fixed"
        );
        let mut off = on;
        off.enabled = false;
        assert!(matches!(
            choose(&off, "codex", "codex", true),
            Choice::Closed("calls_disabled")
        ));
    }

    #[test]
    fn discovery_refuses_what_admission_refuses() {
        let refusal = |policy: &CallsPolicy, provider| image_refusal(policy, "codex", provider);
        assert_eq!(
            refusal(&policy(false, true), "codex"),
            Some("capability_disabled")
        );
        assert_eq!(refusal(&policy(true, false), "codex"), Some("same_agent"));
        assert_eq!(refusal(&policy(true, true), "codex"), None);
        assert_eq!(refusal(&policy(true, false), "claude-code"), None);
        let mut narrowed = policy(true, true);
        narrowed.targets.clear();
        assert_eq!(refusal(&narrowed, "claude-code"), Some("policy_denied"));
        assert_eq!(refusal(&narrowed, "codex"), None);
    }

    #[test]
    fn a_disabled_capability_is_refused_whatever_approval_would_grant() {
        let off = policy(false, true);
        assert_eq!(capability_refusal(&off, true), Some("capability_disabled"));
        assert_eq!(
            capability_refusal(&allow_once(off.clone(), "codex", "claude-code"), true),
            Some("capability_disabled"),
            "Allow once leaves the switch alone"
        );
        assert_eq!(
            capability_refusal(&off, false),
            None,
            "a review is not an image"
        );
        assert_eq!(capability_refusal(&policy(true, false), true), None);
    }

    #[test]
    fn turning_an_image_switch_on_is_widening() {
        assert!(image_widens(&policy(false, false), &policy(true, false)));
        assert!(image_widens(&policy(true, false), &policy(true, true)));
        assert!(!image_widens(&policy(true, true), &policy(false, false)));
    }
}
