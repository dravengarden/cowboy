//! Sparse per-field defaults. Missing means inherit; null stops inheritance.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefaultKey {
    Project,
    Machine,
    RuntimeMachine,
    Provider,
    /// Provider-qualified preset identity: `provider/preset`.
    Preset,
    ReviewDirectory,
    /// `on` or `off`: whether Review may run the worktree's configured
    /// language servers. Unset inherits; the top level means on.
    LanguageServers,
    /// JSON list of [`McpBindingDefault`] granted to each new session.
    McpBindings,
}

pub type FolderDefaults = BTreeMap<DefaultKey, Option<String>>;

/// A directory MCP default names a Plugin, never a generation: each new session
/// pins the generation active on its execution Machine when it is created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpBindingDefault {
    pub name: String,
    pub plugin_id: String,
    pub tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
}

impl McpBindingDefault {
    #[must_use]
    pub fn binding(&self, generation_digest: String) -> crate::mcp_binding::Binding {
        crate::mcp_binding::Binding {
            name: self.name.clone(),
            server: cowboy_plugin_sdk::McpServerSelection {
                plugin_id: self.plugin_id.clone(),
                generation_digest,
            },
            tools: self.tools.clone(),
            sources: self.sources.clone(),
        }
    }
}

/// Parse with the session binding rules; only the generation is left open.
pub fn mcp_bindings(value: &str) -> Result<Vec<McpBindingDefault>, String> {
    let defaults: Vec<McpBindingDefault> = serde_json::from_str(value)
        .map_err(|error| format!("invalid directory MCP bindings: {error}"))?;
    if defaults.is_empty() {
        return Err("directory MCP bindings must not be empty; clear the field instead".into());
    }
    let placeholder = format!("sha256:{}", "0".repeat(64));
    let bindings: Vec<_> = defaults
        .iter()
        .map(|binding| binding.binding(placeholder.clone()))
        .collect();
    crate::mcp_binding::validate_bindings(&bindings)
        .map_err(|error| format!("invalid directory MCP bindings: {error}"))?;
    Ok(defaults)
}

/// Stored preferences may come from a newer Controller. Validate known fields
/// strictly, but leave future keys opaque to this reader. Input APIs stay closed.
pub fn parse_stored(value: &serde_json::Value) -> Result<FolderDefaults, String> {
    let mut fields = value
        .as_object()
        .cloned()
        .ok_or("invalid stored directory defaults")?;
    fields.retain(|key, _| serde_json::from_value::<DefaultKey>(serde_json::json!(key)).is_ok());
    parse(&serde_json::Value::Object(fields))
}

/// Updating known fields (including restoring inheritance) must not erase
/// settings introduced by a newer writer during a rollback.
pub fn stored_json(
    defaults: &FolderDefaults,
    previous: Option<&str>,
) -> Result<String, serde_json::Error> {
    let mut value: serde_json::Map<String, serde_json::Value> = previous
        .map(serde_json::from_str)
        .transpose()?
        .unwrap_or_default();
    value.retain(|key, _| serde_json::from_value::<DefaultKey>(serde_json::json!(key)).is_err());
    let known = serde_json::to_value(defaults)?;
    value.extend(
        known
            .as_object()
            .expect("defaults serialize as an object")
            .clone(),
    );
    serde_json::to_string(&value)
}

