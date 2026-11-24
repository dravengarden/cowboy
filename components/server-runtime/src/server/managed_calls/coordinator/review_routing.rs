//! Cached quota lookup, capabilities review fields, and review admission routing.

use serde_json::{Value, json};

use super::super::super::{AppState, agent_tools, provider_accounts};
use super::super::selection;
use super::error;
use crate::managed_calls::service::Ledger;
use crate::managed_calls::{Conversation, Request};
use crate::provider_account::ProviderAccountId;

/// Cached quota evidence for review selection: each named default account's
/// own usage read. The retired legacy cards have no Service collector, so a
/// child on a `default` account has no quota evidence.
pub(super) async fn quota_evidence(
    state: &AppState,
    meta: &crate::core::SessionMeta,
    named: &[ProviderAccountId],
) -> Value {
    if !state.product_auth_enabled || meta.owner_user_id.is_some() {
        let now = chrono::Utc::now().timestamp_millis();
        let interval = state
            .usage
            .cached_view(|snapshot, _, _| snapshot.refresh_interval_ms)
            .await;
        let mut accounts = Vec::new();
        if let Some(owner) = meta.owner_user_id.as_deref() {
            for id in named {
                let read = provider_accounts::usage::read_one(&state.accounts, owner, id)
                    .await
                    .unwrap_or_else(|code| json!({"account_id":id,"reason":code}));
                accounts.push(selection::named_quota(&read, now, interval));
            }
        }
        json!({"observed_now_ms":now,"accounts":accounts,"unavailable_accounts":[]})
    } else {
        json!({"status":"permission_denied"})
    }
}

pub(super) async fn account_quota(
    state: &AppState,
    meta: &crate::core::SessionMeta,
    account: &ProviderAccountId,
) -> Option<(Value, i64)> {
    if account.is_default() || (state.product_auth_enabled && meta.owner_user_id.is_none()) {
        return None;
    }
    let owner = meta.owner_user_id.as_deref()?;
    let now = chrono::Utc::now().timestamp_millis();
    let interval = state
        .usage
        .cached_view(|snapshot, _, _| snapshot.refresh_interval_ms)
        .await;
    let read = provider_accounts::usage::read_one(&state.accounts, owner, account)
        .await
        .ok()?;
    Some((selection::named_quota(&read, now, interval), now))
}

/// Whether `agent` is the caller's own vendor, and the reviewer family of the
/// model the caller currently runs.
fn caller_model(
    state: &AppState,
    meta: &crate::core::SessionMeta,
    agent: &str,
) -> (bool, Option<Option<&'static str>>) {
    (
        agent_tools::same_family(agent, &meta.provider),
        selection::caller_family(state.supervisor.observed_configuration(&meta.id).as_ref()),
    )
}

/// A Provider's review view for capabilities: routine order, routing and
/// whether any reviewer is eligible once the caller's own model is excluded.
pub(super) fn review_fields(
    state: &AppState,
    meta: &crate::core::SessionMeta,
    provider: &str,
    runnable: &Value,
    policy: &agent_tools::CallsPolicy,
    available: bool,
    quota: Option<(&Value, i64)>,
) -> Value {
    let caller = caller_model(state, meta, provider);
    let routine =
        selection::candidates_for(runnable, provider, &policy.review, false, caller, quota);
    let high = selection::candidates_for(runnable, provider, &policy.review, true, caller, quota);
    json!({
        "review_candidates": routine.list.iter().filter_map(|p| p["id"].as_str()).collect::<Vec<_>>(),
        "review_routing": selection::routing_view(runnable, provider, &policy.review, caller, quota),
        "analysis_routing": selection::analysis_routing_view(runnable, provider),
        "same_vendor": caller.0,
        // Analysis may run where no reviewer is eligible; a high-risk review
        // may still name an explicit-only reviewer.
        "review_available": available && !(routine.list.is_empty() && high.list.is_empty()),
    })
}

#[allow(clippy::too_many_arguments)] // The admission inputs of one review.
pub(super) async fn route_review(
    state: &AppState,
    meta: &crate::core::SessionMeta,
    ledger: &Ledger,
    request: &Request,
    runnable: &Value,
    target: &mut agent_tools::CallTarget,
    (preference, account): (&agent_tools::ReviewPreferences, &ProviderAccountId),
    preset: Option<&str>,
) -> Result<(), Value> {
    let caller = caller_model(state, meta, &target.agent);
    if let Conversation::Continue { child_session_id } = &request.conversation {
        let child_preset = match ledger.child(child_session_id).await {
            Ok(Some(previous)) => previous.preset,
            Ok(None) => return Err(error("conversation_unavailable", "not_submitted")),
            Err(_) => return Err(error("capacity", "not_submitted")),
        };
        let Ok(rounds) = ledger.child_rounds(child_session_id).await else {
            return Err(error("capacity", "not_submitted"));
        };
        return selection::admit_continuation(
            request,
            runnable,
            &target.agent,
            preference,
            (child_preset.as_deref(), rounds),
            caller.0,
        );
    }
    let q_snap = account_quota(state, meta, account).await;
    let q_ref = q_snap.as_ref().map(|(q, now)| (q, *now));
    target.preset = Some(selection::admit(
        request,
        runnable,
        &target.agent,
        preference,
        preset,
        caller,
        q_ref,
    )?);
    target.options = None;
    Ok(())
}
