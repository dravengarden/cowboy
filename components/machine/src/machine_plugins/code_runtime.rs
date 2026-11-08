//! Exact installed Code generations and Machine-owned agent admission.
use super::*;

impl MachinePluginStore {
    /// Attach Device policy before starting automatic Code lifecycle work.
    pub fn configure_code_maintenance(
        &self,
        config: crate::config::Handle,
    ) -> tokio::task::JoinHandle<()> {
        self.use_device_config(config);
        self.code_runtimes.start_agent_maintenance()
    }

    pub async fn navigate_code_buffer(
        &self,
        invocation: CodeNavigationInvocation,
    ) -> Result<crate::machine_protocol::code_buffer_navigation::Snapshot> {
        self.code_runtimes.navigate(invocation).await
    }

    pub async fn synchronize_code_buffer(
        &self,
        invocation: CodeBufferSyncInvocation,
    ) -> Result<crate::machine_protocol::code_buffer_sync::Snapshot> {
        self.code_runtimes.synchronize(invocation).await
    }

    /// Resolve the selected Plugin by capability id, never a global server
    /// executable. Existing routes keep their immutable generation on uninstall.
    pub async fn code_request(
        &self,
        plugin_id: &str,
        payload: &serde_json::Value,
        legacy_socket: Option<&Path>,
    ) -> Result<serde_json::Value> {
        validate_plugin_id(plugin_id)?;
        let root = self.plugin_root(plugin_id);
        let active = read_link_name(&root.join("active")).map(|g| format!("sha256:{g}"));
        let selected_active = active.clone();
        let select = || {
            self.operations.ensure_unfenced(plugin_id)?;
            if let Some(digest) = selected_active {
                let (package, _, content) = self.verified_plugin_generation(plugin_id, &digest)?;
                if let Some(plan) = self.code_launch_plan(&package, &digest, &content)? {
                    return Ok(CodeRuntimeSelection::Installed(plan));
                }
            }
            ensure!(
                !root.join(".code-runtime-owned-v2").exists(),
                "code-intelligence Plugin is not installed; legacy fallback is disabled"
            );
            Ok(CodeRuntimeSelection::Legacy(
                legacy_socket
                    .context("code-intelligence Plugin is not installed")?
                    .to_path_buf(),
            ))
        };
        if payload["type"] == "acquireAgentBuffer" {
            return self
                .code_runtimes
                .acquire_agent(
                    plugin_id,
                    payload,
                    active.clone(),
                    crate::machine_code_plugins::CodeAgentResourcePolicy::from_config(
                        self.device_config.get(),
                    ),
                    select,
                )
                .await;
        }
        self.code_runtimes
            .request_active(plugin_id, payload, active.clone(), select)
            .await
    }
}
