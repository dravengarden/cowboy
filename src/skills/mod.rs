//! Skills — provider-agnostic capability units evaluated at turn-end (design §B).
//!
//! confirm-detect is the first: it classifies whether the agent is asking the
//! user (→ hold the queue + the awaiting widget) and whether the task is done
//! (→ a future "complete" notification). A skill exposes its prompt + extraction
//! for the Info UI (inspectable), and runs a two-layer judgment — L1
//! deterministic per agent-provider (Step 16), falling back to L2 (an LLM call).

use serde::Serialize;

pub mod confirm;

/// The multi-field turn-end verdict a classifier skill produces in ONE LLM call.
#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct Verdict {
    /// The agent is asking / can't proceed without the user → hold + widget.
    pub awaiting_user: bool,
    /// The agent completed the task → a future "task complete" notification.
    pub done: bool,
    /// 0..1, informational only — recall-first: we act on `awaiting_user`
    /// regardless of confidence.
    pub confidence: f32,
    /// A short machine/debug reason.
    pub reason: String,
}

/// Declarative skill metadata, surfaced verbatim in the Info UI so the prompt +
/// the value-extraction rule are inspectable.
#[derive(Debug, Clone, Serialize)]
pub struct SkillMeta {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    /// The stable prompt prefix (the per-turn text is appended at call time, so
    /// this prefix is kept stable for the classifier session's prompt cache).
    pub prompt_template: &'static str,
    /// How the raw LLM output maps onto the typed `Verdict`.
    pub extract: &'static str,
}

/// Every registered skill's metadata — provider-independent; an agent provider
/// may run any subset. (One skill in v1.)
#[must_use]
pub fn registry() -> Vec<SkillMeta> {
    vec![confirm::meta()]
}
