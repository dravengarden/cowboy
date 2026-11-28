//! Persisted config-preference replay, config-startup fencing, and recovery
//! fence release on replacement readiness for [`super::RemoteRuntime`].

use super::{
    CoreCommand, Ordering, Shared, WorkerSnapshot, config_option_accepts, notice_recovery,
};

pub(super) fn queue_persisted_config_for_session(
    shared: &Shared,
    session_id: &str,
    force_replay: bool,
) {
    let persisted_options = shared.hub.persisted_config_options(session_id);
    let worker_options = {
        let workers = shared.workers.lock();
        workers
            .get(session_id)
            .and_then(|worker| worker.config_options.clone())
    };
    // The Hub snapshot is the last controller-authoritative state and may be
    // newer than a detached worker's launch-time snapshot. Prefer it when
    // deciding whether a durable value is still safe to replay.
    let options = persisted_options.or(worker_options);
    queue_persisted_config(shared, session_id, options.as_ref(), force_replay);
}

pub(super) fn queue_persisted_config(
    shared: &Shared,
    session_id: &str,
    options: Option<&serde_json::Value>,
    force_replay: bool,
) {
    // Managed workers advertise only the signed preset options. Wait for that
    // allowlist before replaying: older workers hide every option, and legacy
    // interactive permission preferences must never reach a managed child.
    if shared.hub.is_managed_child(session_id) && options.is_none() {
        return;
    }
    let Some(preferences) = shared.hub.config_preferences(session_id) else {
        return;
    };
    let Some(preferences) = preferences.as_object() else {
        return;
    };
    let Some(meta) = shared
        .hub
        .session_list()
        .into_iter()
        .find(|meta| meta.id == session_id)
    else {
        return;
    };
    let configuration = meta.provider_behavior.as_ref().map_or_else(
        || crate::provider::legacy_behavior(&meta.provider).configuration,
        |behavior| behavior.configuration.clone(),
    );
    let Some(options) = options else {
        for (config_id, value) in preferences {
            if crate::managed_config::is_managed(&configuration, config_id)
                || crate::managed_config::is_internal_preference_key(config_id)
            {
                continue;
            }
            queue_config_value(shared, session_id, config_id, value.clone());
        }
        return;
    };
    let Some(options) = options.as_array() else {
        return;
    };
    for (config_id, value) in preferences {
        if crate::managed_config::is_managed(&configuration, config_id)
            || crate::managed_config::is_internal_preference_key(config_id)
        {
            continue;
        }
        let Some(option) = options
            .iter()
            .find(|option| option.get("id").and_then(serde_json::Value::as_str) == Some(config_id))
        else {
            continue;
        };
        if (!force_replay && config_current_value(option) == Some(value))
            || !config_option_accepts(option, value)
        {
            continue;
        }
        queue_config_value(shared, session_id, config_id, value.clone());
    }
}

pub(super) fn sync_config_for_worker(shared: &Shared, worker: &WorkerSnapshot) {
    if !worker.has_connected_owner() || worker.config_options.is_none() {
        return;
    }
    let already_synced = {
        let mut epochs = shared.config_sync_epochs.lock();
        if epochs
            .get(&worker.session_id)
            .is_some_and(|epoch| epoch == &worker.worker_epoch)
        {
            true
        } else {
            epochs.insert(worker.session_id.clone(), worker.worker_epoch.clone());
            false
        }
    };
    if !already_synced {
        queue_persisted_config(
            shared,
            &worker.session_id,
            worker.config_options.as_ref(),
            false,
        );
    }
    let released_startup = shared.config_startups.lock().remove(&worker.session_id);
    let released_recovery = {
        let mut fences = shared.recovery_fences.lock();
        if fences.get(&worker.session_id).is_some_and(|fence| {
            fence.state == notice_recovery::RecoveryFenceState::AwaitingReplacement
                && worker.worker_epoch != fence.pre_recovery_epoch
        }) {
            fences.remove(&worker.session_id);
            true
        } else {
            false
        }
    };
    if released_startup || released_recovery {
        let _ = shared.notify.send(());
    }
}

