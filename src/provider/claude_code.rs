//! claude-code agent-provider specifics. Today: L1 confirm-detection.
//!
//! Expected to change often — claude-code-specific turn-end markers (a structured
//! "needs input" end, a permission-style handoff that survives to turn-end, …)
//! land here as they're observed live, ahead of the L2 LLM judge.

use super::confirm::{TurnEndCtx, stop_reason_l1};
use crate::skills::Verdict;

/// L1 for claude-code. No reliable claude-code-specific `EndTurn` marker yet, so
/// this is just the shared stop-reason rule: a cut-off/cancelled turn is
/// deterministically "not awaiting"; a normal `EndTurn` falls to L2.
#[must_use]
pub fn confirm_l1(ctx: &TurnEndCtx) -> Option<Verdict> {
    stop_reason_l1(ctx.stop_reason)
}
