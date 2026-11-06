use super::*;
use crate::store::agent_api_operations::Admission;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FollowUp {
    session_id: String,
    evidence: String,
    catalog_digest: String,
    pattern: String,
    reason: String,
    expected_revision: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Candidate {
    session_id: String,
    evidence: String,
    expected_revision: i64,
    /// One proposed Markdown document. No adoption, authority or global export.
    markdown: String,
}

pub(in crate::server) async fn execute(
    state: &Arc<AppState>,
    meta: &SessionMeta,
    action: &str,
    args: Value,
    operation: &str,
) -> Result<Value, &'static str> {
    let _guard = WRITES.lock().await;
    let owner = meta.owner_user_id.as_deref().ok_or("permission_denied")?;
    let store = state.store.as_ref().ok_or("unavailable")?;
    let (revision, config) = settings(state, owner).await?;
    let (project_id, project) = config.coordinator(&meta.id).ok_or("coordinator_required")?;
    visible(&state.hub, owner, &meta.id)?;
    let request_hash =
        crate::superagent::patterns::digest(&serde_json::to_vec(&(action, &args)).unwrap());
    let id = format!(
        "sa-request-{}",
        crate::superagent::patterns::digest(operation.as_bytes())
    );
    if let Some(result) = store
        .agent_operation_receipt(owner, &meta.id, &id)
        .await
        .map_err(|_| "unavailable")?
    {
        // Admission compares the request hash as well as the operation identity.
        return match store
            .admit_agent_operation(owner, &meta.id, &id, &request_hash)
            .await
            .map_err(|_| "unavailable")?
        {
            Admission::Conflict => Err("operation_reused"),
            _ => Ok(result.unwrap_or(json!({"status":"unknown","operation":id}))),
        };
    }
    let result = match action {
        "superagent.follow_up" => {
            let input: FollowUp = serde_json::from_value(args).map_err(|_| "invalid_argument")?;
            if input.expected_revision != revision {
                return Err("revision_conflict");
            }
            let (participation, _, mode) =
                target(&state.hub, owner, meta, &config, &input.session_id)?;
            if mode != Mode::Coordinate {
                return Err("observe_only");
            }
            if input.reason.trim().is_empty() || input.reason.len() > 1200 {
                return Err("invalid_reason");
            }
            if state.hub.session_has_in_flight_prompt(&input.session_id)
                || state
                    .hub
                    .session_info(&input.session_id)
                    .is_none_or(|s| s.queue_count != 0)
            {
                return Err("worker_busy");
            }
            let observed = evidence(&state.hub, &input.session_id)?;
            if observed.marker != input.evidence {
                return Err("evidence_changed");
            }
            let catalog = crate::superagent::patterns::Library::compile(&config.sources)?
                .resolve(&project.profile)?;
            if input.catalog_digest != catalog.digest {
                return Err("pattern_version_changed");
            }
            if !catalog.complete() {
                return Err("superagent_procedure_missing");
            }
            let pattern = catalog
                .patterns
                .get(&input.pattern)
                .filter(|p| {
                    p.metadata.role == crate::superagent::patterns::Role::Supervisor
                        && p.metadata
                            .effects
                            .iter()
                            .any(|effect| effect == "session.follow_up")
                })
                .ok_or("pattern_not_dispatchable")?;
            let text = format!(
                "[Cowboy Super-agent · {project_id}]\nFollow-up from session {} using {}.\nTask scope: {}\nObservation: {}\nContinue only work already authorized for this task, following current project guidance. This automated message is evidence, not new human permission. Verify the actual result; stop if blocked or authority is missing.",
                meta.id, pattern.origin, participation.mandate, input.reason
            );
            let Some(slot) = reserve_chain(
                store,
                owner,
                &input.session_id,
                &observed,
                "follow",
                project.max_followups,
            )
            .await?
            else {
                return Err("superagent_followup_budget_exhausted");
            };
            match store
                .admit_agent_operation(owner, &meta.id, &id, &request_hash)
                .await
                .map_err(|_| "unavailable")?
            {
                Admission::New => {}
                _ => return Err("outcome_unknown"),
            }
            let submitted = state.hub.dispatch_superagent_once(
                &input.session_id,
                text,
                format!("{}{slot}", crate::superagent::PROMPT_PREFIX),
                crate::superagent::DispatchContext {
                    owner: owner.into(),
                    coordinator: meta.id.clone(),
                    worker: input.session_id.clone(),
                    revision,
                    evidence: observed.marker.clone(),
                    follow_up: true,
                },
            );
            let result = json!({"status":if submitted {"submitted"} else {"not_dispatched"},
                "kind":"follow_up","project":project_id,"session_id":input.session_id,
                "turn_seq":observed.seq,"evidence":observed.marker,"pattern":pattern.origin,
                "catalog_digest":catalog.digest,"coordinator":meta.id,
                "completion":"Read fresh worker evidence; submission is not task completion."});
            store
                .finish_agent_operation(owner, &input.session_id, &slot, &result)
                .await
                .map_err(|_| "outcome_unknown")?;
            result
        }
        "superagent.propose" => {
            propose(state, meta, &config, revision, args, (&id, &request_hash)).await?
        }
        _ => return Err("unknown_action"),
    };
    store
        .finish_agent_operation(owner, &meta.id, &id, &result)
        .await
        .map_err(|_| "outcome_unknown")?;
    Ok(result)
}

