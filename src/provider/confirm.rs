//! L1 confirm-detection: deterministic, no-LLM turn-end classification per agent
//! provider (design §B, the cheap-certain layer before the DeepSeek L2 judge).
//!
//! The one signal that's reliable across providers is the ACP **stop reason**:
//! any non-`EndTurn` stop (cancelled, token/turn limit, refusal, error) means the
//! turn was cut off — the agent is NOT asking the user, so we can answer without
//! an LLM call. `EndTurn` is genuinely ambiguous (a finished task and a question
//! both end the turn normally), so it falls through to L2.
//!
//! Per-provider files (`claude_code.rs`, `codex.rs`) start from this shared rule
//! and add their own markers as they're discovered live — they're expected to
//! change often, which is why each lives in its own module.

use crate::skills::Verdict;

/// What an L1 detector inspects at turn-end.
pub struct TurnEndCtx<'a> {
    /// The ACP `StopReason` debug string from the last `TurnEnd` event
    /// (`"EndTurn"`, `"Cancelled"`, `"MaxTokens"`, `"error: …"`, …), if any.
    pub stop_reason: Option<&'a str>,
}

/// The cross-provider stop-reason rule. `Some` ⇒ deterministic (skip L2); `None`
/// ⇒ ambiguous, hand to the LLM judge.
#[must_use]
pub(crate) fn stop_reason_l1(stop: Option<&str>) -> Option<Verdict> {
    match stop {
        // A normal turn-end (or an unknown/absent reason) is ambiguous: it could
        // be a finished task OR a question. Let L2 decide.
        Some("EndTurn") | None => None,
        // Anything else — cut off, cancelled, refused, errored — is not a
        // question and not a completed deliverable. Don't hold, don't notify.
        Some(other) => Some(Verdict {
            awaiting_user: false,
            done: false,
            confidence: 1.0,
            reason: format!("L1: stop_reason={other}"),
        }),
    }
}

/// Dispatch L1 to the agent-provider's detector. Returns `Some(verdict)` only when
/// DETERMINISTIC (the caller skips the LLM); `None` to fall through to L2.
#[must_use]
pub fn l1(provider_id: &str, ctx: &TurnEndCtx) -> Option<Verdict> {
    match provider_id {
        "claude-code" => super::claude_code::confirm_l1(ctx),
        "codex" => super::codex::confirm_l1(ctx),
        "gemini" => super::gemini::confirm_l1(ctx),
        // Unknown provider → only the portable stop-reason rule applies.
        _ => stop_reason_l1(ctx.stop_reason),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn end_turn_is_ambiguous_falls_to_l2() {
        assert!(stop_reason_l1(Some("EndTurn")).is_none());
        assert!(stop_reason_l1(None).is_none());
    }

    #[test]
    fn cut_off_reasons_are_not_awaiting() {
        for r in [
            "Cancelled",
            "MaxTokens",
            "MaxTurnRequests",
            "Refusal",
            "error: boom",
        ] {
            let v = stop_reason_l1(Some(r)).expect("deterministic");
            assert!(!v.awaiting_user, "{r} must not hold the queue");
            assert!(!v.done);
        }
    }

    #[test]
    fn dispatch_routes_known_providers() {
        let ctx = TurnEndCtx {
            stop_reason: Some("Cancelled"),
        };
        assert!(l1("claude-code", &ctx).is_some());
        assert!(l1("codex", &ctx).is_some());
        // EndTurn always falls through, every provider.
        let end = TurnEndCtx {
            stop_reason: Some("EndTurn"),
        };
        assert!(l1("claude-code", &end).is_none());
        assert!(l1("codex", &end).is_none());
    }
}
