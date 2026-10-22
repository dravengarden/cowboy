use super::*;
use crate::telemetry_plugin::writer_admission::MACHINE_POLICY_FILE;

#[test]
fn invalid_writer_does_not_create_a_journal_owner() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join(MACHINE_POLICY_FILE), b"{}").unwrap();
    fs::set_permissions(
        root.path().join(MACHINE_POLICY_FILE),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    assert!(operations::Journal::open(root.path()).is_err());
    assert!(
        !root.path().join("plugin-operations").exists(),
        "invalid policy must precede journal initialization"
    );
}

#[test]
fn invalid_writer_does_not_initialize_auth_or_migrate_legacy_plugins() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("providers")).unwrap();
    fs::write(
        root.path().join("providers/fixture"),
        b"retained fixture bytes",
    )
    .unwrap();
    fs::write(root.path().join(MACHINE_POLICY_FILE), b"{}").unwrap();
    fs::set_permissions(
        root.path().join(MACHINE_POLICY_FILE),
        fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    assert!(MachinePluginStore::new(root.path(), Platform::Linux, "x86_64".into()).is_err());
    for unexpected in ["plugin-operations", "provider-auth", "plugins"] {
        assert!(
            !root.path().join(unexpected).exists(),
            "invalid policy must precede {unexpected} initialization"
        );
    }
    assert_eq!(
        fs::read(root.path().join("providers/fixture")).unwrap(),
        b"retained fixture bytes"
    );
}
