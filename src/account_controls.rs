//! Account controls of named-only Providers (Antigravity): the durable
//! per-account state, the one-turn reservation and the settlement rules
//! (`docs/antigravity-real-accounts-design.md`, "Account controls", "Kill
//! switch and logout", amendments R3-4, R3-9, R3-10, R3-16).
//!
//! This module is pure policy over the stored records; the Controller's
//! dispatcher, reconciler and owner actions apply it. Nothing here reads
//! model text: a turn is classified only from the proxy's closed refusal and
//! violation codes in its terminal record.

use serde::{Deserialize, Serialize};

/// The proxy's JSON-RPC error prefix of a backend refusal (`proxy.mjs`).
pub const REFUSAL_PREFIX: &str = "antigravity_backend_refusal:";
/// The proxy's JSON-RPC error prefix of a built-in tool that executed.
pub const VIOLATION_PREFIX: &str = "antigravity_policy_violation:";
/// How long a rate limit holds when the backend gave no reset time.
pub const DEFAULT_RATE_LIMIT_MS: i64 = 60 * 60 * 1000;

/// The durable state of one account. Anything but `Ok` parks every queue of
/// the account; `Suspended`, `Blocked` and a detected policy violation also
/// stop its units on every Machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountState {
    Ok,
    VerificationRequired,
    LocationUnsupported,
    RateLimited,
    Suspended,
    Blocked,
    UnknownRefusal,
    NeedsAttention,
}

impl AccountState {
    pub const ALL: [Self; 8] = [
        Self::Ok,
        Self::VerificationRequired,
        Self::LocationUnsupported,
        Self::RateLimited,
        Self::Suspended,
        Self::Blocked,
        Self::UnknownRefusal,
        Self::NeedsAttention,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::VerificationRequired => "verification_required",
            Self::LocationUnsupported => "location_unsupported",
            Self::RateLimited => "rate_limited",
            Self::Suspended => "suspended",
            Self::Blocked => "blocked",
            Self::UnknownRefusal => "unknown_refusal",
            Self::NeedsAttention => "needs_attention",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.as_str() == value)
    }

    /// The state a proxy backend class settles the account in.
    #[must_use]
    pub fn from_backend_class(class: &str) -> Self {
        match class {
            "ok" => Self::Ok,
            "verification_required" => Self::VerificationRequired,
            "location_unsupported" => Self::LocationUnsupported,
            "rate_limited" => Self::RateLimited,
            "suspended" => Self::Suspended,
            // Any class this Controller does not know is not proof of success.
            _ => Self::UnknownRefusal,
        }
    }

    /// An automatic transition (turn settlement) never replaces a more
    /// severe state; only the owner lowers one.
    #[must_use]
    pub const fn severity(self) -> u8 {
        match self {
            Self::Ok => 0,
            Self::RateLimited => 1,
            Self::VerificationRequired | Self::LocationUnsupported => 2,
            Self::UnknownRefusal => 3,
            Self::NeedsAttention => 4,
            Self::Suspended => 5,
            Self::Blocked => 6,
        }
    }

    /// Whether entering this state stops the account's units.
    #[must_use]
    pub const fn stops_units(self) -> bool {
        matches!(self, Self::Suspended | Self::Blocked)
    }

    /// The owner-facing explanation of a parked queue.
    #[must_use]
    pub const fn describe(self) -> &'static str {
        match self {
            Self::Ok => "ready",
            Self::VerificationRequired => "Google asks to verify this Antigravity account",
            Self::LocationUnsupported => {
                "Google refused this Antigravity account from the runtime Machine's location"
            }
            Self::RateLimited => "this Antigravity account is rate-limited",
            Self::Suspended => "Google reported this Antigravity account as suspended",
            Self::Blocked => "this Antigravity account is blocked by its owner",
            Self::UnknownRefusal => "Google's outcome of the last turn could not be proven",
            Self::NeedsAttention => "this Antigravity account needs the owner's attention",
        }
    }
}

/// One account's durable control record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Control {
    pub account_id: String,
    pub owner_user_id: String,
    pub provider_id: String,
    pub state: AccountState,
    /// Bounded machine-readable reason (a refusal class, violation tool or
    /// owner action); never model text.
    pub reason: String,
    /// The end of a rate limit.
    pub until_ms: Option<i64>,
    pub updated_at_ms: i64,
    pub revision: i64,
}

/// A durable reservation of an account's single active turn (R3-9).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReservationState {
    /// Taken before dispatch; nothing has left the Controller.
    Reserved,
    /// Persisted before the prompt is enqueued to the worker: the backend
    /// may have received it.
    MayHaveDispatched,
}