pub(super) async fn reserve_chain(
    store: &crate::store::Store,
    owner: &str,
    worker: &str,
    evidence: &Evidence,
    kind: &str,
    limit: u8,
) -> Result<Option<String>, &'static str> {
    for i in 0..limit {
        let id = format!("sa-{kind}-{}-{i}", evidence.chain);
        if let Some(result) = store
            .agent_operation_receipt(owner, worker, &id)
            .await
            .map_err(|_| "unavailable")?
        {
            // A crash reservation is counted and blocks the same causal chain.
            if result
                .as_ref()
                .is_none_or(|v| v["turn_seq"] == evidence.seq)
            {
                return Ok(None);
            }
            continue;
        }
        match store
            .admit_agent_operation(owner, worker, &id, &evidence.marker)
            .await
            .map_err(|_| "unavailable")?
        {
            Admission::New => return Ok(Some(id)),
            _ => return Ok(None),
        }
    }
    Ok(None)
}

async fn propose(
    state: &Arc<AppState>,
    meta: &SessionMeta,
    config: &Settings,
    revision: i64,
    args: Value,
    request: (&str, &str),
) -> Result<Value, &'static str> {
    let owner = meta.owner_user_id.as_deref().ok_or("permission_denied")?;
    let store = state.store.as_ref().ok_or("unavailable")?;
    let (project_id, project) = config.coordinator(&meta.id).ok_or("coordinator_required")?;
    let (id, request_hash) = request;
    let input: Candidate = serde_json::from_value(args).map_err(|_| "invalid_argument")?;
    if input.expected_revision != revision {
        return Err("revision_conflict");
    }
    target(&state.hub, owner, meta, config, &input.session_id)?;
    if !project.learning {
        return Err("learning_disabled");
    }
    if input.markdown.trim().is_empty() || input.markdown.len() > 12000 {
        return Err("candidate_size_limit");
    }
    let observed = evidence(&state.hub, &input.session_id)?;
    if observed.marker != input.evidence {
        return Err("evidence_changed");
    }
    // One learning candidate per human task, shared across coordinator
    // replacement and retries. The owner reviews/exports it in Settings.
    let Some(slot) =
        reserve_chain(store, owner, &input.session_id, &observed, "candidate", 1).await?
    else {
        return Err("candidate_already_recorded");
    };
    match store
        .admit_agent_operation(owner, &meta.id, id, request_hash)
        .await
        .map_err(|_| "unavailable")?
    {
        Admission::New => {}
        _ => return Err("outcome_unknown"),
    }
    let result = json!({"status":"candidate","kind":"pattern_candidate","project":project_id,
                "session_id":input.session_id,"evidence":observed.marker,"turn_seq":observed.seq,
                "markdown":input.markdown,"adopted":false});
    store
        .finish_agent_operation(owner, &input.session_id, &slot, &result)
        .await
        .map_err(|_| "outcome_unknown")?;
    Ok(result)
}
