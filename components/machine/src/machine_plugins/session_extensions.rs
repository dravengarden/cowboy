//! Session-extension descriptors: the runtime Machine's installed, active
//! `memory_backend` and `call_profile` Plugins combined with the Controller's
//! per-session switches. Provider launchers read only this private file and,
//! for memory, the Machine-private binding it names; nothing here is secret.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, ensure};
use cowboy_plugin_sdk::{PluginKind, PluginPayload};
use serde_json::{Value, json};

use super::{MachinePluginStore, atomic_write};
use crate::machine_protocol::PluginInstallationState;
use crate::runtime_wire::{SESSION_EXTENSIONS_SCHEMA, SessionExtensions};

/// Provider launchers refuse larger descriptors.
const DESCRIPTOR_LIMIT: usize = 65_536;
/// Call profiles a descriptor carries; each guidance is at most 4 KiB.
const MAX_CALL_PROFILES: usize = 8;

/// Name of the variable that locates a session's extension descriptor.
pub const SESSION_EXTENSIONS_ENV: &str = "COWBOY_SESSION_EXTENSIONS";
/// Value of that variable when the descriptor could not be resolved: the
/// Provider then runs without memory instead of falling back to a host
/// enrollment that could override the session's switches.
pub const SESSION_EXTENSIONS_DISABLED: &str = "disabled";

fn valid_session_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

/// The private descriptor path of one session below the Machine state root.
pub fn descriptor_path(state_dir: &Path, session_id: &str) -> Option<PathBuf> {
    valid_session_id(session_id).then(|| {
        state_dir
            .join("extensions")
            .join("sessions")
            .join(format!("{session_id}.json"))
    })
}

impl MachinePluginStore {
    /// Resolve the descriptor for one session from this Machine's active
    /// session-extension Plugins. More than one active memory backend is
    /// ambiguous, so memory is then disabled rather than guessed.
    pub fn session_extensions(&self, switches: SessionExtensions) -> Result<Value> {
        ensure!(
            switches.schema == SESSION_EXTENSIONS_SCHEMA,
            "unsupported session extension schema"
        );
        let mut memory = Vec::new();
        let mut profiles = Vec::new();
        for installed in self.inventory()? {
            if installed.state != PluginInstallationState::Active
                || !matches!(
                    installed.plugin_kind,
                    PluginKind::MemoryBackend | PluginKind::CallProfile
                )
            {
                continue;
            }
            let (package, _, _) = self
                .verified_plugin_generation(&installed.plugin_id, &installed.generation_digest)
                .with_context(|| format!("verifying {} for a session", installed.plugin_id))?;
            let plugin = json!({
                "id": installed.plugin_id,
                "version": installed.plugin_version,
                "digest": installed.generation_digest,
            });
            match package.payload {
                PluginPayload::MemoryBackend(contract) => memory.push(json!({
                    "plugin": plugin,
                    "protocol": contract.protocol,
                    "tools": contract.tools,
                    "recall": contract.recall,
                    "observe": contract.observe,
                    "enable_tools": switches.memory_tools && !contract.tools.is_empty(),
                    "enable_recall": switches.memory_recall && contract.recall,
                })),
                PluginPayload::CallProfile(contract) => profiles.push(json!({
                    "plugin": plugin,
                    "purposes": contract.purposes,
                    "guidance": contract.guidance,
                })),
                _ => {}
            }
        }
        if memory.len() > 1 {
            tracing::warn!(
                backends = memory.len(),
                "more than one active memory backend; memory is disabled for the session"
            );
        }
        // Both switches off means no memory at all, including observation.
        let memory = (memory.len() == 1 && (switches.memory_tools || switches.memory_recall))
            .then(|| memory.remove(0));
        if profiles.len() > MAX_CALL_PROFILES {
            tracing::warn!(
                profiles = profiles.len(),
                kept = MAX_CALL_PROFILES,
                "too many active call profiles; the rest are omitted"
            );
            profiles.truncate(MAX_CALL_PROFILES);
        }
        Ok(json!({
            "schema": SESSION_EXTENSIONS_SCHEMA,
            "memory": memory,
            "calls": (!profiles.is_empty()).then(|| json!({"profiles": profiles})),
        }))
    }

