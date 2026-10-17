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
}

pub type FolderDefaults = BTreeMap<DefaultKey, Option<String>>;

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
