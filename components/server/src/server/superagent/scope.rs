use super::*;

#[derive(Clone)]
pub(super) struct Evidence {
    pub(super) marker: String,
    pub(super) chain: String,
    pub(super) seq: u64,
    pub(super) text: String,
}

pub(super) fn visible(
    hub: &crate::core::Hub,
    owner: &str,
    id: &str,
) -> Result<SessionMeta, &'static str> {
    if hub.is_managed_child(id) {
        return Err("managed_child_not_eligible");
    }
    hub.session_info(id)
        .map(|i| i.meta)
        .filter(|m| {
            m.owner_user_id.as_deref() == Some(owner)
                && !m.system
                && !m.closing
                && m.trashed_at_ms.is_none()
        })
        .ok_or("session_not_available")
}

pub(super) fn live(meta: &SessionMeta) -> bool {
    !meta.paused && meta.status == crate::core::Status::Running && meta.attention.is_empty()
}

pub(super) fn same_machine(a: &SessionMeta, b: &SessionMeta) -> bool {
    machine(a).is_some() && machine(a) == machine(b)
}

fn machine(m: &SessionMeta) -> Option<String> {
    m.execution_binding.as_ref().map_or_else(
        || Some(m.machine_id.clone()),
        |binding| binding.decode().ok().map(|b| b.environment.machine_id),
    )
}

/// A remote agent cannot use preference writes to steer a different Machine.
/// Library edits affect every bound coordinator; unrelated settings stay editable.
pub(super) fn editable(
    hub: &crate::core::Hub,
    colocated: &std::collections::BTreeSet<String>,
    actor: &SessionMeta,
    before: &Settings,
    after: &Settings,
) -> Result<(), &'static str> {
    let own = machine(actor).ok_or("machine_scope_denied")?;
    if colocated.contains(&own) {
        return Ok(());
    }
    let mut affected = std::collections::BTreeSet::new();
    for id in before.projects.keys().chain(after.projects.keys()) {
        if before.sources != after.sources || before.projects.get(id) != after.projects.get(id) {
            for config in [before, after] {
                if let Some(project) = config.projects.get(id) {
                    affected.insert(project.coordinator.as_str());
                }
            }
        }
    }
    for id in before.sessions.keys().chain(after.sessions.keys()) {
        if before.sessions.get(id) != after.sessions.get(id) {
            affected.insert(id.as_str());
            for config in [before, after] {
                if let Some((_, project, _)) = config.effective(id) {
                    affected.insert(project.coordinator.as_str());
                }
            }
        }
    }
    for id in affected {
        let target = hub.session_info(id).ok_or("machine_scope_denied")?;
        if !same_machine(actor, &target.meta) {
            return Err("machine_scope_denied");
        }
    }
    Ok(())
}

fn human_prompt(e: &crate::core::Envelope) -> bool {
    crate::prompt_origin::origin_from_cmid(e.cmid.as_deref()).is_human()
        && matches!(&e.event, crate::core::Event::Update { update }
            if update["sessionUpdate"] == "user_message_chunk"
            && crate::prompt_origin::is_human_prompt_update(update))
}

pub(super) async fn current_evidence(state: &AppState, id: &str) -> Result<Evidence, &'static str> {
    history_evidence(&state.hub, state.store.as_ref(), id).await
}