impl ReservationState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reserved => "reserved",
            Self::MayHaveDispatched => "may_have_dispatched",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "reserved" => Some(Self::Reserved),
            "may_have_dispatched" => Some(Self::MayHaveDispatched),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reservation {
    pub account_id: String,
    pub owner_user_id: String,
    pub session_id: String,
    pub turn_id: String,
    /// The worker incarnation known when the turn was marked; empty while
    /// the dispatch revives the session.
    pub worker_epoch: String,
    /// The session's newest event sequence before this turn: its terminal
    /// record is the first turn end after it.
    pub after_seq: i64,
    pub state: ReservationState,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// A Machine fence the Controller owes the account's daemon.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FenceState {
    Pending,
    Confirmed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fence {
    pub account_id: String,
    pub machine_id: String,
    pub action: crate::provider_account::AccountFenceAction,
    pub operation_id: String,
    pub state: FenceState,
    pub detail: String,
    pub updated_at_ms: i64,
}

/// The idempotent operation ID of one fence: derived from the control
/// revision that requested it, so a retry repeats the same request.
#[must_use]
pub fn fence_operation_id(
    account_id: &str,
    machine_id: &str,
    action: crate::provider_account::AccountFenceAction,
    revision: i64,
) -> String {
    use sha2::Digest as _;
    let scope = serde_json::to_vec(&(account_id, machine_id, action, revision))
        .expect("fence scope serializes");
    let digest = sha2::Sha256::digest(scope);
    let mut id = String::from("account-fence-");
    for byte in &digest[..16] {
        use std::fmt::Write as _;
        let _ = write!(id, "{byte:02x}");
    }
    id
}

/// What a dispatch of an account's prompt may do now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Gate {
    Proceed {
        /// An explicit owner send after a rate limit ended resumes the account.
        clears_rate_limit: bool,
    },
    Park(String),
}

/// Everything the dispatch gate decides on, read before the reservation.
#[derive(Clone, Copy, Debug)]
pub struct GateInput<'a> {
    pub admitted: bool,
    pub control: Option<&'a Control>,
    /// The method and sequence of an unacknowledged uncertain effect.
    pub uncertain: Option<(&'a str, u64)>,
    /// An owner pressed send (or send now); automatic drains are not.
    pub explicit: bool,
    pub now_ms: i64,
}

/// The dispatch gate of one prompt (before its reservation).
#[must_use]
pub fn gate(input: &GateInput<'_>) -> Gate {
    if !input.admitted {
        return Gate::Park(
            "this Antigravity account is not admitted (accounts.antigravity_admitted)".into(),
        );
    }
    let mut clears_rate_limit = false;
    if let Some(control) = input.control {
        match control.state {
            AccountState::Ok => {}
            AccountState::RateLimited => {
                let until = control.until_ms.unwrap_or(i64::MAX);
                if input.now_ms < until {
                    return Gate::Park(format!(
                        "{} until {}",
                        control.state.describe(),
                        format_utc(until)
                    ));
                }
                if !input.explicit {
                    return Gate::Park(
                        "the account's rate limit ended; send again to resume it".into(),
                    );
                }
                clears_rate_limit = true;
            }
            state => return Gate::Park(state.describe().into()),
        }
    }
    if let Some((method, _)) = input.uncertain {
        return Gate::Park(format!(
            "the outcome of a target {method} is unknown; acknowledge it after reviewing the workspace"
        ));
    }
    Gate::Proceed { clears_rate_limit }
}

/// A worker revival of a named-only session: admitted, not rate-limited
/// beyond its end, and otherwise `ok`.
pub fn revival_allowed(
    admitted: bool,
    control: Option<&Control>,
    now_ms: i64,
) -> Result<(), String> {
    match gate(&GateInput {
        admitted,
        control,
        uncertain: None,
        explicit: true,
        now_ms,
    }) {
        Gate::Proceed { .. } => Ok(()),
        Gate::Park(reason) => Err(reason),
    }
}

/// Whether another session's reservation holds the account's slot.
#[must_use]
pub fn slot_reason(reservation: &Reservation, session_id: &str) -> Option<String> {
    (reservation.session_id != session_id).then(|| {
        format!(
            "another turn of this Antigravity account is running (session {})",
            reservation.session_id
        )
    })
}

/// How a reserved turn ended, from Controller evidence only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TurnOutcome {
    /// The turn's terminal record without a refusal: the proxy settled its
    /// backend evidence as `ok` (or the client cancelled it).
    Completed,
    /// The proxy settled a refusal class.
    Refused(AccountState),
    /// The proxy saw a built-in tool execute.
    PolicyViolation(String),
    /// A terminal error with no settled backend evidence.
    Unproven,
    /// Proven loss of the turn's unit; `evidence_complete` only when the
    /// unit's backend evidence settled `ok` before it was lost.
    UnitLost { evidence_complete: bool },
    /// Nothing left the Controller: the dispatch never enqueued the prompt.
    NotDispatched,
}

