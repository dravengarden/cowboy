//! A Plugin release moves the generation a session's policy pinned. The Machine
//! prepares only its active generation, so once the in-memory launch owner is
//! lost (a Controller restart) the old pin can never be prepared again and every
//! send fails. Launch recovery therefore advances such a policy to the
//! generation now active on the execution Machine. Name, tools, sources and
//! view grants remain the owner's selection; the Machine still verifies them
//! against the new signed contract before it prepares anything.
use super::*;
use crate::machine_protocol::{PluginInstallationState, PluginInventory};

/// The bindings with every superseded pin replaced, or `None` when none moved.
/// A Plugin that is not active on the Machine keeps its pin: there is nothing
/// to advance to, and the failure stays visible.
fn advanced(bindings: &[Binding], installed: &[PluginInventory]) -> Option<Vec<Binding>> {
    let mut changed = false;
    let next = bindings
        .iter()
        .map(|binding| {
            let mut binding = binding.clone();
            let active = installed.iter().find(|plugin| {
                plugin.plugin_id == binding.server.plugin_id
                    && plugin.plugin_kind == cowboy_plugin_sdk::PluginKind::McpServer
                    && plugin.state == PluginInstallationState::Active
            });
            if let Some(active) = active
                && active.generation_digest != binding.server.generation_digest
            {
                binding.server.generation_digest = active.generation_digest.clone();
                changed = true;
            }
            binding
        })
        .collect();
    changed.then_some(next)
}

/// Commit the advanced policy and return its epoch; `None` when the policy
/// already pins the active generations.
pub async fn advance(state: &AppState, session: &str) -> Result<Option<String>, String> {
    let Some(stored) = Stored::load(&state.hub, session) else {
        return Ok(None);
    };
    if stored.bindings.is_empty() {
        return Ok(None);
    }
    let store = state.store.as_ref().ok_or("storage unavailable")?;
    let machine = &stored.placement.execution_machine;
    let record = store
        .list_machines()
        .await
        .map_err(|_| "Machine registry unavailable")?
        .into_iter()
        .find(|record| record.id == *machine && !record.revoked)
        .ok_or_else(|| format!("execution Machine {machine} is not enrolled"))?;
    let installed = super::super::execution::sessions::providers(&record);
    let Some(bindings) = advanced(&stored.bindings, &installed) else {
        return Ok(None);
    };
    let meta = state
        .hub
        .session_info(session)
        .ok_or("session removed")?
        .meta;
    let next = state
        .extensions
        .bindings
        .replace(
            &state.hub,
            store,
            &meta,
            Update {
                expected_epoch: Some(stored.epoch.clone()),
                bindings,
                apps: stored.apps.clone(),
            },
        )
        .await
        .map_err(|code| format!("MCP policy refused: {code}"))?;
    defaults::follow(&state.hub, session, &stored.epoch, &next);
    tracing::info!(
        session = %session,
        previous_epoch = %stored.epoch,
        epoch = %next.epoch,
        "MCP policy advanced to the active Plugin generation"
    );
    Ok(Some(next.epoch))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installed(plugin_id: &str, kind: &str, state: &str, digest: char) -> PluginInventory {
        serde_json::from_value(json!({
            "plugin_id": plugin_id, "plugin_version": "0.3.0", "plugin_kind": kind,
            "generation_digest": format!("sha256:{}", digest.to_string().repeat(64)),
            "contract_fingerprint": format!("sha256:{}", "c".repeat(64)), "state": state,
        }))
        .unwrap()
    }

    fn binding(plugin_id: &str, digest: char) -> Binding {
        serde_json::from_value(json!({
            "name": plugin_id,
            "server": {"plugin_id": plugin_id, "generation_digest": format!("sha256:{}", digest.to_string().repeat(64))},
            "tools": ["suger_status"], "sources": ["github"],
        }))
        .unwrap()
    }

    #[test]
    fn a_superseded_pin_follows_the_active_generation_and_keeps_the_selection() {
        let bindings = vec![binding("suger", 'a'), binding("other", 'd')];
        let next = advanced(
            &bindings,
            &[
                installed("suger", "mcp_server", "active", 'b'),
                installed("other", "mcp_server", "active", 'd'),
            ],
        )
        .expect("the superseded pin advances");
        assert_eq!(next[0], binding("suger", 'b'));
        assert_eq!(next[1], bindings[1]);
    }

    #[test]
    fn current_missing_or_inactive_plugins_keep_their_pin() {
        let bindings = vec![binding("suger", 'a')];
        assert_eq!(
            advanced(
                &bindings,
                &[installed("suger", "mcp_server", "active", 'a')]
            ),
            None
        );
        assert_eq!(advanced(&bindings, &[]), None);
        assert_eq!(
            advanced(
                &bindings,
                &[installed("suger", "agent_provider", "active", 'b')]
            ),
            None
        );
        assert_eq!(
            advanced(
                &bindings,
                &[installed("suger", "mcp_server", "failed", 'b')]
            ),
            None
        );
    }
}
