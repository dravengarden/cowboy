//! Exact installed MCP resolution. A prepared executable is not a session
//! grant; the relay must admit a live binding before starting this process.
use std::path::PathBuf;

use anyhow::{Result, ensure};
use cowboy_plugin_sdk::{McpServerContract, McpServerSelection, PluginPayload};

use super::{MachinePluginStore, read_link_name, runtime_command};
use crate::mcp_binding::Binding;

#[derive(Debug)]
pub struct PreparedMcpServer {
    pub selection: McpServerSelection,
    pub contract: McpServerContract,
    pub executable: PathBuf,
}

/// Verified executable and a subset of its signed tools; still not a live grant.
#[derive(Debug)]
pub struct PreparedMcpBinding {
    pub name: String,
    pub server: PreparedMcpServer,
    pub tools: Vec<String>,
    pub sources: Vec<String>,
}

impl PreparedMcpBinding {
    pub fn native_descriptor(&self, epoch: &str) -> Result<crate::mcp_binding::NativeDescriptor> {
        let descriptor = crate::mcp_binding::NativeDescriptor {
            schema: 1,
            epoch: epoch.into(),
            binding: Binding {
                name: self.name.clone(),
                server: self.server.selection.clone(),
                tools: self.tools.clone(),
                sources: self.sources.clone(),
            },
            server_version: self.server.contract.version.clone(),
            tools: self.server.contract.discovery_for(&self.tools)?,
        };
        descriptor.validate()?;
        Ok(descriptor)
    }

    /// Signed generation metadata only; no process is started and no grant issued.
    pub fn discovery(&self) -> Result<serde_json::Value> {
        let tools = self.server.contract.discovery_for(&self.tools)?;
        Ok(
            serde_json::json!({"serverInfo":{"name":self.server.contract.id,"version":self.server.contract.version},"capabilities":{"tools":{}},"tools":tools}),
        )
    }
}

impl MachinePluginStore {
    pub fn prepare_mcp_binding(&self, binding: &Binding) -> Result<PreparedMcpBinding> {
        binding.validate()?;
        let server = self.prepare_mcp_server(&binding.server)?;
        ensure!(
            binding
                .tools
                .iter()
                .all(|tool| server.contract.tools.contains(tool)),
            "MCP binding requests an unsigned tool"
        );
        ensure!(
            binding
                .sources
                .iter()
                .all(|source| server.contract.sources.contains(source)),
            "MCP binding requests an unsigned source"
        );
        Ok(PreparedMcpBinding {
            name: binding.name.clone(),
            server,
            tools: binding.tools.clone(),
            sources: binding.sources.clone(),
        })
    }

    /// Reuse signed generation and private artifact verification. Never resolve
    /// commands through PATH or substitute the currently active generation.
    pub fn prepare_mcp_server(&self, selection: &McpServerSelection) -> Result<PreparedMcpServer> {
        selection.validate()?;
        self.ensure_mcp_selection_active(selection)?;
        let (package, _, content) =
            self.verified_plugin_generation(&selection.plugin_id, &selection.generation_digest)?;
        let PluginPayload::McpServer(contract) = package.payload else {
            anyhow::bail!("selected Plugin is not an MCP server");
        };
        let executable =
            runtime_command(&content.join("package.cowboy-plugin"), &contract.command)?;
        self.ensure_mcp_selection_active(selection)?;
        Ok(PreparedMcpServer {
            selection: selection.clone(),
            contract,
            executable,
        })
    }

    fn ensure_mcp_selection_active(&self, selection: &McpServerSelection) -> Result<()> {
        self.operations.ensure_unfenced(&selection.plugin_id)?;
        let active = read_link_name(&self.plugin_root(&selection.plugin_id).join("active"))
            .map(|generation| format!("sha256:{generation}"));
        ensure!(
            active.as_deref() == Some(selection.generation_digest.as_str()),
            "selected MCP generation is not active"
        );
        Ok(())
    }
}

#[cfg(target_os = "linux")]
mod process;
#[cfg(target_os = "linux")]
pub use process::McpProcess;

#[cfg(target_os = "linux")]
mod descriptors;
#[cfg(target_os = "linux")]
mod sessions;
#[cfg(target_os = "linux")]
pub use sessions::McpSessions;

#[cfg(test)]
pub(super) mod tests;