/// Classify a terminal record's stop reason. Only the proxy's closed codes
/// are read, never the message around them.
#[must_use]
pub fn classify_stop_reason(stop_reason: &str) -> TurnOutcome {
    let code = |prefix: &str| {
        stop_reason.find(prefix).map(|at| {
            stop_reason[at + prefix.len()..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
                .take(128)
                .collect::<String>()
        })
    };
    if let Some(tool) = code(VIOLATION_PREFIX) {
        return TurnOutcome::PolicyViolation(tool);
    }
    if let Some(class) = code(REFUSAL_PREFIX) {
        return match AccountState::from_backend_class(&class) {
            AccountState::Ok => TurnOutcome::Completed,
            state => TurnOutcome::Refused(state),
        };
    }
    if stop_reason.starts_with("error:") {
        TurnOutcome::Unproven
    } else {
        TurnOutcome::Completed
    }
}

/// The effect of settling a reservation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settlement {
    /// Release the account's slot.
    pub release: bool,
    /// The settled outcome name, for receipts and logs.
    pub settled: &'static str,
    /// The account state to enter, with its reason; `None` keeps it.
    pub state: Option<(AccountState, String)>,
    /// Stop every unit of the account.
    pub stop_units: bool,
}

/// Settle `reservation` with `outcome` (R1-8, R3-9, R3-10).
#[must_use]
pub fn settle(state: ReservationState, outcome: &TurnOutcome) -> Settlement {
    let enter = |account: AccountState, reason: &str| Some((account, reason.to_owned()));
    match outcome {
        // Only a reservation that never reached `may_have_dispatched` can
        // settle not_dispatched; anything else keeps the slot.
        TurnOutcome::NotDispatched if state == ReservationState::Reserved => Settlement {
            release: true,
            settled: "not_dispatched",
            state: None,
            stop_units: false,
        },
        TurnOutcome::NotDispatched => Settlement {
            release: false,
            settled: "unsettled",
            state: enter(AccountState::NeedsAttention, "dispatch_unproven"),
            stop_units: false,
        },
        TurnOutcome::Completed => Settlement {
            release: true,
            settled: "completed",
            state: None,
            stop_units: false,
        },
        TurnOutcome::Refused(account) => Settlement {
            release: true,
            settled: "refused",
            state: enter(*account, account.as_str()),
            stop_units: account.stops_units(),
        },
        TurnOutcome::PolicyViolation(tool) => Settlement {
            release: true,
            settled: "policy_violation",
            state: enter(
                AccountState::NeedsAttention,
                &format!("policy_violation:{tool}"),
            ),
            stop_units: true,
        },
        TurnOutcome::Unproven => Settlement {
            release: true,
            settled: "unknown_refusal",
            state: enter(AccountState::UnknownRefusal, "unknown_refusal"),
            stop_units: false,
        },
        TurnOutcome::UnitLost {
            evidence_complete: true,
        } => Settlement {
            release: true,
            settled: "interrupted",
            state: None,
            stop_units: false,
        },
        TurnOutcome::UnitLost {
            evidence_complete: false,
        } => Settlement {
            release: true,
            settled: "unknown_refusal",
            state: enter(AccountState::UnknownRefusal, "unit_lost_without_evidence"),
            stop_units: false,
        },
    }
}

/// Owner actions on an account's controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnerAction {
    /// Return a non-ok, non-blocked account to `ok` (admits its units again
    /// when they were stopped).
    Clear,
    /// The kill switch: durable `blocked` first, then stop every unit.
    Block,
    /// Lift the kill switch.
    Unblock,
    /// Retry the account's unconfirmed Machine fences.
    StopUnits,
    /// Release a turn slot whose settlement cannot be proven; requires a
    /// confirmed account stop.
    ForceRelease,
}

/// The state an owner action moves the account to, or why it is refused.
pub fn owner_transition(
    action: OwnerAction,
    current: AccountState,
) -> Result<Option<AccountState>, &'static str> {
    match (action, current) {
        (OwnerAction::Block, AccountState::Blocked) => Ok(None),
        (OwnerAction::Block, _) => Ok(Some(AccountState::Blocked)),
        (OwnerAction::Unblock, AccountState::Blocked) => Ok(Some(AccountState::Ok)),
        (OwnerAction::Unblock, _) => Err("account_not_blocked"),
        (OwnerAction::Clear, AccountState::Blocked) => Err("account_blocked_use_unblock"),
        (OwnerAction::Clear, AccountState::Ok) => Ok(None),
        (OwnerAction::Clear, _) => Ok(Some(AccountState::Ok)),
        (OwnerAction::StopUnits | OwnerAction::ForceRelease, _) => Ok(None),
    }
}

/// `YYYY-MM-DD HH:MM UTC` for a parked queue's reason.
#[must_use]
pub fn format_utc(ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms).map_or_else(
        || "an unknown time".to_owned(),
        |time| time.format("%Y-%m-%d %H:%M UTC").to_string(),
    )
}

#[cfg(test)]
#[path = "account_controls_tests.rs"]
mod tests;
