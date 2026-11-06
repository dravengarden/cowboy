use super::*;
use patterns::{Library, Source};

fn source() -> Source {
    Source { revision: "fixture".into(), profile: "PROFILE.md".into(), files: BTreeMap::from([
        ("PROFILE.md".into(), "+++\nschema = \"cowboy.pattern-profile/v1\"\nid = \"global\"\nscope = \"global\"\nguidance = [\"AGENTS.md\"]\n[patterns]\nfollow = \"follow.md\"\nprocedure = \"procedure.md\"\n+++\n# Profile\n".into()),
        ("AGENTS.md".into(), "Follow explicit task authority.\n".into()),
        ("follow.md".into(), "+++\nschema = \"cowboy.pattern/v1\"\nid = \"follow\"\nrevision = 1\nrole = \"supervisor\"\nsummary = \"Finish authorized work\"\nevents = [\"turn.completed\"]\nuses = [\"procedure\"]\neffects = [\"session.follow_up\"]\n+++\n# Follow up\n".into()),
        ("procedure.md".into(), "+++\nschema = \"cowboy.pattern/v1\"\nid = \"procedure\"\nrevision = 1\nrole = \"procedure\"\nsummary = \"Owning workflow\"\n+++\n# Never infer permission\n".into()),
    ]) }
}

#[test]
fn superagent_bundle_pins_body_and_guidance_and_rejects_escapes() {
    let original = source();
    let first = Library::compile(std::slice::from_ref(&original))
        .unwrap()
        .resolve("global")
        .unwrap();
    assert!(first.complete());
    let mut changed = original.clone();
    changed
        .files
        .get_mut("AGENTS.md")
        .unwrap()
        .push_str("Revised authority.");
    let next = Library::compile(&[changed])
        .unwrap()
        .resolve("global")
        .unwrap();
    assert_ne!(first.digest, next.digest);
    assert_eq!(
        first.patterns["follow"].digest,
        next.patterns["follow"].digest
    );
    for path in ["../secret", "/etc/passwd", "folder/../secret", "C:\\secret"] {
        let mut changed = original.clone();
        changed.files.insert(path.into(), "secret".into());
        assert!(Library::compile(&[changed]).is_err(), "{path}");
    }
}

#[test]
fn superagent_inheritance_requires_exact_explicit_replacement() {
    let global = source();
    let mut workspace = global.clone();
    workspace.files.insert("PROFILE.md".into(), "+++\nschema = \"cowboy.pattern-profile/v1\"\nid = \"workspace\"\nscope = \"workspace\"\nextends = \"global\"\nguidance = [\"AGENTS.md\"]\n[replace.procedure]\nfrom = \"global/procedure@1\"\nfile = \"procedure.md\"\n+++\n# Workspace\n".into());
    let catalog = Library::compile(&[global.clone(), workspace.clone()])
        .unwrap()
        .resolve("workspace")
        .unwrap();
    assert_eq!(
        catalog.patterns["procedure"].origin,
        "workspace/procedure@1"
    );
    workspace
        .files
        .get_mut("PROFILE.md")
        .unwrap()
        .push_str("\nBody edit\n");
    assert!(Library::compile(&[global.clone(), workspace.clone()]).is_ok());
    *workspace.files.get_mut("PROFILE.md").unwrap() =
        workspace.files["PROFILE.md"].replace("global/procedure@1", "global/procedure@2");
    assert!(Library::compile(&[global, workspace]).is_err());
}

#[test]
fn superagent_unknown_and_duplicate_keys_cannot_become_operational_rules() {
    for field in [
        "deploy = true",
        "scope = \"project\"",
        "schema = \"cowboy.pattern-profile/v2\"",
    ] {
        let mut bundle = source();
        *bundle.files.get_mut("PROFILE.md").unwrap() =
            bundle.files["PROFILE.md"].replacen("[patterns]", &format!("{field}\n[patterns]"), 1);
        assert!(Library::compile(&[bundle]).is_err());
    }
}

#[test]
fn superagent_session_off_and_mandate_are_independent_from_patterns() {
    let mut config = Settings {
        sources: vec![source()],
        ..Settings::default()
    };
    config.projects.insert(
        "cowboy".into(),
        Project {
            profile: "global".into(),
            coordinator: "coordinator".into(),
            mode: Mode::Observe,
            preset: Preset::AstraMax,
            max_followups: 2,
            learning: false,
        },
    );
    config.sessions.insert(
        "worker".into(),
        Participation {
            project: "cowboy".into(),
            mode: None,
            mandate: String::new(),
        },
    );
    assert!(config.validate().is_ok());
    config.projects.get_mut("cowboy").unwrap().mode = Mode::Coordinate;
    assert_eq!(config.validate(), Err("superagent_mandate_required"));
    config.sessions.get_mut("worker").unwrap().mode = Some(Mode::Off);
    assert!(config.validate().is_ok());
    config
        .sessions
        .insert("coordinator".into(), config.sessions["worker"].clone());
    assert!(config.validate().is_err());
}

#[tokio::test]
async fn superagent_sparse_settings_preserve_library_and_original_receipts() {
    use crate::store::preferences::{PreferenceChange, PreferenceResult};
    use serde_json::json;
    let dir = tempfile::tempdir().unwrap();
    let store = crate::store::Store::connect("sqlite::memory:", dir.path().to_owned())
        .await
        .unwrap();
    store.migrate().await.unwrap();
    let source = source();
    let imported = PreferenceChange {
        scope: SCOPE.into(),
        expected_revision: 0,
        changes: json!({"configuration":{"sources":[source]}}),
    };
    assert!(matches!(
        store
            .update_preferences("owner", "import", &imported)
            .await
            .unwrap(),
        PreferenceResult::Applied(_)
    ));
    let project = PreferenceChange {
        scope: SCOPE.into(),
        expected_revision: 1,
        changes: json!({"configuration":{"projects":{"cowboy":{
            "profile":"global","coordinator":"coordinator","preset":"astra-max","mode":"off"
        }}}}),
    };
    let first = store
        .update_preferences("owner", "project", &project)
        .await
        .unwrap();
    let PreferenceResult::Applied(snapshot) = &first else {
        panic!("project not committed");
    };
    assert_eq!(snapshot.values["configuration"]["sources"], json!([source]));
    let discovery = settings_discovery(json!({"preferences":snapshot}));
    assert!(
        discovery["preferences"]["values"]["configuration"]
            .get("sources")
            .is_none()
    );
    assert_eq!(
        discovery["preferences"]["source_catalog"][0]["file_count"],
        4
    );
    let invalid = PreferenceChange {
        expected_revision: 2,
        changes: json!({"configuration":{"sources":[]}}),
        ..project.clone()
    };
    assert!(matches!(
        store
            .update_preferences("owner", "invalid", &invalid)
            .await
            .unwrap(),
        PreferenceResult::Invalid(_)
    ));
    let remove = PreferenceChange {
        expected_revision: 2,
        changes: json!({"configuration":{"projects":{}}}),
        ..project.clone()
    };
    assert!(matches!(
        store
            .update_preferences("owner", "remove", &remove)
            .await
            .unwrap(),
        PreferenceResult::Applied(_)
    ));
    assert_eq!(
        store
            .update_preferences("owner", "project", &project)
            .await
            .unwrap(),
        first
    );
}