/// Uncertain target effects no longer park session queues or require owner
/// acknowledgement. Send `u64::MAX` on prompt commands so already-deployed
/// Machine brokers (which normalize `None` to `Some(0)` on v8+ Controllers) and
/// workers never refuse prompts on an uncertain effect.
pub(super) const UNCERTAIN_EFFECT_BYPASS_SEQUENCE: u64 = u64::MAX;

// Completion proves admission when the separate CommandAck was lost. Start
// events and active snapshots precede the fallible ACP send and cannot prove it.
pub(super) fn fail_config_startup(shared: &Shared, session_id: &str, reason: &str) {
    let rejected = {
        let mut startups = shared.config_startups.lock();
        let recovery_fences = shared.recovery_fences.lock();
        if !startups.remove(session_id) {
            return;
        }
        // Finding 3: a negative ACK or crash from an earlier `EnsureSession`
        // must never reopen or sweep prompts guarded by an active execution
        // recovery fence.
        if recovery_fences.contains_key(session_id) {
            return;
        }
        take_pending_prompts(shared, session_id)
    };
    for command in rejected {
        notice_recovery::requeue_rejected_command(
            shared,
            session_id,
            Some(command),
            Some(reason.to_owned()),
        );
    }
}

pub(super) fn take_pending_prompts(shared: &Shared, session_id: &str) -> Vec<CoreCommand> {
    let mut rejected = Vec::new();
    shared.pending.lock().retain(|key, command| {
        if matches!(command, CoreCommand::Prompt { session_id: owner, .. } if owner == session_id) {
            rejected.push((key.clone(), command.clone()));
            false
        } else {
            true
        }
    });
    rejected
        .into_iter()
        .map(|(key, command)| {
            shared.sent.lock().remove(&key);
            command
        })
        .collect()
}

pub(super) fn queue_config_value(
    shared: &Shared,
    session_id: &str,
    config_id: &str,
    value: serde_json::Value,
) {
    let same_pending = shared.pending.lock().iter().any(|(_, command)| {
        matches!(
            command,
            CoreCommand::SetConfigOption {
                session_id: pending_session,
                config_id: pending_id,
                value: pending_value,
                ..
            } if pending_session == session_id
                && pending_id == config_id
                && pending_value == &value
        )
    });
    if same_pending {
        return;
    }
    let stale_keys: Vec<String> = shared
        .pending
        .lock()
        .iter()
        .filter_map(|(key, command)| {
            matches!(
                command,
                CoreCommand::SetConfigOption {
                    session_id: pending_session,
                    config_id: pending_id,
                    ..
                } if pending_session == session_id && pending_id == config_id
            )
            .then_some(key.clone())
        })
        .collect();
    for key in stale_keys {
        shared.pending.lock().remove(&key);
        shared.sent.lock().remove(&key);
    }
    let counter = shared.command_counter.fetch_add(1, Ordering::Relaxed);
    let command_id = format!("config-{}-{counter}", std::process::id());
    super::insert_pending(
        shared,
        command_id.clone(),
        CoreCommand::SetConfigOption {
            session_id: session_id.to_owned(),
            command_id,
            config_id: config_id.to_owned(),
            value,
        },
    );
    let _ = shared.notify.send(());
}

pub(super) fn discard_pending_config(shared: &Shared, session_id: &str) {
    let mut removed = Vec::new();
    shared.pending.lock().retain(|key, command| {
        let obsolete = matches!(command, CoreCommand::SetConfigOption { session_id: owner, .. }
            if owner == session_id);
        if obsolete {
            removed.push(key.clone());
        }
        !obsolete
    });
    let mut sent = shared.sent.lock();
    for key in removed {
        sent.remove(&key);
    }
}

fn config_current_value(option: &serde_json::Value) -> Option<&serde_json::Value> {
    option
        .get("currentValue")
        .or_else(|| option.get("current_value"))
}

pub(super) enum PromptPrep {
    Ready,
    Skip,
    ParkAmbiguous {
        session_id: String,
        command: Box<CoreCommand>,
        replacement_worker: Option<Box<WorkerSnapshot>>,
    },
}

