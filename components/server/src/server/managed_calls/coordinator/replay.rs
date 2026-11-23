//! A repeated request id observes the original call.

use std::sync::Arc;

use serde_json::Value;

use super::{envelope, error, wait_started};
use crate::managed_calls::Request;
use crate::managed_calls::lifecycle::Record;
use crate::managed_calls::service::Ledger;

use super::super::super::AppState;
use super::super::super::agent_tools;

/// A repeated request id names the same call: an explicit agent must match,
/// and Auto matches a call Auto chose.
fn same_request(original: &Record, requested: &str, preset: Option<&str>) -> bool {
    if preset.is_some_and(|preset| original.preset.as_deref() != Some(preset)) {
        return false;
    }
    if requested == agent_tools::AUTO {
        original.selection.as_deref() == Some(agent_tools::AUTO)
    } else {
        original.provider == requested
    }
}

/// The original call of a repeated request id, even if the Provider has since
/// become unavailable or the policy changed; conflicting inputs are refused.
pub(super) async fn replay(
    state: &Arc<AppState>,
    ledger: &Ledger,
    original: &Record,
    (requested, preset): (&str, Option<&str>),
    request: &Request,
    deadline: tokio::time::Instant,
) -> Value {
    if original.request_digest != request.digest() || !same_request(original, requested, preset) {
        return error("request_conflict", "not_submitted");
    }
    super::super::runner::ensure_runner(state, original);
    envelope(&wait_started(state, ledger, original, deadline).await)
}