    /// Atomically publish the session's descriptor (mode 0600) and return its
    /// path for the worker environment.
    pub fn write_session_extensions(
        &self,
        state_dir: &Path,
        session_id: &str,
        switches: SessionExtensions,
        relay: Option<Value>,
    ) -> Result<PathBuf> {
        let path = descriptor_path(state_dir, session_id).context("invalid session id")?;
        let mut descriptor = self.session_extensions(switches)?;
        if let (Some(memory), Some(relay)) = (
            descriptor.get_mut("memory").and_then(Value::as_object_mut),
            relay,
        ) {
            memory.insert("relay".to_owned(), relay);
        }
        let bytes = serde_json::to_vec(&descriptor)?;
        ensure!(
            bytes.len() <= DESCRIPTOR_LIMIT,
            "session extension descriptor exceeds its read limit"
        );
        let parent = path.parent().context("descriptor has no parent")?;
        std::fs::create_dir_all(parent)?;
        atomic_write(&path, &bytes, 0o600)?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt as _;

    use super::*;

    #[tokio::test]
    async fn descriptor_carries_active_extensions_and_session_switches() {
        let root = std::env::temp_dir().join(format!(
            "cowboy-session-extension-descriptor-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let publisher =
            crate::machine_auth::MachineIdentity::load_or_create(&root.join("publisher")).unwrap();
        let store = MachinePluginStore::new(
            &root.join("machine"),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap();
        let switches = SessionExtensions {
            schema: SESSION_EXTENSIONS_SCHEMA,
            memory_tools: true,
            memory_recall: false,
        };
        let empty = store.session_extensions(switches).unwrap();
        assert!(empty["memory"].is_null() && empty["calls"].is_null());

        let matrix = super::super::tests::session_extension_release(
            &publisher,
            include_str!("../../../../plugins/matrix/plugin.json"),
            include_str!("../../../../plugins/matrix/contract.json"),
        );
        let review = super::super::tests::session_extension_release(
            &publisher,
            include_str!("../../../../plugins/review/plugin.json"),
            include_str!("../../../../plugins/review/contract.json"),
        );
        let installed = store.install(&matrix).await.unwrap();
        store.install(&review).await.unwrap();
        let path = store
            .write_session_extensions(&root.join("state"), "sess-1", switches, None)
            .unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let descriptor: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let memory = &descriptor["memory"];
        assert_eq!(memory["plugin"]["id"], "matrix");
        assert_eq!(memory["plugin"]["digest"], installed.generation_digest);
        assert_eq!(memory["protocol"], "matrix_http_v1");
        assert_eq!(memory["tools"].as_array().unwrap().len(), 7);
        assert_eq!(memory["enable_tools"], true);
        assert_eq!(memory["enable_recall"], false);
        let profile = &descriptor["calls"]["profiles"][0];
        assert_eq!(profile["plugin"]["id"], "review");
        assert_eq!(profile["purposes"][1]["id"], "design_review");
        assert!(
            profile["guidance"]
                .as_str()
                .unwrap()
                .contains("{{peer_command}}")
        );
        assert!(descriptor_path(&root, "../escape").is_none());
        let off = SessionExtensions {
            memory_tools: false,
            memory_recall: false,
            ..switches
        };
        assert!(store.session_extensions(off).unwrap()["memory"].is_null());
        assert!(!store.session_extensions(off).unwrap()["calls"].is_null());
        store
            .uninstall("matrix", &installed.generation_digest)
            .await
            .unwrap();
        assert!(store.session_extensions(switches).unwrap()["memory"].is_null());
        std::fs::remove_dir_all(root).unwrap();
    }
}
