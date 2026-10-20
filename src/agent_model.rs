//! Types and markers shared by the restartable control plane and detached ACP
//! workers. Keeping this boundary small lets worker-generation hashing ignore
//! unrelated Hub/API edits without relying on a hand-waved compatibility rule.

use serde::{Deserialize, Serialize};

/// Provider/session status as shown in the session list.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Agent subprocess spawning / ACP handshake in flight.
    Starting,
    /// Session established; idle, ready for a prompt.
    Running,
    /// A prompt or a provider-reported autonomous continuation is being processed.
    Busy,
    /// Agent exited cleanly (or was stopped).
    Exited,
    /// Agent crashed / the ACP connection failed.
    Crashed,
    /// A turn had no surviving worker during control-plane restore.
    Interrupted,
}

/// Latest provider-reported usage for one ACP session. `raw` preserves optional
/// standard fields (`cost`) and provider extensions (`_meta`) without teaching
/// the worker protocol every provider's private schema.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionUsage {
    pub used: u64,
    pub size: u64,
    #[serde(default)]
    pub raw: serde_json::Value,
    #[serde(default)]
    pub observed_at_ms: i64,
}

pub use crate::runtime_wire::TurnUsage;

impl TurnUsage {
    /// ACP adapters report either disjoint fresh/read/write input counts or
    /// inclusive input. Require the reported totals to reconcile before
    /// projecting a common representation; never count reasoning twice.
    #[must_use]
    pub fn from_acp(usage: &agent_client_protocol::schema::v1::Usage) -> Option<Self> {
        let total = usage.total_tokens;
        let output = usage.output_tokens;
        let input = total.checked_sub(output)?;
        let read = usage.cached_read_tokens;
        let write = usage.cached_write_tokens;
        let disjoint = usage
            .input_tokens
            .checked_add(read.unwrap_or(0))?
            .checked_add(write.unwrap_or(0))?;
        if (usage.input_tokens != input && disjoint != input)
            || read.is_some_and(|tokens| tokens > input)
            || write.is_some_and(|tokens| tokens > input.saturating_sub(read.unwrap_or(0)))
            || usage.thought_tokens.is_some_and(|tokens| tokens > output)
            || total > 9_007_199_254_740_991
        {
            return None;
        }
        Some(Self {
            total_tokens: total,
            input_tokens: input,
            output_tokens: output,
            cached_input_tokens: read,
            uncached_input_tokens: read.map(|tokens| input - tokens),
            cache_write_tokens: write,
            reasoning_tokens: usage.thought_tokens,
        })
    }
}

/// A normalized session event fanned out to clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    /// A serialized ACP `SessionUpdate`.
    Update { update: serde_json::Value },
    PermissionRequest {
        request_id: String,
        tool_call: serde_json::Value,
        options: serde_json::Value,
    },
    PermissionResolved {
        request_id: String,
        option_id: Option<String>,
    },
    Lifecycle {
        status: Status,
        detail: Option<String>,
    },
    TurnEnd {
        stop_reason: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<TurnUsage>,
    },
}

/// Retired synthetic-continuation marker. Kept only to classify historical
/// events and purge queued rows written by older controllers.
pub(crate) const LEGACY_AUTO_CONTINUE_PREFIX: &str = "__cont__";
pub(crate) const SCHED_PREFIX: &str = "__sched__";
pub const WAKEUP_PREFIX: &str = "__wake__";

#[cfg(test)]
mod usage_tests {
    use super::*;
    use agent_client_protocol::schema::v1::Usage;

    #[test]
    fn turn_usage_normalizes_cache_without_double_counting_reasoning() {
        let fresh = Usage::new(1110, 80, 110)
            .cached_read_tokens(900)
            .cached_write_tokens(20)
            .thought_tokens(50);
        let usage = TurnUsage::from_acp(&fresh).unwrap();
        assert_eq!(usage.input_tokens, 1000);
        assert_eq!(usage.uncached_input_tokens, Some(100));
        assert_eq!(usage.total_tokens, usage.input_tokens + usage.output_tokens);
        let mut inclusive = fresh.clone();
        inclusive.input_tokens = 1000;
        assert_eq!(TurnUsage::from_acp(&inclusive), Some(usage));
        inclusive.total_tokens = 1090;
        assert!(TurnUsage::from_acp(&inclusive).is_none());
        inclusive = fresh;
        inclusive.thought_tokens = Some(111);
        assert!(TurnUsage::from_acp(&inclusive).is_none());
        let unknown_cache = TurnUsage::from_acp(&Usage::new(110, 100, 10)).unwrap();
        assert_eq!(unknown_cache.cached_input_tokens, None);
        assert_eq!(unknown_cache.uncached_input_tokens, None);
    }

    #[test]
    fn old_turns_keep_unknown_usage_and_new_turns_survive_serialization() {
        let old = r#"{"kind":"turn_end","stop_reason":"end_turn"}"#;
        let event: Event = serde_json::from_str(old).unwrap();
        assert!(matches!(event, Event::TurnEnd { usage: None, .. }));
        assert_eq!(serde_json::to_string(&event).unwrap(), old);
        let usage = TurnUsage::from_acp(&Usage::new(100, 90, 10)).unwrap();
        let event = Event::TurnEnd {
            stop_reason: "end_turn".into(),
            usage: Some(usage),
        };
        let restored: Event = serde_json::from_value(serde_json::to_value(event).unwrap()).unwrap();
        assert!(matches!(restored, Event::TurnEnd { usage: Some(value), .. } if value == usage));
    }
}
