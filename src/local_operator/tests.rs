use super::*;
use clap::Parser;
use std::os::unix::fs::{PermissionsExt as _, symlink};

#[test]
fn host_grant_is_explicit_private_revocable_and_epoch_bound() {
    let root = tempfile::tempdir().unwrap();
    let dir = directory(root.path()).unwrap();
    let live = Arc::new(AtomicBool::new(true));
    assert!(Grant::capture(&dir, uid(), live.clone()).is_err());
    enable(&dir).unwrap();
    assert!(Grant::capture(&dir, uid().saturating_add(1), live.clone()).is_err());
    let grant = Grant::capture(&dir, uid(), live.clone()).unwrap();
    assert!(grant.current());
    assert_eq!(
        grant.actor(),
        crate::plugin_operation::Actor::Admin {
            account: format!("unix-uid:{}", uid())
        }
    );
    enable(&dir).unwrap();
    assert!(
        !grant.current(),
        "a replacement policy cannot renew an existing operation"
    );
    let fresh = Grant::capture(&dir, uid(), live.clone()).unwrap();
    std::fs::set_permissions(dir.join(GRANT_NAME), std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(!fresh.current());
    assert!(Grant::capture(&dir, uid(), live.clone()).is_err());
    std::fs::set_permissions(dir.join(GRANT_NAME), std::fs::Permissions::from_mode(0o600)).unwrap();
    live.store(false, Ordering::Release);
    assert!(!fresh.current());
    assert!(Grant::capture(&dir, uid(), live).is_err());
}

#[test]
fn host_policy_refuses_symlinks_hardlinks_wrong_schema_and_shared_directories() {
    let root = tempfile::tempdir().unwrap();
    let dir = directory(root.path()).unwrap();
    enable(&dir).unwrap();
    let path = dir.join(GRANT_NAME);
    let original = dir.join("original.json");
    std::fs::rename(&path, &original).unwrap();
    symlink(&original, &path).unwrap();
    assert!(policy(&dir).is_err());
    std::fs::remove_file(&path).unwrap();
    std::fs::hard_link(&original, &path).unwrap();
    assert!(policy(&dir).is_err());
    std::fs::remove_file(&path).unwrap();
    enable(&dir).unwrap();
    std::fs::write(&path, br#"{"schema":2,"generation":"untrusted"}"#).unwrap();
    assert!(policy(&dir).is_err());
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(directory(root.path()).is_err());
}

#[derive(Parser)]
struct TestCli {
    #[command(flatten)]
    args: OperatorArgs,
}

#[test]
fn install_cli_requires_a_version_digest_and_durable_operation_identity() {
    assert!(
        TestCli::try_parse_from([
            "operator",
            "install",
            "--machine",
            "hawk",
            "--plugin",
            "claude-code"
        ])
        .is_err()
    );
    let args = TestCli::try_parse_from([
        "operator",
        "upgrade",
        "--machine",
        "hawk",
        "--plugin",
        "claude-code",
        "--version",
        "3.1.24",
        "--digest",
        &format!("sha256:{}", "a".repeat(64)),
        "--operation-id",
        "reviewed-operation-1",
    ])
    .unwrap();
    assert!(
        matches!(args.args.command, OperatorCommand::Install { operation_id, .. } if operation_id == "reviewed-operation-1")
    );
    assert!(TestCli::try_parse_from(["operator", "shell", "anything"]).is_err());
}
