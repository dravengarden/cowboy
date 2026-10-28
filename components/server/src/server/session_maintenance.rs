//! One admission boundary for foreground wakes, reclaim and Provider updates.
//! The Hub's durable queue owns waiting messages; this module owns no messages.
use super::*;
use crate::config::schema::{
    PLUGIN_AUTO_UPDATE_IDLE_SESSIONS, SESSIONS_MEMORY_RECLAIM, SESSIONS_RECLAIM_ON_CAPACITY,
};
use crate::core::settings_keys::{MACHINE_MAINTENANCE_PREFIX, SESSION_RESOURCE_WAIT_PREFIX};
use crate::machine_protocol::MachineCapacity;

pub(super) const WAIT_PREFIX: &str = "Waiting for resources: ";
pub(super) const MACHINE_CONNECTION_WAIT: &str = "waiting for Machine connection";
/// The wait of an admitted prompt whose execution Machine is offline or booting.
pub(super) const TARGET_WAIT_REASON: &str = "the execution Machine is not ready";
const WAKE_HEADROOM: u64 = 1 << 30;

#[derive(Default)]
pub(super) struct Coordinator {
    capacities: parking_lot::RwLock<HashMap<String, MachineCapacity>>,
    reclaiming: parking_lot::Mutex<std::collections::HashSet<String>>,
}

impl Coordinator {
    pub(super) fn observe_capacity(&self, machine: &str, capacity: MachineCapacity) {
        self.capacities.write().insert(machine.to_owned(), capacity);
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Policy {
    pub memory_reclaim: Option<bool>,
    pub reclaim_on_capacity: Option<bool>,
    pub auto_update_idle_sessions: Option<bool>,
}

pub(super) fn policy(hub: &Hub, config: &crate::config::Handle, machine: &str) -> Policy {
    let saved: Policy = hub
        .setting(&format!("{MACHINE_MAINTENANCE_PREFIX}{machine}"))
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_default();
    Policy {
        memory_reclaim: Some(
            saved
                .memory_reclaim
                .unwrap_or_else(|| config.get(&SESSIONS_MEMORY_RECLAIM)),
        ),
        reclaim_on_capacity: Some(
            saved
                .reclaim_on_capacity
                .unwrap_or_else(|| config.get(&SESSIONS_RECLAIM_ON_CAPACITY)),
        ),
        auto_update_idle_sessions: Some(
            saved
                .auto_update_idle_sessions
                .unwrap_or_else(|| config.get(&PLUGIN_AUTO_UPDATE_IDLE_SESSIONS)),
        ),
    }
}

pub(super) fn wait(hub: &Hub, session: &str, reason: &str) {
    hub.set_resource_wait(session, Some(reason.to_owned()));
    let key = format!("{SESSION_RESOURCE_WAIT_PREFIX}{session}");
    let current = hub.setting(&key);
    let since = current
        .as_ref()
        .and_then(|v| v.get("since_ms").and_then(serde_json::Value::as_i64))
        .unwrap_or_else(now_ms);
    let mut value = serde_json::json!({"reason": reason, "since_ms": since});
    if let Some(explicit) = current
        .as_ref()
        .and_then(|v| v.get("explicit").and_then(serde_json::Value::as_bool))
    {
        value["explicit"] = serde_json::Value::Bool(explicit);
    }
    if hub.setting(&key).as_ref() != Some(&value) {
        hub.set_setting(key, value);
    }
}

fn waiting(hub: &Hub, session: &str) -> Option<(i64, String)> {
    let value = hub.setting(&format!("{SESSION_RESOURCE_WAIT_PREFIX}{session}"))?;
    Some((
        value.get("since_ms")?.as_i64()?,
        value.get("reason")?.as_str()?.to_owned(),
    ))
}

fn resource_block(state: &AppState, machine: &str) -> Option<&'static str> {
    if machine == "local" {
        return None;
    }
    if !state
        .runtime_router
        .runtime(machine)
        .is_some_and(|runtime| runtime.connected())
    {
        return Some(MACHINE_CONNECTION_WAIT);
    }
    let capacities = state.session_maintenance.capacities.read();
    let Some(capacity) = capacities.get(machine) else {
        return Some("checking Machine capacity");
    };
    if capacity.draining {
        return Some("Machine is draining");
    }
    if session_reclaim::occupied(state, machine) >= capacity.max_sessions as usize {
        return Some("all worker slots are occupied");
    }
    memory_wait(state, machine)
}

/// A prompt waiting for its rebooting execution Machine is only retried once
/// that Machine is connected again; until then the queue is left untouched.
fn execution_target_connected(state: &AppState, meta: &crate::core::SessionMeta) -> bool {
    let Some(record) = &meta.execution_binding else {
        return true;
    };
    let Some(target) = record
        .decode()
        .ok()
        .map(|binding| binding.environment.machine_id)
        .or_else(|| {
            record
                .recovery()
                .map(|intent| intent.previous.environment.machine_id)
        })
    else {
        return true;
    };
    state.machine_control.operation_connection(&target).is_ok()
}

pub(super) fn clear_wait(hub: &Hub, session: &str) {
    hub.set_resource_wait(session, None);
    hub.set_explicit_wait(session, false);
    let key = format!("{SESSION_RESOURCE_WAIT_PREFIX}{session}");
    if hub.setting(&key).is_some() {
        hub.set_setting(key, serde_json::Value::Null);
    }
}

pub(super) fn has_waiters(state: &AppState, machine: &str) -> bool {
    state.hub.session_list().iter().any(|s| {
        s.machine_id == machine
            // Waiting for an execution Machine needs no runtime worker slot.
            && waiting(&state.hub, &s.id).is_some_and(|(_, reason)| reason != TARGET_WAIT_REASON)
            && state.hub.session_info(&s.id).is_some_and(|i| {
                i.queue_count > 0 && (!i.meta.paused || state.hub.explicit_wait(&s.id))
            })
    })
}

/// Missing observations on an old/non-Linux Machine retain its slot guard.
/// Once a Machine reports memory, a stale observation cannot admit a burst.
pub(super) fn memory_wait(state: &AppState, machine: &str) -> Option<&'static str> {
    let observed = state.machine_control.host_resources(machine)?;
    if now_ms().saturating_sub(observed.observed_at_ms) > 60_000 {
        return Some("waiting for a fresh memory observation");
    }
    let resources = &observed.resources;
    let headroom = resources
        .memory_headroom_bytes
        .unwrap_or(resources.memory_available_bytes)
        .min(resources.memory_available_bytes);
    // Starting workers still have allocations ahead of them.
    let starting = state
        .hub
        .session_list()
        .iter()
        .filter(|s| {
            s.machine_id == machine
                && s.status == Status::Starting
                && holds_worker_slot(&state.runtime_router, s)
        })
        .count() as u64;
    (headroom < WAKE_HEADROOM.saturating_add(starting.saturating_mul(512 << 20)))
        .then_some("waiting for memory; idle sessions can release resources")
}

