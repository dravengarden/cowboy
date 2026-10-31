//! Background-task reconciliation for clients (docs/background-tasks.md §4).
//!
//! A device that missed a subagent's terminal snapshot (offline, a hidden tab,
//! a reconnect gap) holds a cached `running` row the bounded snapshot tail
//! never repairs: the Controller replaces that row in place at its old anchor.
//! The focused bootstrap therefore carries `subagent_state` — every child the
//! Controller knows as non-terminal plus every child observed after the
//! client's `since_seq` — and `POST …/subagents/lookup` resolves any other id
//! the client still renders as working. Call rows stay ledger-authoritative
//! (`/calls`, polled); their transcript anchors are written here.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use axum::{
    Extension, Json,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{AppState, AuthenticatedProductRequest, session_is_visible};
use crate::core::{Event, Hub};
use crate::store::Store;
use crate::transcript_anchors::{
    CALL_ANCHOR_UPDATE, LOOKUP_MAX_IDS, METRICS, SUBAGENT_PREFIX, snapshot_order,
    subagent_terminal, valid_id,
};

/// Calls repaired per Controller start; the rest wait for the next start.
const CALL_ANCHOR_REPAIR_LIMIT: usize = 2_000;

/// How a Call reached admission, for linking its anchor to the tool card that
/// asked for it. Only structured MCP identity links; shell output never does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::server) enum CallOrigin {
    /// `cowboy_execute` `calls.start` / `review.start` with this operation.
    Mcp { action: String, operation: String },
    /// The private gateway (`cowboy call`, `cowboy review` through a shell).
    Gateway,
    /// Recreated at Controller start for a call admitted before its anchor.
    Repaired,
}

impl CallOrigin {
    fn to_json(&self) -> Value {
        match self {
            Self::Mcp { action, operation } => {
                json!({"kind": "mcp", "action": action, "operation": operation})
            }
            Self::Gateway => json!({"kind": "gateway"}),
            Self::Repaired => json!({"kind": "repaired"}),
        }
    }
}

/// Append the one transcript anchor of an accepted Call to its parent. The
/// reducers key it by `call:<id>`, so a repeat replaces the same row.
pub(in crate::server) fn push_call_anchor(
    hub: &Hub,
    parent: &str,
    call_id: &str,
    request_id: &str,
    origin: &CallOrigin,
    approval_request_id: Option<&str>,
) {
    if !valid_id(call_id) {
        return;
    }
    let mut update = json!({
        "sessionUpdate": CALL_ANCHOR_UPDATE,
        "callId": call_id,
        "requestId": request_id,
        "origin": origin.to_json(),
    });
    if let Some(id) = approval_request_id {
        update["approvalRequestId"] = json!(id);
    }
    hub.push(parent, Event::Update { update });
}

/// The newest admission when this database first ran a Controller that writes
/// Call anchors. Calls admitted earlier never had one; repairing them would
/// append old history at the end of their transcripts, so only active ones are
/// anchored.
const ANCHORS_SINCE_SETTING: &str = crate::core::settings_keys::TRANSCRIPT_ANCHORS_CALLS_SINCE;

async fn anchors_since_admission(store: &Store) -> Option<i64> {
    let settings = store.load_settings().await.ok()?;
    if let Some(since) = settings
        .iter()
        .find(|(key, _)| key == ANCHORS_SINCE_SETTING)
        .and_then(|(_, value)| value.as_i64())
    {
        return Some(since);
    }
    let boundary = store.managed_call_admission_high_water().await.ok()?;
    store
        .put_setting(ANCHORS_SINCE_SETTING, &json!(boundary))
        .await
        .ok()?;
    Some(boundary)
}

/// I8: a Call admitted before its anchor was written (a crash between the
/// ledger commit and the push) gets its anchor at the next start.
pub(in crate::server) async fn repair_call_anchors(hub: &Hub, store: &Store) {
    let Some(since) = anchors_since_admission(store).await else {
        tracing::warn!("reading the Call anchor epoch failed; anchors are not repaired");
        return;
    };
    let missing = match store
        .calls_missing_anchor(since, CALL_ANCHOR_REPAIR_LIMIT)
        .await
    {
        Ok(missing) => missing,
        Err(error) => {
            tracing::warn!(%error, "listing calls without transcript anchors failed");
            return;
        }
    };
    for (parent, call_id) in missing {
        if hub
            .session_info(&parent)
            .is_none_or(|info| info.meta.closing)
        {
            continue;
        }
        let request_id = match store.managed_call(&parent, &call_id).await {
            Ok(Some(record)) => record.request_id,
            _ => continue,
        };
        push_call_anchor(
            hub,
            &parent,
            &call_id,
            &request_id,
            &CallOrigin::Repaired,
            None,
        );
        METRICS.call_anchor_repairs.fetch_add(1, Ordering::Relaxed);
    }
}

fn child_of(update: &Value) -> Option<&str> {
    update.get("sessionId").and_then(Value::as_str)
}

/// Merge the store's anchored rows with this process's observations: the
/// higher Controller receipt seq (then revision) wins; the durable anchor seq
/// is preferred because the Hub's may be a process-local position.
fn merge_observations(
    durable: Vec<crate::store::transcript_anchors::AnchoredRow>,
    live: Vec<(u64, Value)>,
) -> HashMap<String, (u64, Value)> {
    let mut merged: HashMap<String, (u64, Value)> = HashMap::new();
    for row in durable {
        let Some(child) = row.key.strip_prefix(SUBAGENT_PREFIX).map(str::to_owned) else {
            continue;
        };
        merged.insert(child, (row.anchor_seq, row.update));
    }
    for (anchor, update) in live {
        let Some(child) = child_of(&update).map(str::to_owned) else {
            continue;
        };
        match merged.get_mut(&child) {
            Some((_, current)) if snapshot_order(current) >= snapshot_order(&update) => {}
            Some((_, current)) => *current = update,
            None => {
                merged.insert(child, (anchor, update));
            }
        }
    }
    merged
}

