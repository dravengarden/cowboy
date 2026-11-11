//! Directory MCP defaults. Creation pins the Plugin generation active on the
//! session's execution Machine; the policy is committed and prepared once the
//! execution binding is final and before the first worker launch. A failure
//! never blocks the session: it runs without the binding and keeps the reason.
use super::*;
use crate::core::settings_keys::SESSION_MCP_DEFAULT_PREFIX;
use crate::execution_environment::ExecutionBinding;
use crate::machine_protocol::{PluginInstallationState, PluginInventory};
use crate::session_folders::defaults::{DefaultKey, McpBindingDefault, mcp_bindings};

const PREPARE_ATTEMPTS: usize = 5;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum Outcome {
    Pending {
        bindings: Vec<Binding>,
    },
    Applied {
        epoch: String,
        bindings: Vec<Binding>,
    },
    Skipped {
        reason: String,
    },
}

fn key(session: &str) -> String {
    format!("{SESSION_MCP_DEFAULT_PREFIX}{session}")
}

fn load(hub: &Hub, session: &str) -> Option<Outcome> {
    serde_json::from_value(hub.setting(&key(session))?).ok()
}

fn record(hub: &Hub, session: &str, outcome: &Outcome) {
    if let Outcome::Skipped { reason } = outcome {
        tracing::warn!(session = %session, %reason, "directory MCP default not applied");
    }
    hub.set_setting(key(session), json!(outcome));
}

/// The recorded outcome, projected next to the session's MCP policy.
pub(super) fn observation(hub: &Hub, session: &str) -> serde_json::Value {
    hub.setting(&key(session))
        .unwrap_or(serde_json::Value::Null)
}

/// Resolve the creating directory's default for a just-registered session.
pub(in crate::server) async fn resolve_at_creation(
    state: &AppState,
    principal: &crate::product_auth::ProductPrincipal,
    session: &str,
    folder: Option<&str>,
) {
    let Ok(defaults) = super::super::folder_defaults::resolve(state, principal, folder) else {
        return;
    };
    let Some(Some(value)) = defaults.values.get(&DefaultKey::McpBindings) else {
        return;
    };
    let outcome = match pin(state, session, value).await {
        Ok(bindings) => Outcome::Pending { bindings },
        Err(reason) => Outcome::Skipped { reason },
    };
    record(&state.hub, session, &outcome);
}

async fn pin(state: &AppState, session: &str, value: &str) -> Result<Vec<Binding>, String> {
    let defaults = mcp_bindings(value)?;
    let machine = execution_machine(&state.hub, session)?;
    let store = state.store.as_ref().ok_or("Machine registry unavailable")?;
    let record = store
        .list_machines()
        .await
        .map_err(|_| "Machine registry unavailable")?
        .into_iter()
        .find(|record| record.id == machine && !record.revoked)
        .ok_or_else(|| format!("execution Machine {machine} is not enrolled"))?;
    let bindings = resolve(
        &defaults,
        &super::super::execution::sessions::providers(&record),
        &machine,
    )?;
    crate::mcp_binding::validate_bindings(&bindings)
        .map_err(|error| format!("invalid MCP bindings: {error}"))?;
    Ok(bindings)
}

/// Only a session preparing a separate execution environment can carry MCP.
fn execution_machine(hub: &Hub, session: &str) -> Result<String, String> {
    let meta = hub.session_info(session).ok_or("session removed")?.meta;
    if meta.system || hub.is_managed_child(session) {
        return Err("managed and system sessions cannot use MCP bindings".into());
    }
    meta.execution_binding
        .as_ref()
        .and_then(ExecutionBinding::preparation)
        .map(|preparation| preparation.machine_id)
        .ok_or_else(|| {
            "MCP bindings need a session whose runtime Machine differs from its project Machine"
                .into()
        })
}

/// Each default selects the generation currently active on the Machine.
fn resolve(
    defaults: &[McpBindingDefault],
    installed: &[PluginInventory],
    machine: &str,
) -> Result<Vec<Binding>, String> {
    defaults
        .iter()
        .map(|default| {
            installed
                .iter()
                .find(|plugin| {
                    plugin.plugin_id == default.plugin_id
                        && plugin.plugin_kind == cowboy_plugin_sdk::PluginKind::McpServer
                        && plugin.state == PluginInstallationState::Active
                })
                .map(|plugin| default.binding(plugin.generation_digest.clone()))
                .ok_or_else(|| {
                    format!(
                        "MCP Plugin {} is not installed on Machine {machine}",
                        default.plugin_id
                    )
                })
        })
        .collect()
}

