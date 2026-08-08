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

/// L1 for claude-code. No reliable claude-code-specific `EndTurn` marker yet, so
/// this is just the shared stop-reason rule: a cut-off/cancelled turn is
/// deterministically "not awaiting"; a normal `EndTurn` falls to L2.
#[must_use]
pub fn confirm_l1(ctx: &TurnEndCtx) -> Option<Verdict> {
    stop_reason_l1(ctx.stop_reason)
}

#[cfg(test)]
mod tests {
    use super::is_context_window_rejection;

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
}
