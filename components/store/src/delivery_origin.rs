//! Controller-authoritative caller and delivery origin types (C-auth).
//!
//! Kept in a leaf module with no dependency on `model` or `prompt_origin` so
//! `QueuedMessage` can carry durable provenance without creating a module cycle.

use serde::{Deserialize, Serialize};

/// Authenticated caller class assigned at Controller ingress from the verified
/// credential (or internal producer), never inferred from a client message id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RequestCaller {
    /// A person acting through a browser/native-shell login cookie, a human
    /// sender-constrained device token, or single-user local mode.
    Person,
    /// A scoped automation credential (`cow_automation_*`), non-human session,
    /// or static bearer token. Never counts as `Person` even when resolved to a
    /// human user's `ProductPrincipal`.
    Automation,
    /// A delegated Agent API caller acting on behalf of `session_id`.
    Agent { session_id: String },
    /// A Cowboy-internal producer (schedule, managed_call, session_notice,
    /// superagent, code_intelligence, auto-resume).
    Cowboy { source: String },
}

impl RequestCaller {
    #[must_use]
    pub fn is_person(&self) -> bool {
        matches!(self, Self::Person)
    }

    #[must_use]
    pub fn agent(session_id: impl Into<String>) -> Self {
        Self::Agent {
            session_id: session_id.into(),
        }
    }

    #[must_use]
    pub fn cowboy(source: impl Into<String>) -> Self {
        Self::Cowboy {
            source: source.into(),
        }
    }
}

/// Typed provenance of a prompt's content, initiating trigger, or dispatch.
/// Assigned by the Controller from verified ingress credentials or internal
/// call sites — never derived from client message ids.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeliveryOrigin {
    /// A person acting directly.
    Owner,
    /// A scoped automation credential or static bearer token.
    Automation,
    /// A delegated session acting through the Agent API.
    Peer { from_session: String },
    /// A Cowboy-internal producer or legacy unattributed queue row.
    Cowboy { source: String },
}

impl DeliveryOrigin {
    #[must_use]
    pub fn from_caller(caller: &RequestCaller) -> Self {
        match caller {
            RequestCaller::Person => Self::Owner,
            RequestCaller::Automation => Self::Automation,
            RequestCaller::Agent { session_id } => Self::Peer {
                from_session: session_id.clone(),
            },
            RequestCaller::Cowboy { source } => Self::Cowboy {
                source: source.clone(),
            },
        }
    }

    #[must_use]
    pub fn peer(from_session: impl Into<String>) -> Self {
        Self::Peer {
            from_session: from_session.into(),
        }
    }

    #[must_use]
    pub fn cowboy(source: impl Into<String>) -> Self {
        Self::Cowboy {
            source: source.into(),
        }
    }

    #[must_use]
    pub fn is_owner(&self) -> bool {
        matches!(self, Self::Owner)
    }
}

/// Durable Controller-side record of a prompt or steer handoff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptDispatchRecord {
    pub session_id: String,
    pub cmid: String,
    pub origin: DeliveryOrigin,
    pub trigger: DeliveryOrigin,
    pub dispatched_at_ms: i64,
    #[serde(default)]
    pub undelivered_retired: bool,
}

/// Compute the authoritative dispatch origin for a send.
///
/// A dispatch is `Owner` ONLY when BOTH the content writer (`content_origin`)
/// AND the initiating trigger (`trigger`) are `DeliveryOrigin::Owner`.
/// - Missing `content_origin` or `trigger` (legacy rows) -> non-owner.
/// - Non-owner trigger (e.g. an agent activating/sending a human's row) -> trigger origin.
/// - Non-owner content with owner trigger (e.g. a human activating/retrying an agent's row) -> content origin (never promotes).
#[must_use]
pub fn compute_dispatch_origin(
    content_origin: Option<&DeliveryOrigin>,
    trigger: Option<&DeliveryOrigin>,
) -> DeliveryOrigin {
    match (content_origin, trigger) {
        (Some(DeliveryOrigin::Owner), Some(DeliveryOrigin::Owner)) => DeliveryOrigin::Owner,
        (Some(content), _) if !content.is_owner() => content.clone(),
        (_, Some(trigger)) if !trigger.is_owner() => trigger.clone(),
        _ => DeliveryOrigin::cowboy("legacy_queue"),
    }
}
