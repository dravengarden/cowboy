//! Final admission at native handoff, after the Controller channel delay.
use super::*;
use crate::store::agent_api_operations::Admission;
use crate::superagent::DispatchContext;

async fn admitted(state: &AppState, context: &DispatchContext) -> Result<(), &'static str> {
    let (revision, config) = settings(state, &context.owner).await?;
    if revision != context.revision {
        return Err("superagent_settings_changed");
    }
    let coordinator = visible(&state.hub, &context.owner, &context.coordinator)?;
    let (_, project, mode) = target(
        &state.hub,
        &context.owner,
        &coordinator,
        &config,
        &context.worker,
    )?;
    if context.follow_up && mode != Mode::Coordinate {
        return Err("observe_only");
    }
    model_ready(state, &coordinator, project.preset)?;
    if !context.follow_up {
        wake::quota_ready(state, &context.owner, &coordinator).await?;
    }
    if evidence(&state.hub, &context.worker)?.marker != context.evidence {
        return Err("evidence_changed");
    }
    let owner = state
        .store
        .as_ref()
        .ok_or("unavailable")?
        .user_by_id(&context.owner)
        .await
        .map_err(|_| "unavailable")?
        .ok_or("permission_denied")?;
    if owner.disabled_at_ms.is_some()
        || !super::super::product_principal(&state.hub, &owner).can_reorder()
    {
        return Err("permission_denied");
    }
    Ok(())
}

pub(in crate::server) async fn dispatch(state: Arc<AppState>, req: crate::core::DispatchReq) {
    let context = req
        .ownership
        .supervision()
        .expect("typed supervision dispatch");
    let Some(cmid) = req.cmid.as_deref() else {
        return;
    };
    let Some(store) = state.store.as_ref() else {
        state.hub.abandon_superagent_prompt(&req.session_id, cmid);
        return;
    };
    let _guard = WRITES.lock().await;
    let operation = format!(
        "sa-delivery-{}",
        crate::superagent::patterns::digest(cmid.as_bytes())
    );
    let new = store
        .admit_agent_operation(
            &context.owner,
            &context.worker,
            &operation,
            &context.evidence,
        )
        .await;
    if !matches!(new, Ok(Admission::New)) {
        state.hub.abandon_superagent_prompt(&req.session_id, cmid);
        return;
    }
    let result = match admitted(&state, context).await {
        Err(reason) => Err(reason.to_owned()),
        Ok(()) => {
            let key = super::super::provider_fence_key_for_session(&state.hub, &req.session_id);
            let fences = state.plugin_lifecycle_fences.read();
            if key
                .as_ref()
                .and_then(|key| fences.get(key))
                .is_some_and(|s| *s != PluginFenceState::Installing)
            {
                Err("superagent_provider_held".into())
            } else {
                state
                    .supervisor
                    .send_superagent_existing(context, &req.text, cmid.to_owned())
            }
        }
    };
    if result.is_err() {
        state.hub.abandon_superagent_prompt(&req.session_id, cmid);
    }
    let outcome = json!({"kind":"delivery","status":if result.is_ok(){"sent"}else{"not_sent"},
        "session_id":context.worker,"coordinator":context.coordinator,"evidence":context.evidence,
        "reason":result.err(),"completion":"Native task completion requires fresh evidence."});
    if let Err(error) = store
        .finish_agent_operation(&context.owner, &context.worker, &operation, &outcome)
        .await
    {
        tracing::warn!(%error, "superagent delivery receipt remains unknown");
    }
}
