//! Honor the agent's `ScheduleWakeup` tool — fire a wake-prompt at the scheduled
//! time.
//!
//! `ScheduleWakeup` is a built-in Claude Code `/loop` tool: in a normal CLI the
//! loop runtime re-invokes the agent at the scheduled time. Under cowboy's ACP
//! driving COWBOY is the runtime, but it used to ignore the tool, so the wakeup
//! never fired — the agent's deferred work latched until the next user message,
//! which it then *consumed* ("I asked X, got a stale self-check, my message went
//! unanswered"). cowboy sees the `ScheduleWakeup` `tool_call` in the ACP stream
//! (`acp.rs` intercepts it, extracts `{prompt, delaySeconds}`) and routes an
//! [`ScheduleCmd::Arm`] here.
//!
//! A single background task holds ONE pending wakeup per session (latest wins —
//! the `/loop` re-arms each turn) and fires it via [`Hub::submit`], so the wakeup
//! runs as its OWN turn (idle → dispatch, busy → queue) and never piggybacks on a
//! user message. One task + sleep-until-next keeps timers O(1) in session count.

use std::collections::HashMap;

use tokio::sync::mpsc;

use crate::core::Hub;

/// Marks a fired-wakeup prompt's echo so the UI can render it as a "↻ scheduled
/// wakeup" note rather than a user bubble (mirrors `AUTO_CONTINUE_PREFIX`).
pub const WAKEUP_PREFIX: &str = "__wake__";

/// `ScheduleWakeup` clamps `delaySeconds` to `[60, 3600]`; mirror it so a bad
/// value can't busy-loop or schedule absurdly far out.
const MIN_DELAY_S: i64 = 60;
const MAX_DELAY_S: i64 = 3600;

/// Safety net: stop firing after this many consecutive wakeup turns with no
/// human turn in between, so a self-re-arming loop can't burn the token pool
/// forever. Reset by any human submit (see `Hub::submit`). Generous — a
/// legitimate long `/loop` rarely runs this many iterations unattended.
const MAX_CONSECUTIVE_FIRES: u32 = 100;

/// Command to the scheduler task.
pub enum ScheduleCmd {
    /// Arm (replace) a session's pending wakeup. `fire_at_ms` is epoch ms.
    Arm {
        session_id: String,
        fire_at_ms: i64,
        prompt: String,
    },
    /// A human turn arrived → reset that session's consecutive-fire guard.
    HumanTurn { session_id: String },
}

struct Pending {
    fire_at_ms: i64,
    prompt: String,
}

/// Absolute fire time (epoch ms) for a `delaySeconds`, clamped to the tool's
/// own bounds. Computed at intercept time — the delay runs from when the agent
/// called the tool, which is ≈ now.
#[must_use]
pub fn fire_at_from_delay(delay_seconds: i64) -> i64 {
    now_ms() + delay_seconds.clamp(MIN_DELAY_S, MAX_DELAY_S) * 1000
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Background task: own the pending-wakeup table and fire due ones via
/// `Hub::submit`. Returns when the command channel closes (Hub dropped).
pub async fn run_scheduler(hub: Hub, mut rx: mpsc::UnboundedReceiver<ScheduleCmd>) {
    let mut pending: HashMap<String, Pending> = HashMap::new();
    // Consecutive autonomous fires since the last human turn, per session.
    let mut fires: HashMap<String, u32> = HashMap::new();
    loop {
        let now = now_ms();
        let next_delay = pending
            .values()
            .map(|p| (p.fire_at_ms - now).max(0))
            .min()
            .map(|ms| std::time::Duration::from_millis(u64::try_from(ms).unwrap_or(0)));
        tokio::select! {
            cmd = rx.recv() => match cmd {
                None => return,
                Some(ScheduleCmd::Arm { session_id, fire_at_ms, prompt }) => {
                    pending.insert(session_id, Pending { fire_at_ms, prompt });
                }
                Some(ScheduleCmd::HumanTurn { session_id }) => {
                    fires.remove(&session_id);
                }
            },
            // Only armed when something is pending; fires the soonest-due batch.
            () = async { tokio::time::sleep(next_delay.unwrap_or_default()).await },
                if next_delay.is_some() =>
            {
                let now = now_ms();
                let due: Vec<String> = pending
                    .iter()
                    .filter(|(_, p)| p.fire_at_ms <= now)
                    .map(|(s, _)| s.clone())
                    .collect();
                for sid in due {
                    let Some(p) = pending.remove(&sid) else { continue };
                    // Consumed (whether we fire or cap-drop it) → clear the
                    // persisted record so it can't re-fire on the next restart. A
                    // re-arm during the fired turn re-persists a fresh one.
                    hub.clear_persisted_wakeup(&sid);
                    let n = fires.entry(sid.clone()).or_insert(0);
                    *n += 1;
                    if *n > MAX_CONSECUTIVE_FIRES {
                        tracing::warn!(
                            session = %sid, fires = *n,
                            "scheduler: consecutive-wakeup cap hit; dropping (send a message to resume)",
                        );
                        continue;
                    }
                    tracing::info!(session = %sid, "scheduler: firing scheduled wakeup");
                    // Own turn: idle → dispatch immediately, busy → queue behind
                    // the running turn. The `__wake__` cmid tags the echo so the
                    // UI shows a "↻ scheduled wakeup" note, not a user bubble.
                    let cmid = format!("{WAKEUP_PREFIX}{sid}-{now}");
                    hub.submit(&sid, p.prompt, Vec::new(), Some(cmid));
                }
            }
        }
    }
}
