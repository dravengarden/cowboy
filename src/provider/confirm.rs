//! L1 confirm-detection: deterministic, no-LLM turn-end classification before
//! the shared L2 judge.
//!
//! The one signal that's reliable across providers is the ACP **stop reason**:
//! any non-`EndTurn` stop (cancelled, token/turn limit, refusal, error) means the
//! turn was cut off — the agent is NOT asking the user, so we can answer without
//! an LLM call. `EndTurn` is genuinely ambiguous (a finished task and a question
//! both end the turn normally), so it falls through to L2.
//!
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

/// Return `Some(verdict)` only for deterministic protocol stop reasons.
#[must_use]
pub fn l1(ctx: &TurnEndCtx) -> Option<Verdict> {
    stop_reason_l1(ctx.stop_reason)
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
    fn portable_dispatch_uses_stop_reason() {
        let ctx = TurnEndCtx {
            stop_reason: Some("Cancelled"),
        };
        assert!(l1(&ctx).is_some());
        // EndTurn always falls through, every provider.
        let end = TurnEndCtx {
            stop_reason: Some("EndTurn"),
        };
        assert!(l1(&end).is_none());
    }
}
