//! codex agent-provider specifics. Today: L1 confirm-detection.
//!
//! Expected to change often — codex-specific turn-end markers land here as
//! they're observed live, ahead of the L2 LLM judge.

use super::confirm::{TurnEndCtx, stop_reason_l1};
use crate::skills::Verdict;

/// L1 for codex. No reliable codex-specific `EndTurn` marker yet, so this is just
/// the shared stop-reason rule: a cut-off/cancelled turn is deterministically
/// "not awaiting"; a normal `EndTurn` falls to L2.
#[must_use]
pub fn confirm_l1(ctx: &TurnEndCtx) -> Option<Verdict> {
    stop_reason_l1(ctx.stop_reason)
}