fn admit(state: &Arc<AppState>, session: &str) -> Result<Box<dyn Send>, String> {
    let info = state
        .hub
        .session_info(session)
        .ok_or("Session no longer exists")?;
    let machine = &info.meta.machine_id;
    if machine == "local" {
        return Ok(Box::new(()));
    }
    let blocked = |reason: &str| format!("{WAIT_PREFIX}{reason}");
    let guard = session_reclaim::decisions(machine)
        .try_lock_owned()
        .map_err(|_| blocked("maintenance is finishing"))?;
    if let Some(reason) = resource_block(state, machine) {
        return Err(blocked(reason));
    }
    session_reclaim::recent_wakes(Some(session));
    Ok(Box::new(guard))
}

/// Explicit reloads share the maintenance fence. Replacing an existing worker
/// needs no additional slot; initializing a dormant one does.
pub(super) fn reload_lease(state: &Arc<AppState>, session: &str) -> Result<Box<dyn Send>, String> {
    let info = state
        .hub
        .session_info(session)
        .ok_or("Session no longer exists")?;
    if !holds_worker_slot(&state.runtime_router, &info.meta) {
        return admit(state, session);
    }
    session_reclaim::decisions(&info.meta.machine_id)
        .try_lock_owned()
        .map(|guard| Box::new(guard) as Box<dyn Send>)
        .map_err(|_| format!("{WAIT_PREFIX}maintenance is finishing"))
}

