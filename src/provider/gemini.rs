//! gemini agent-provider specifics. Today: L1 confirm-detection.
//!
//! Expected to change often — gemini-specific turn-end markers land here as
//! they're observed live, ahead of the L2 LLM judge.

use super::confirm::{TurnEndCtx, stop_reason_l1};
use crate::skills::Verdict;

const RETIRED_CONSUMER_ERROR: &str =
    "This client is no longer supported for Gemini Code Assist for individuals";

pub const RETIRED_CONSUMER_DETAIL: &str = "Gemini CLI no longer supports Google Login for personal, Google AI Pro, or AI Ultra accounts. Use a Gemini API key, Code Assist Standard/Enterprise, or migrate to Antigravity.";

/// Google retired consumer Google Login for Gemini CLI on 2026-06-18. This is
/// an account capability boundary, not a transient ACP/network failure, so an
/// opened session must not spin up another worker until the user explicitly
/// retries after changing credentials.
#[must_use]
pub fn is_retired_consumer_error(detail: &str) -> bool {
    detail.contains(RETIRED_CONSUMER_ERROR) || detail == RETIRED_CONSUMER_DETAIL
}

#[must_use]
pub fn user_facing_startup_error(detail: &str) -> Option<&'static str> {
    is_retired_consumer_error(detail).then_some(RETIRED_CONSUMER_DETAIL)
}

/// L1 for gemini. No reliable gemini-specific `EndTurn` marker yet, so this is
/// just the shared stop-reason rule: a cut-off/cancelled turn is
/// deterministically "not awaiting"; a normal `EndTurn` falls to L2.
#[must_use]
pub fn confirm_l1(ctx: &TurnEndCtx) -> Option<Verdict> {
    stop_reason_l1(ctx.stop_reason)
}

#[cfg(test)]
mod tests {
    use super::{RETIRED_CONSUMER_DETAIL, is_retired_consumer_error, user_facing_startup_error};

    #[test]
    fn consumer_retirement_is_terminal_and_gets_actionable_copy() {
        let raw = "acp connection: This client is no longer supported for Gemini Code Assist for individuals. To continue using Gemini, please migrate";
        assert!(is_retired_consumer_error(raw));
        assert_eq!(
            user_facing_startup_error(raw),
            Some(RETIRED_CONSUMER_DETAIL)
        );
        assert!(!is_retired_consumer_error("acp connection: timed out"));
    }
}