pub(super) async fn history_evidence(
    hub: &crate::core::Hub,
    store: Option<&crate::store::Store>,
    id: &str,
) -> Result<Evidence, &'static str> {
    let info = hub.session_info(id).ok_or("not_found")?;
    let (mut log, _) = hub.snapshot(id).ok_or("not_found")?;
    if !log.iter().any(human_prompt) {
        let store = store.ok_or("unavailable")?;
        let mut before = log.first().map_or(info.event_count, |e| e.seq);
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let (page, next, done) =
                tokio::time::timeout_at(deadline, store.history_page(id, before, 200))
                    .await
                    .map_err(|_| "causal_history_scan_timeout")?
                    .map_err(|_| "causal_history_unavailable")?;
            if !log
                .iter()
                .any(|e| matches!(e.event, crate::core::Event::TurnEnd { .. }))
                && let Some(end) = page
                    .iter()
                    .rev()
                    .find(|e| matches!(e.event, crate::core::Event::TurnEnd { .. }))
            {
                log.insert(0, end.clone());
            }
            if let Some(human) = page.iter().rev().find(|e| human_prompt(e)) {
                log.insert(0, human.clone());
                break;
            }
            if done {
                return Err("causal_history_unavailable");
            }
            let next = next
                .filter(|next| *next < before)
                .ok_or("causal_history_unavailable")?;
            before = next;
        }
    }
    let current = hub.session_info(id).ok_or("not_found")?;
    if current.event_count != info.event_count
        || current.meta.agent_session_id != info.meta.agent_session_id
    {
        return Err("evidence_changed");
    }
    evidence_from_log(&info, id, &log)
}

#[cfg(test)]
pub(super) fn evidence(hub: &crate::core::Hub, id: &str) -> Result<Evidence, &'static str> {
    let info = hub.session_info(id).ok_or("not_found")?;
    let (log, _) = hub.snapshot(id).ok_or("not_found")?;
    evidence_from_log(&info, id, &log)
}

fn evidence_from_log(
    info: &crate::core::SessionInfo,
    id: &str,
    log: &[crate::core::Envelope],
) -> Result<Evidence, &'static str> {
    let native = info
        .meta
        .agent_session_id
        .as_ref()
        .ok_or("native_session_unknown")?;
    let human = log
        .iter()
        .rev()
        .find(|e| human_prompt(e))
        .ok_or("causal_history_unavailable")?;
    let end = log
        .iter()
        .rev()
        .find(|e| matches!(e.event, crate::core::Event::TurnEnd { .. }))
        .filter(|e| e.seq > human.seq)
        .ok_or("completed_turn_required")?;
    // Native workers persist the ACP enum's Debug spelling; older records use
    // its wire spelling. Neither cancellation nor an unknown reason completes work.
    if !matches!(&end.event, crate::core::Event::TurnEnd { stop_reason, .. } if matches!(stop_reason.as_str(), "end_turn" | "EndTurn"))
    {
        return Err("turn_not_completed");
    }
    let chain =
        crate::superagent::patterns::digest(format!("{id}/{native}/{}", human.seq).as_bytes());
    let marker =
        crate::superagent::patterns::digest(format!("{chain}/{}", info.event_count).as_bytes());
    // The hot tail is evidence, not a second memory/transcript store.
    let text = log
        .iter()
        .rev()
        .take(80)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .filter_map(|event| match &event.event {
            crate::core::Event::Update { update } => update
                .pointer("/content/text")
                .and_then(Value::as_str)
                .map(|text| {
                    format!(
                        "{} {}: {}\n",
                        event.seq,
                        update["sessionUpdate"].as_str().unwrap_or("update"),
                        text
                    )
                }),
            crate::core::Event::TurnEnd { stop_reason, .. } => {
                Some(format!("{} turn_end: {stop_reason}\n", event.seq))
            }
            _ => None,
        })
        .collect::<String>();
    Ok(Evidence {
        marker,
        chain,
        seq: end.seq,
        text,
    })
}

pub(super) fn target<'a>(
    hub: &crate::core::Hub,
    owner: &str,
    parent: &SessionMeta,
    config: &'a Settings,
    id: &str,
) -> Result<
    (
        &'a crate::superagent::Participation,
        &'a crate::superagent::Project,
        Mode,
    ),
    &'static str,
> {
    let binding = config.effective(id).ok_or("session_not_enrolled")?;
    if binding.2 == Mode::Off || binding.1.coordinator != parent.id {
        return Err("superagent_scope_denied");
    }
    let worker = visible(hub, owner, id)?;
    let current = visible(hub, owner, &parent.id)?;
    if !live(&worker)
        || current.paused
        || !matches!(
            current.status,
            crate::core::Status::Running | crate::core::Status::Busy
        )
        || !same_machine(&current, &worker)
    {
        return Err("superagent_session_held");
    }
    Ok(binding)
}