pub(super) fn start(state: &Arc<AppState>) {
    let weak = Arc::downgrade(state);
    state
        .supervisor
        .set_wake_admission(Arc::new(move |session| {
            let state = weak.upgrade().ok_or("Controller is stopping")?;
            admit(&state, session)
        }));
    let weak = Arc::downgrade(state);
    state
        .supervisor
        .set_recovery_admission(Arc::new(move |session| {
            let state = weak.upgrade().ok_or("Controller is stopping")?;
            reload_lease(&state, session)
        }));
    let weak = Arc::downgrade(state);
    tokio::spawn(async move {
        // Seed offline Machines once. Accepted handshakes carry subsequent
        // capacity changes; do not reload full Plugin inventories every tick.
        if let Some(state) = weak.upgrade()
            && let Some(store) = &state.store
            && let Ok(machines) = store.list_machines().await
        {
            let mut capacities = state.session_maintenance.capacities.write();
            for machine in machines.into_iter().filter(|machine| !machine.revoked) {
                if let Some(capacity) = machine
                    .inventory
                    .get("capacity")
                    .and_then(|value| serde_json::from_value(value.clone()).ok())
                {
                    capacities.entry(machine.id).or_insert(capacity);
                }
            }
        }
        let mut last_projection = std::collections::BTreeMap::new();
        loop {
            let Some(state) = weak.upgrade() else { return };
            if *state.shutdown.borrow() {
                return;
            }
            let sessions = state.hub.session_list();
            let mut projection = std::collections::BTreeMap::<String, usize>::new();
            for session in &sessions {
                if holds_worker_slot(&state.runtime_router, session) {
                    *projection.entry(session.machine_id.clone()).or_default() += 1;
                }
            }
            if projection != last_projection {
                state.machine_snapshots.publish().await;
                last_projection = projection;
            }
            let mut waiting_sessions = sessions
                .iter()
                .filter_map(|meta| {
                    waiting(&state.hub, &meta.id).map(|(since, reason)| (since, meta, reason))
                })
                .collect::<Vec<_>>();
            waiting_sessions.sort_by_key(|(since, meta, _)| (*since, meta.id.as_str()));
            let mut resumed = std::collections::HashSet::new();
            for (_, meta, reason) in waiting_sessions {
                let target_wait = reason == TARGET_WAIT_REASON;
                state.hub.set_resource_wait(&meta.id, Some(reason));
                let Some(info) = state.hub.session_info(&meta.id) else {
                    continue;
                };
                if info.queue_count == 0 && !state.hub.session_has_in_flight_prompt(&meta.id) {
                    clear_wait(&state.hub, &meta.id);
                    continue;
                }
                // An explicit send keeps its intent across a transient wait:
                // it bypasses Pause and revives a crashed session once, as the
                // original send would have. An admitted send to a crashed or
                // interrupted session that only waits for its execution Machine
                // still revives once; any other wait in those states stops
                // automatic delivery.
                let explicit = state.hub.explicit_wait(&meta.id);
                if matches!(meta.status, Status::Crashed | Status::Interrupted)
                    && !target_wait
                    && !explicit
                {
                    clear_wait(&state.hub, &meta.id);
                    continue;
                }
                if (!meta.paused || explicit)
                    && (target_wait
                        || explicit
                        || matches!(meta.status, Status::Exited | Status::Running))
                {
                    // Avoid popping/re-persisting the entire message on every
                    // blocked tick. The dispatcher still atomically rechecks.
                    if !execution_target_connected(&state, meta) {
                        continue;
                    }
                    if !holds_worker_slot(&state.runtime_router, meta) {
                        if let Some(reason) = resource_block(&state, &meta.machine_id) {
                            wait(&state.hub, &meta.id, reason);
                            continue;
                        }
                        if session_reclaim::decisions(&meta.machine_id)
                            .try_lock_owned()
                            .is_err()
                            || !resumed.insert(meta.machine_id.clone())
                        {
                            continue;
                        }
                    }
                    if explicit {
                        state.hub.drain_now(&meta.id);
                    } else {
                        state.hub.retry_resource_wait(&meta.id);
                    }
                }
            }
            let capacities = state.session_maintenance.capacities.read().clone();
            for (machine, capacity) in capacities {
                let woken = session_reclaim::recent_wakes(None);
                let eligible = session_reclaim::candidates(&state, &machine)
                    .into_iter()
                    .filter(|candidate| !woken.contains(&candidate.id))
                    .collect::<Vec<_>>();
                if capacity.draining
                    || !has_waiters(&state, &machine)
                    || policy(&state.hub, &state.service_config, &machine).reclaim_on_capacity
                        != Some(true)
                    || session_reclaim::occupied(&state, &machine) < capacity.max_sessions as usize
                    || session_reclaim::choose(
                        &eligible,
                        state
                            .service_config
                            .get(&crate::config::schema::SESSIONS_RECLAIM_MIN_IDLE),
                    )
                    .is_none()
                    || !state
                        .session_maintenance
                        .reclaiming
                        .lock()
                        .insert(machine.clone())
                {
                    continue;
                }
                let state = Arc::clone(&state);
                tokio::spawn(async move {
                    // Uses the same Machine decision lock as admission, memory
                    // reclaim and updates; the actual dispatch checks again.
                    let _ = session_reclaim::reserve_slot(&state, &machine, &capacity).await;
                    state.session_maintenance.reclaiming.lock().remove(&machine);
                });
            }
            drop(state);
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    });
}

pub(super) async fn inventory(
    State(state): State<Arc<AppState>>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
) -> Response {
    let machines = match state.machine_snapshots.load().await {
        Ok(m) => m,
        Err(e) => return (StatusCode::SERVICE_UNAVAILABLE, e.to_string()).into_response(),
    };
    let sessions = state
        .hub
        .session_list_filtered(|owner| authenticated.principal.can_see(owner));
    Json(machines.into_iter().map(|machine| {
        let rows: Vec<_> = sessions.iter().filter(|s| s.machine_id == machine.id).collect();
        let waiting: Vec<_> = rows.iter().filter_map(|s| {
            let (_, reason) = waiting(&state.hub, &s.id)?;
            let info = state.hub.session_info(&s.id)?;
            (info.queue_count > 0).then(|| serde_json::json!({"id": s.id, "title": s.title, "reason": reason, "paused": s.paused}))
        }).collect();
        serde_json::json!({
            "can_configure": authenticated.principal.role.at_least(crate::admin::AdminRole::Operator),
            "policy": policy(&state.hub, &state.service_config, &machine.id),
            "sessions": rows.len(),
            "working": rows.iter().filter(|s| s.status == Status::Busy).count(),
            "warm": rows.iter().filter(|s| s.status == Status::Running && holds_worker_slot(&state.runtime_router, s)).count(),
            "sleeping": rows.iter().filter(|s| s.status == Status::Exited).count(),
            "starting": rows.iter().filter(|s| s.status == Status::Starting).count(),
            "updating": rows.iter().filter(|s| s.provider_update.is_some()).count(),
            "waiting": waiting,
            // The resource panel does not need Plugin or workspace inventories.
            "machine": {
                "id": machine.id, "display_name": machine.display_name,
                "local": machine.local, "connected": machine.connected,
                "capacity": machine.capacity, "active_sessions": machine.active_sessions,
                "resources": machine.resources,
            },
        })
    }).collect::<Vec<_>>()).into_response()
}

pub(super) async fn configure(
    State(state): State<Arc<AppState>>,
    Path(machine): Path<String>,
    Extension(authenticated): Extension<AuthenticatedProductRequest>,
    Json(value): Json<Policy>,
) -> Response {
    if !authenticated
        .principal
        .role
        .at_least(crate::admin::AdminRole::Operator)
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    if !state
        .session_maintenance
        .capacities
        .read()
        .contains_key(&machine)
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    let key = format!("{MACHINE_MAINTENANCE_PREFIX}{machine}");
    let (value, snapshot) = state.hub.with_settings_mut(|settings| {
        let mut saved: Policy = settings
            .get(&key)
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();
        if value.memory_reclaim.is_some() {
            saved.memory_reclaim = value.memory_reclaim;
        }
        if value.reclaim_on_capacity.is_some() {
            saved.reclaim_on_capacity = value.reclaim_on_capacity;
        }
        if value.auto_update_idle_sessions.is_some() {
            saved.auto_update_idle_sessions = value.auto_update_idle_sessions;
        }
        let value = serde_json::to_value(saved).expect("maintenance policy");
        let snapshot = Hub::commit_setting_locked(settings, key.clone(), value.clone());
        (value, snapshot)
    });
    state.hub.publish_setting(key, value, snapshot);
    Json(policy(&state.hub, &state.service_config, &machine)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_machine_overrides_preserve_defaults_and_other_machines() {
        let hub = Hub::new();
        let config = crate::config::Handle::new(crate::config::Config::defaults(
            crate::config::Scope::Service,
        ));
        let before = policy(&hub, &config, "ovh");
        hub.set_setting(
            format!("{MACHINE_MAINTENANCE_PREFIX}ovh"),
            serde_json::json!({"memory_reclaim": true}),
        );
        let after = policy(&hub, &config, "ovh");
        assert_eq!(after.memory_reclaim, Some(true));
        assert_eq!(after.reclaim_on_capacity, before.reclaim_on_capacity);
        assert_eq!(
            after.auto_update_idle_sessions,
            before.auto_update_idle_sessions
        );
        assert_eq!(
            policy(&hub, &config, "hawk").memory_reclaim,
            before.memory_reclaim
        );
    }

    #[test]
    fn resource_wait_reason_survives_restart_without_resetting_queue_order() {
        let hub = Hub::new();
        wait(&hub, "a", "memory");
        let since = waiting(&hub, "a").unwrap().0;
        wait(&hub, "a", "capacity");
        assert_eq!(waiting(&hub, "a"), Some((since, "capacity".into())));
        let restored = Hub::new();
        restored.load_settings(hub.settings_snapshot().into_iter().collect());
        assert_eq!(waiting(&restored, "a"), waiting(&hub, "a"));
        clear_wait(&restored, "a");
        assert_eq!(waiting(&restored, "a"), None);
    }
}