/// Commit a pending default before the session's first launch. Idempotent and
/// serialized; a session still preparing its execution binding stays pending.
pub(in crate::server) async fn apply(state: &AppState, session: &str) {
    if !matches!(load(&state.hub, session), Some(Outcome::Pending { .. })) {
        return;
    }
    let _serial = state.extensions.bindings.defaults.lock().await;
    let Some(Outcome::Pending { bindings }) = load(&state.hub, session) else {
        return;
    };
    let Some(meta) = state.hub.session_info(session).map(|info| info.meta) else {
        return;
    };
    if meta
        .execution_binding
        .as_ref()
        .is_none_or(|binding| binding.preparation().is_some())
    {
        return;
    }
    let outcome = match commit(state, &meta, bindings).await {
        Ok(stored) => Outcome::Applied {
            epoch: stored.epoch,
            bindings: stored.bindings,
        },
        Err(reason) => Outcome::Skipped { reason },
    };
    record(&state.hub, session, &outcome);
}

async fn commit(
    state: &AppState,
    meta: &SessionMeta,
    bindings: Vec<Binding>,
) -> Result<Stored, String> {
    if state.supervisor.has_live_worker(&meta.id) {
        return Err("the agent started before the directory MCP default was applied".into());
    }
    let store = state.store.as_ref().ok_or("storage unavailable")?;
    let policies = &state.extensions.bindings;
    let stored = policies
        .replace(
            &state.hub,
            store,
            meta,
            // View grants are per session and never inherited from a
            // directory default (docs/widget-apps-mcp.md §11).
            Update {
                expected_epoch: None,
                bindings,
                apps: std::collections::BTreeMap::new(),
            },
        )
        .await
        .map_err(|code| format!("MCP policy refused: {code}"))?;
    let mut last = String::new();
    for attempt in 0..PREPARE_ATTEMPTS {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        match state
            .extensions
            .launches
            .prepare(state, &meta.id, &stored.epoch)
            .await
        {
            Ok(()) => return Ok(stored),
            Err(error) => last = format!("{error:#}"),
        }
    }
    // A policy that cannot be prepared would refuse every launch.
    let cleared = policies
        .replace(
            &state.hub,
            store,
            meta,
            Update {
                expected_epoch: Some(stored.epoch),
                bindings: Vec::new(),
                apps: std::collections::BTreeMap::new(),
            },
        )
        .await;
    Err(match cleared {
        Ok(_) => format!("MCP launch preparation failed: {last}"),
        Err(code) => {
            format!("MCP launch preparation failed: {last}; clearing the policy failed: {code}")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed(plugin_id: &str, kind: &str, state: &str) -> PluginInventory {
        serde_json::from_value(json!({
            "plugin_id": plugin_id, "plugin_version": "0.3.0", "plugin_kind": kind,
            "generation_digest": format!("sha256:{}", "b".repeat(64)),
            "contract_fingerprint": format!("sha256:{}", "c".repeat(64)), "state": state,
        }))
        .unwrap()
    }

    fn suger() -> Vec<McpBindingDefault> {
        mcp_bindings(r#"[{"name":"suger","plugin_id":"suger","tools":["suger_status","suger_read"],"sources":["github","local_env"]}]"#).unwrap()
    }

    #[test]
    fn resolution_pins_the_active_generation_of_the_named_mcp_plugin() {
        let bindings = resolve(
            &suger(),
            &[
                installed("other", "mcp_server", "active"),
                installed("suger", "mcp_server", "active"),
            ],
            "hawk",
        )
        .unwrap();
        assert_eq!(
            json!(bindings),
            json!([{"name":"suger","server":{"plugin_id":"suger","generation_digest":format!("sha256:{}", "b".repeat(64))},
                "tools":["suger_status","suger_read"],"sources":["github","local_env"]}])
        );
    }

    #[test]
    fn missing_inactive_or_wrong_kind_plugins_are_not_installed() {
        for inventory in [
            vec![],
            vec![installed("suger", "mcp_server", "installing")],
            vec![installed("suger", "agent_provider", "active")],
        ] {
            assert_eq!(
                resolve(&suger(), &inventory, "hawk").unwrap_err(),
                "MCP Plugin suger is not installed on Machine hawk"
            );
        }
    }

    #[test]
    fn system_and_single_machine_sessions_are_skipped_with_a_reason() {
        let hub = Hub::new();
        for (id, system) in [("plain", false), ("system", true)] {
            hub.create_local_session(
                id.into(),
                "codex".into(),
                "/tmp".into(),
                "t".into(),
                crate::core::SessionOrigin::Web,
                system,
            );
        }
        assert!(
            execution_machine(&hub, "plain")
                .unwrap_err()
                .contains("runtime Machine differs")
        );
        assert!(
            execution_machine(&hub, "system")
                .unwrap_err()
                .contains("system sessions")
        );
        assert!(execution_machine(&hub, "missing").is_err());
    }

    #[test]
    fn outcomes_round_trip_as_tagged_settings() {
        let hub = Hub::new();
        record(
            &hub,
            "s",
            &Outcome::Skipped {
                reason: "not installed".into(),
            },
        );
        assert_eq!(
            observation(&hub, "s"),
            json!({"state":"skipped","reason":"not installed"})
        );
        assert!(matches!(load(&hub, "s"), Some(Outcome::Skipped { .. })));
        assert_eq!(observation(&hub, "none"), serde_json::Value::Null);
    }
}