/// Validate the complete sparse draft before creating any directory.
pub(super) fn parse_create(args: &serde_json::Value) -> Result<FolderDefaults, String> {
    args.get("defaults")
        .map_or_else(|| Ok(FolderDefaults::default()), parse)
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedDefaults {
    pub values: FolderDefaults,
    pub sources: BTreeMap<DefaultKey, String>,
}

pub fn parse(value: &serde_json::Value) -> Result<FolderDefaults, String> {
    let defaults: FolderDefaults = serde_json::from_value(value.clone())
        .map_err(|error| format!("invalid directory defaults: {error}"))?;
    for (key, value) in &defaults {
        if let Some(value) = value {
            if *key == DefaultKey::ReviewDirectory {
                if !crate::review_directory::valid(value) {
                    return Err("invalid Review directory identity".into());
                }
                continue;
            }
            if *key == DefaultKey::McpBindings {
                mcp_bindings(value)?;
                continue;
            }
            if *key == DefaultKey::LanguageServers {
                if !matches!(value.as_str(), "on" | "off") {
                    return Err("project language servers must be on or off".into());
                }
                continue;
            }
            if matches!(key, DefaultKey::Machine | DefaultKey::RuntimeMachine) && value == "local" {
                return Err("directory defaults require an enrolled Machine id, not the legacy local sentinel".into());
            }
            if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
                return Err(format!("invalid directory default {key:?}"));
            }
            if *key == DefaultKey::Preset
                && !value
                    .split_once('/')
                    .is_some_and(|(provider, preset)| !provider.is_empty() && !preset.is_empty())
            {
                return Err("preset must be provider/preset".into());
            }
        }
    }
    Ok(defaults)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session_folders::{FolderActor, SessionFolders};
    use serde_json::json;

    #[test]
    fn stored_future_defaults_are_opaque_and_survive_older_writes() {
        let saved = json!({"project":"cowboy","future_directory":"workspace::hawk::cowboy","future":{"nested":true}});
        assert!(
            parse(&saved).is_err(),
            "new writes must still reject unknown keys"
        );
        let mut older = parse_stored(&saved).unwrap();
        assert_eq!(older.len(), 1);
        older.remove(&DefaultKey::Project);
        older.insert(DefaultKey::Provider, Some("codex".into()));
        let written: serde_json::Value =
            serde_json::from_str(&stored_json(&older, Some(&saved.to_string())).unwrap()).unwrap();
        assert_eq!(
            written,
            json!({"provider":"codex","future_directory":"workspace::hawk::cowboy","future":{"nested":true}})
        );
        assert!(parse_stored(&json!({"project":42,"future":"x"})).is_err());
    }

    fn tree() -> SessionFolders {
        let mut tree = SessionFolders::default();
        tree.apply(
            &FolderActor::local(),
            "create",
            &json!({"id":"root","name":"Root","project":"suger"}),
        )
        .unwrap();
        tree.apply(
            &FolderActor::local(),
            "create",
            &json!({"id":"child","name":"Child","parent":"root"}),
        )
        .unwrap();
        tree.apply(&FolderActor::local(), "defaults", &json!({"id":"root","values":{"machine":"hawk","provider":"claude-code","preset":"claude-code/opus-default"}})).unwrap();
        tree
    }

    #[test]
    fn create_saves_sparse_defaults_atomically_and_replays_exactly() {
        let mut tree = tree();
        let actor = FolderActor::local();
        let args = json!({"id":"new","name":"New","parent":"root","defaults":{"project":null,"provider":"codex","preset":"codex/review"}});
        tree.apply(&actor, "create", &args).unwrap();
        let resolved = tree.resolve_defaults(&actor, "new").unwrap();
        assert_eq!(resolved.values[&DefaultKey::Project], None);
        assert_eq!(
            resolved.values[&DefaultKey::Machine].as_deref(),
            Some("hawk")
        );
        assert_eq!(resolved.sources[&DefaultKey::Machine], "root");
        assert_eq!(resolved.sources[&DefaultKey::Provider], "new");
        assert_eq!(
            tree.folders()
                .iter()
                .find(|f| f.id == "new")
                .unwrap()
                .defaults
                .len(),
            3
        );
        assert!(tree.is_replayed_create(&actor, &args));
        let mut changed = args;
        changed["defaults"]["preset"] = json!("codex/other");
        assert!(!tree.is_replayed_create(&actor, &changed));
        changed.as_object_mut().unwrap().remove("defaults");
        assert!(!tree.is_replayed_create(&actor, &changed));
    }

    #[test]
    fn invalid_creation_defaults_leave_no_partial_directory() {
        let mut tree = tree();
        let actor = FolderActor::local();
        let before = tree.value();
        for invalid in [
            json!(null),
            json!([]),
            json!({"unknown":"x"}),
            json!({"machine":"local"}),
            json!({"preset":"opus"}),
            json!({"project":42}),
        ] {
            assert!(
                tree.apply(
                    &actor,
                    "create",
                    &json!({"id":"new","name":"New","parent":"root","defaults":invalid})
                )
                .is_err()
            );
            assert_eq!(tree.value(), before);
        }
        let foreign = FolderActor {
            user_id: Some("other".into()),
            sees_all: false,
        };
        assert!(
            tree.apply(
                &foreign,
                "create",
                &json!({"id":"new","name":"New","parent":"root","defaults":{"project":"suger"}})
            )
            .is_err()
        );
        assert_eq!(tree.value(), before);
    }

    #[test]
    fn inherit_override_block_and_restore_each_field() {
        let mut tree = tree();
        let actor = FolderActor::local();
        let inherited = tree.resolve_defaults(&actor, "child").unwrap();
        assert_eq!(
            inherited.values[&DefaultKey::Project].as_deref(),
            Some("suger")
        );
        assert_eq!(inherited.sources[&DefaultKey::Machine], "root");
        tree.apply(
            &actor,
            "defaults",
            &json!({"id":"child","values":{"project":null,"machine":"falcon"}}),
        )
        .unwrap();
        let overridden = tree.resolve_defaults(&actor, "child").unwrap();
        assert_eq!(overridden.values[&DefaultKey::Project], None);
        assert_eq!(
            overridden.values[&DefaultKey::Machine].as_deref(),
            Some("falcon")
        );
        assert_eq!(
            overridden.values[&DefaultKey::Provider],
            inherited.values[&DefaultKey::Provider]
        );
        tree.apply(
            &actor,
            "defaults",
            &json!({"id":"child","values":{},"inherit":["project","machine"]}),
        )
        .unwrap();
        assert_eq!(tree.resolve_defaults(&actor, "child").unwrap(), inherited);
        tree.apply(
            &actor,
            "defaults",
            &json!({"id":"root","values":{"machine":"new-host"}}),
        )
        .unwrap();
        assert_eq!(
            tree.resolve_defaults(&actor, "child").unwrap().values[&DefaultKey::Machine].as_deref(),
            Some("new-host")
        );
    }

    #[test]
    fn mcp_binding_defaults_validate_inherit_and_clear() {
        let suger = r#"[{"name":"suger","plugin_id":"suger","tools":["suger_status","suger_read"],"sources":["github","local_env"]}]"#;
        let parsed = mcp_bindings(suger).unwrap();
        let digest = format!("sha256:{}", "a".repeat(64));
        assert_eq!(
            parsed[0].binding(digest.clone()).server.generation_digest,
            digest
        );
        for invalid in [
            "[]",
            "{}",
            r#"[{"name":"s","plugin_id":"suger","tools":[]}]"#,
            r#"[{"name":"s","plugin_id":"suger","tools":["a","a"]}]"#,
            r#"[{"name":"s","plugin_id":"suger","tools":["a"],"generation_digest":"x"}]"#,
            r#"[{"name":"s","plugin_id":"../x","tools":["a"]}]"#,
            r#"[{"name":"s","plugin_id":"a","tools":["a"]},{"name":"s","plugin_id":"b","tools":["a"]}]"#,
        ] {
            assert!(
                parse(&json!({"mcp_bindings":invalid})).is_err(),
                "{invalid}"
            );
        }
        let mut tree = tree();
        let actor = FolderActor::local();
        tree.apply(
            &actor,
            "defaults",
            &json!({"id":"root","values":{"mcp_bindings":suger}}),
        )
        .unwrap();
        let child = tree.resolve_defaults(&actor, "child").unwrap();
        assert_eq!(
            child.values[&DefaultKey::McpBindings].as_deref(),
            Some(suger)
        );
        assert_eq!(child.sources[&DefaultKey::McpBindings], "root");
        tree.apply(
            &actor,
            "defaults",
            &json!({"id":"child","values":{"mcp_bindings":null}}),
        )
        .unwrap();
        let blocked = tree.resolve_defaults(&actor, "child").unwrap();
        assert_eq!(blocked.values[&DefaultKey::McpBindings], None);
        tree.apply(
            &actor,
            "defaults",
            &json!({"id":"child","values":{},"inherit":["mcp_bindings"]}),
        )
        .unwrap();
        assert_eq!(tree.resolve_defaults(&actor, "child").unwrap(), child);
    }

    #[test]
    fn moving_changes_only_inherited_fields() {
        let mut tree = tree();
        let actor = FolderActor::local();
        tree.apply(
            &actor,
            "defaults",
            &json!({"id":"child","values":{"provider":"codex"}}),
        )
        .unwrap();
        tree.apply(&actor, "move", &json!({"id":"child","parent":null}))
            .unwrap();
        let resolved = tree.resolve_defaults(&actor, "child").unwrap();
        assert!(!resolved.values.contains_key(&DefaultKey::Machine));
        assert_eq!(
            resolved.values[&DefaultKey::Provider].as_deref(),
            Some("codex")
        );
    }

    #[test]
    fn validation_and_owner_failures_never_mutate() {
        assert!(parse(&json!({"machine":"local"})).is_err());
        let mut tree = tree();
        let before = tree.value();
        for args in [
            json!({"id":"root","values":{"secret":"x"}}),
            json!({"id":"root","values":{"machine":42}}),
            json!({"id":"root","values":{"preset":"opus"}}),
            json!({"id":"root","values":{"machine":"hawk"},"inherit":["machine"]}),
        ] {
            assert!(
                tree.apply(&FolderActor::local(), "defaults", &args)
                    .is_err()
            );
            assert_eq!(tree.value(), before);
        }
        let other = FolderActor {
            user_id: Some("other".into()),
            sees_all: false,
        };
        assert!(tree.resolve_defaults(&other, "root").is_err());
        assert!(
            tree.apply(
                &other,
                "defaults",
                &json!({"id":"root","values":{"machine":"other"}})
            )
            .is_err()
        );
        assert_eq!(tree.value(), before);
    }
}