fn item(anchor_seq: u64, update: Value) -> Value {
    json!({"anchor_seq": anchor_seq, "update": update})
}

/// The bootstrap's authoritative subagent state, or `None` for an unknown
/// session. Store failures yield an incomplete (never an empty-and-complete)
/// state.
pub(in crate::server) async fn subagent_state(
    hub: &Hub,
    store: Option<&Store>,
    session_id: &str,
    since_seq: Option<u64>,
) -> Option<Value> {
    let observations = hub.subagent_observations(session_id)?;
    let observed: HashSet<&str> = observations
        .items
        .iter()
        .filter_map(|(_, update)| child_of(update))
        .collect();
    let wanted = |update: &Value| {
        !subagent_terminal(update)
            || since_seq.is_some_and(|since| snapshot_order(update).0 > since)
    };
    let mut complete = true;
    let durable = match store {
        Some(store) => {
            if let Err(error) = store.backfill_subagent_anchors(session_id).await {
                tracing::warn!(%error, session = %session_id, "subagent anchor backfill failed");
                complete = false;
            }
            let keep = |row: &crate::store::transcript_anchors::AnchoredRow| {
                wanted(&row.update)
                    || child_of(&row.update).is_some_and(|child| observed.contains(child))
            };
            match store.subagent_anchor_rows(session_id, keep).await {
                Ok(rows) => rows,
                Err(error) => {
                    tracing::warn!(%error, session = %session_id, "reading anchored subagents failed");
                    complete = false;
                    Vec::new()
                }
            }
        }
        None => Vec::new(),
    };
    if store.is_none() && !observations.memory_only {
        complete = false;
    }
    let watermark_seq = observations.watermark_seq;
    let incarnation = observations.incarnation.clone();
    let items: Vec<Value> = merge_observations(durable, observations.items.clone())
        .into_values()
        .filter(|(_, update)| wanted(update))
        .map(|(anchor, update)| item(anchor, update))
        .collect();
    Some(json!({
        "session_id": session_id,
        "epoch": hub.boot_epoch(),
        "watermark_seq": watermark_seq,
        "complete": complete,
        "incarnation": incarnation,
        "items": items,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Lookup {
    ids: Vec<String>,
}

/// I6: resolve every id a client still renders as non-terminal. `not_found`
/// means the row no longer exists (cleared or retired) — never finished.
pub(super) async fn lookup(
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    Path(session_id): Path<String>,
    Json(body): Json<Lookup>,
) -> Response {
    if !session_is_visible(&state.hub, &authenticated.principal, &session_id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    if body.ids.is_empty()
        || body.ids.len() > LOOKUP_MAX_IDS
        || body.ids.iter().any(|id| !valid_id(id))
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    match lookup_items(&state.hub, state.store.as_ref(), &session_id, &body.ids).await {
        Ok(Some(value)) => ([(header::CACHE_CONTROL, "no-store")], Json(value)).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(()) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

/// The lookup answer for already validated ids; `Ok(None)` for an unknown
/// session, `Err` when the store cannot answer (never a false `not_found`).
pub(in crate::server) async fn lookup_items(
    hub: &Hub,
    store: Option<&Store>,
    session_id: &str,
    ids: &[String],
) -> Result<Option<Value>, ()> {
    METRICS.lookup_requests.fetch_add(1, Ordering::Relaxed);
    METRICS
        .lookup_ids
        .fetch_add(u64::try_from(ids.len()).unwrap_or(0), Ordering::Relaxed);
    let Some(observations) = hub.subagent_observations(session_id) else {
        return Ok(None);
    };
    let durable = match store {
        Some(store) => {
            if let Err(error) = store.backfill_subagent_anchors(session_id).await {
                tracing::warn!(%error, session = %session_id, "subagent anchor backfill failed");
                return Err(());
            }
            store
                .subagent_anchor_rows_for(session_id, ids)
                .await
                .map_err(|_| ())?
        }
        None => Vec::new(),
    };
    let merged = merge_observations(durable, observations.items);
    let items: Vec<Value> = ids
        .iter()
        .map(|id| match merged.get(id) {
            Some((anchor, update)) => {
                json!({"id": id, "status": "found", "anchor_seq": anchor, "update": update})
            }
            None => json!({"id": id, "status": "not_found"}),
        })
        .collect();
    Ok(Some(json!({
        "schema": 1,
        "epoch": hub.boot_epoch(),
        "incarnation": observations.incarnation,
        "items": items,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newer_observation_wins_and_keeps_the_durable_anchor() {
        let durable = vec![crate::store::transcript_anchors::AnchoredRow {
            key: "subagent:a".into(),
            anchor_seq: 5,
            update: json!({"sessionId":"a","state":"running","observedSeq":5,"revision":1}),
        }];
        let live = vec![(
            900,
            json!({"sessionId":"a","state":"idle","observedSeq":400,"revision":2}),
        )];
        let merged = merge_observations(durable, live);
        assert_eq!(merged["a"].0, 5);
        assert_eq!(merged["a"].1["state"], "idle");
        let older_live = vec![(
            900,
            json!({"sessionId":"a","state":"running","observedSeq":3,"revision":0}),
        )];
        let durable = vec![crate::store::transcript_anchors::AnchoredRow {
            key: "subagent:a".into(),
            anchor_seq: 5,
            update: json!({"sessionId":"a","state":"idle","observedSeq":400,"revision":2}),
        }];
        assert_eq!(
            merge_observations(durable, older_live)["a"].1["state"],
            "idle"
        );
    }
}