pub(super) fn prepare_or_park_prompt_for_send(
    shared: &std::sync::Arc<Shared>,
    key: &str,
    version: u64,
    command: &mut CoreCommand,
) -> bool {
    match prepare_prompt_for_send(shared, key, version, command) {
        PromptPrep::Ready => true,
        PromptPrep::Skip => false,
        PromptPrep::ParkAmbiguous {
            session_id,
            command,
            replacement_worker,
        } => {
            notice_recovery::park_and_retire_ambiguous_pending(
                shared,
                &session_id,
                key,
                version,
                *command,
                "prompt delivery across worker change is unresolved; retained as draft",
                replacement_worker.as_deref(),
            );
            false
        }
    }
}

/// Stamp `command`'s (when `CoreCommand::Prompt`) worker epoch and legacy
/// uncertain-effect bypass sequence before transport handoff, parking any
/// already-attempted send across a worker epoch change.
#[allow(clippy::too_many_lines)]
pub(super) fn prepare_prompt_for_send(
    shared: &Shared,
    key: &str,
    version: u64,
    command: &mut CoreCommand,
) -> PromptPrep {
    let CoreCommand::Prompt {
        session_id,
        expected_worker_epoch,
        acknowledged_uncertain_sequence,
        ..
    } = command
    else {
        return PromptPrep::Ready;
    };
    let worker_snapshot = shared.workers.lock().get(session_id).cloned();
    let already_attempted = shared.attempted_sends.lock().contains(key);
    if already_attempted {
        // Never rebind or blindly replay an already-attempted prompt across a
        // worker change or without a bound worker epoch. Only a live worker on
        // the exact bound epoch can safely deduplicate a retry by `command_id`.
        let current_epoch = worker_snapshot
            .as_ref()
            .map(|worker| worker.worker_epoch.as_str());
        // Finding 13: Only a connected, non-terminal replacement worker (never
        // a broker placeholder) authorizes releasing the idle guard.
        let replacement_worker = expected_worker_epoch.as_deref().and_then(|bound| {
            worker_snapshot
                .as_ref()
                .filter(|w| {
                    w.worker_epoch != bound
                        && w.has_connected_owner()
                        && !matches!(
                            w.state,
                            super::WorkerState::Exited | super::WorkerState::Crashed
                        )
                })
                .cloned()
        });
        let ambiguous = match (expected_worker_epoch.as_deref(), current_epoch) {
            (Some(bound), Some(current)) => current != bound,
            (None, _) => true,
            (Some(_), None) => false,
        };
        if ambiguous {
            let snapshot = {
                let pending = shared.pending.lock();
                (pending.version(key) == Some(version))
                    .then(|| pending.get(key).cloned())
                    .flatten()
            };
            return match snapshot {
                Some(cmd) => PromptPrep::ParkAmbiguous {
                    session_id: session_id.clone(),
                    command: Box::new(cmd),
                    replacement_worker: replacement_worker.map(Box::new),
                },
                None => PromptPrep::Skip,
            };
        }
        if current_epoch.is_none() {
            return PromptPrep::Skip;
        }
        return PromptPrep::Ready;
    }
    let mut mutated = false;
    if acknowledged_uncertain_sequence.is_none() {
        *acknowledged_uncertain_sequence = Some(UNCERTAIN_EFFECT_BYPASS_SEQUENCE);
        mutated = true;
    }
    if expected_worker_epoch.is_none() {
        let bound = worker_snapshot
            .filter(|worker| {
                !worker.drain_requested
                    && matches!(
                        worker.state,
                        super::WorkerState::Running | super::WorkerState::Busy
                    )
            })
            .map(|worker| worker.worker_epoch);
        if bound.is_some() {
            *expected_worker_epoch = bound;
            mutated = true;
        }
    }
    if mutated
        && !shared
            .pending
            .lock()
            .update_if_version(key, version, command.clone())
    {
        return PromptPrep::Skip;
    }
    PromptPrep::Ready
}
