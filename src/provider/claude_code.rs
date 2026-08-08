//! claude-code agent-provider specifics. Today: L1 confirm-detection.
//!
//! Expected to change often — claude-code-specific turn-end markers (a structured
//! "needs input" end, a permission-style handoff that survives to turn-end, …)
//! land here as they're observed live, ahead of the L2 LLM judge.

use super::confirm::{TurnEndCtx, stop_reason_l1};
use crate::skills::Verdict;

/// Whether a completed ACP request was rejected because its prompt no longer
/// fits the provider's context window. The adapter remains connected after
/// these API responses, so recycling it only reloads the same oversized native
/// thread and makes the next request larger.
#[must_use]
pub fn is_context_window_rejection(detail: &str) -> bool {
    let detail = detail.to_ascii_lowercase();
    detail.contains("maximum context length")
        || detail.contains("prompt is too long")
        || (detail.contains("context window")
            && (detail.contains("exceed")
                || detail.contains("full")
                || detail.contains("limit reached")))
}

/// Whether Claude Code ended a streaming API attempt before receiving any
/// upstream event. The adapter remains connected after this turn-scoped error,
/// and no model text or tool call can have been produced by that attempt.
#[must_use]
pub fn is_empty_stream_failure(detail: &str) -> bool {
    detail
        .to_ascii_lowercase()
        .contains("stream ended without receiving any events")
}

/// Turn failures after which the connected Claude ACP worker is still usable.
/// Context rejection is currently specific to the isolated DeepSeek lane;
/// empty-stream failures are emitted by both ordinary Claude and DeepSeek.
#[must_use]
pub fn keeps_worker_alive(provider_id: &str, detail: &str) -> bool {
    (provider_id == "claude-deepseek" && is_context_window_rejection(detail))
        || (matches!(provider_id, "claude-code" | "claude-deepseek")
            && is_empty_stream_failure(detail))
}

/// Retry one empty-stream turn only while Cowboy has observed no visible ACP
/// update. The attempt bound prevents a persistent provider outage from
/// becoming an unbounded duplicate-request loop.
#[must_use]
pub fn should_retry_empty_stream(
    provider_id: &str,
    detail: &str,
    visible_update: bool,
    retries: usize,
) -> bool {
    matches!(provider_id, "claude-code" | "claude-deepseek")
        && is_empty_stream_failure(detail)
        && !visible_update
        && retries == 0
}

/// L1 for claude-code. No reliable claude-code-specific `EndTurn` marker yet, so
/// this is just the shared stop-reason rule: a cut-off/cancelled turn is
/// deterministically "not awaiting"; a normal `EndTurn` falls to L2.
#[must_use]
pub fn confirm_l1(ctx: &TurnEndCtx) -> Option<Verdict> {
    stop_reason_l1(ctx.stop_reason)
}

#[cfg(test)]
mod tests {
    use super::{
        is_context_window_rejection, is_empty_stream_failure, keeps_worker_alive,
        should_retry_empty_stream,
    };

    #[test]
    fn context_limit_rejections_are_distinct_from_other_api_errors() {
        assert!(is_context_window_rejection(
            "Internal error: API Error: 400 This model's maximum context length is 1048576 tokens. However, you requested 1048875 tokens"
        ));
        assert!(is_context_window_rejection(
            "prompt is too long: 205000 tokens > 200000 maximum"
        ));
        assert!(is_context_window_rejection("context window limit reached"));
        assert!(!is_context_window_rejection(
            "API Error: 400 thinking content must be preserved"
        ));
        assert!(!is_context_window_rejection("socket connection timed out"));
    }

    #[test]
    fn empty_stream_retry_is_bounded_to_zero_output_claude_turns() {
        let detail = "Internal error: API Error: Stream ended without receiving any events {\"errorKind\":\"unknown\"}";
        assert!(is_empty_stream_failure(detail));
        assert!(should_retry_empty_stream("claude-code", detail, false, 0));
        assert!(should_retry_empty_stream(
            "claude-deepseek",
            detail,
            false,
            0
        ));
        assert!(!should_retry_empty_stream("claude-code", detail, true, 0));
        assert!(!should_retry_empty_stream("claude-code", detail, false, 1));
        assert!(!should_retry_empty_stream("codex", detail, false, 0));
        assert!(!should_retry_empty_stream(
            "claude-code",
            "Connection closed mid-response",
            false,
            0
        ));
    }

    #[test]
    fn only_turn_scoped_failures_keep_the_claude_worker_alive() {
        let empty = "API Error: Stream ended without receiving any events";
        let context = "API Error: 400 This model's maximum context length is 1048576 tokens";
        assert!(keeps_worker_alive("claude-code", empty));
        assert!(keeps_worker_alive("claude-deepseek", empty));
        assert!(keeps_worker_alive("claude-deepseek", context));
        assert!(!keeps_worker_alive("claude-code", context));
        assert!(!keeps_worker_alive(
            "claude-code",
            "agent subprocess exited mid-session"
        ));
    }
}
