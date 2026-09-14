use super::*;
use crate::telemetry_plugin::writer_admission::{MACHINE_POLICY_FILE, preflight};
use clap::{CommandFactory as _, FromArgMatches as _};
use std::fs;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _, symlink};

fn args(root: &Path, extra: &[&str]) -> Args {
    parse_args(root, extra).unwrap()
}

fn parse_args(root: &Path, extra: &[&str]) -> Result<Args, clap::Error> {
    // No inherited Machine enrollment token, home or host configuration is a
    // test input. Remove clap environment bindings without mutating process env.
    let mut command = Args::command();
    let ids = command
        .get_arguments()
        .map(|arg| arg.get_id().clone())
        .collect::<Vec<_>>();
    for id in ids {
        command = command.mut_arg(id, |arg| arg.env(None::<&str>));
    }
    let matches = command.try_get_matches_from(
        ["cowboy-machine", "--state-dir", root.to_str().unwrap()]
            .into_iter()
            .chain(extra.iter().copied()),
    )?;
    Args::from_arg_matches(&matches)
}

fn config() -> serde_json::Value {
    serde_json::json!({
        "schema": 1, "service_id": "service-fixture", "machine_id": "machine-fixture",
        "purposes": {"binding": true, "machine_recovery": false, "service_resolution": false},
        "legacy_fence": "retain_managed_namespace"
    })
}

fn write_policy(root: &Path, value: &serde_json::Value) {
    let path = root.join(MACHINE_POLICY_FILE);
    fs::write(&path, serde_json::to_vec(value).unwrap()).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
}

#[derive(Debug, PartialEq, Eq)]
struct Entry {
    mode: u32,
    owner: u32,
    inode: u64,
    modified: (i64, i64),
    changed: (i64, i64),
    digest_or_link: String,
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, Entry> {
    use sha2::{Digest as _, Sha256};
    fn visit(root: &Path, path: &Path, entries: &mut BTreeMap<PathBuf, Entry>) {
        let metadata = path.symlink_metadata().unwrap();
        let digest_or_link = if metadata.file_type().is_symlink() {
            fs::read_link(path).unwrap().display().to_string()
        } else if metadata.is_file() {
            format!("{:x}", Sha256::digest(fs::read(path).unwrap()))
        } else {
            String::new()
        };
        entries.insert(
            path.strip_prefix(root).unwrap().to_owned(),
            Entry {
                mode: metadata.mode(),
                owner: metadata.uid(),
                inode: metadata.ino(),
                modified: (metadata.mtime(), metadata.mtime_nsec()),
                changed: (metadata.ctime(), metadata.ctime_nsec()),
                digest_or_link,
            },
        );
        if metadata.is_dir() {
            for entry in fs::read_dir(path).unwrap() {
                visit(root, &entry.unwrap().path(), entries);
            }
        }
    }
    let mut entries = BTreeMap::new();
    visit(root, root, &mut entries);
    entries
}

#[tokio::test]
async fn missing_state_can_be_inspected_without_creating_it_or_requiring_a_controller() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("not-created");
    let args = args(&missing, &["--check-telemetry-writer-policy"]);
    assert!(args.controller_url.is_none());
    run_args(args).await.unwrap();
    let report = serde_json::to_value(preflight::inspect(&missing).unwrap()).unwrap();
    assert_eq!(
        report["writer_policy"],
        serde_json::json!({"state":"unconfigured"})
    );
    assert!(!missing.exists());
}

#[tokio::test]
async fn preflight_does_not_open_live_journal_other_state_or_enrollment() {
    let root = tempfile::tempdir().unwrap();
    let store = MachinePluginStore::new(root.path(), Platform::Linux, "x86_64".into()).unwrap();
    write_policy(root.path(), &config());
    // Invalid unrelated bytes and an existing exclusive owner must not be
    // touched or adopted by this narrowly named configuration check.
    fs::write(
        root.path().join("telemetry.json"),
        b"fixture-private-destination-must-not-be-read",
    )
    .unwrap();
    fs::write(
        root.path().join("provider-usage.sqlite3"),
        b"not-a-database",
    )
    .unwrap();
    fs::write(
        root.path().join("machine-id"),
        b"persisted-fixture-different-from-cli",
    )
    .unwrap();
    fs::write(
        root.path()
            .join("plugin-operations/telemetry-bindings-v1.json"),
        b"invalid-journal",
    )
    .unwrap();
    let token = root.path().join("enrollment-token");
    fs::write(&token, b"fixture-enrollment-token-must-survive").unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let controller = format!("http://{}", listener.local_addr().unwrap());
    let before = snapshot(root.path());
    run_args(args(
        root.path(),
        &[
            "--check-telemetry-writer-policy",
            "--controller-url",
            &controller,
            "--enrollment-token-file",
            token.to_str().unwrap(),
            "--workspace-config",
            "fixture-workspace-must-not-be-read",
            "--socket",
            root.path().join("broker.sock").to_str().unwrap(),
            "--worker-command",
            "fixture-worker-must-not-spawn",
        ],
    ))
    .await
    .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(50), listener.accept())
            .await
            .is_err()
    );
    assert_eq!(before, snapshot(root.path()));
    drop(store);
}

#[test]
fn report_exposes_only_closed_declarations_not_owner_or_readiness() {
    let root = tempfile::tempdir().unwrap();
    for bits in 0..8 {
        let mut policy = config();
        policy["purposes"] = serde_json::json!({
            "binding": bits & 1 != 0, "machine_recovery": bits & 2 != 0, "service_resolution": bits & 4 != 0
        });
        write_policy(root.path(), &policy);
        let before = snapshot(root.path());
        let report = serde_json::to_value(preflight::inspect(root.path()).unwrap()).unwrap();
        assert_eq!(
            report["schema"],
            "dravengarden.cowboy.machine-telemetry-writer-preflight/v1"
        );
        assert_eq!(
            report["writer_policy"],
            serde_json::json!({
                "state": "configuration_valid", "declared_purposes": policy["purposes"]
            })
        );
        assert_eq!(report.as_object().unwrap().len(), 3);
        assert_eq!(report["not_checked"].as_array().unwrap().len(), 6);
        let encoded = report.to_string();
        for hidden in [
            "service-fixture",
            "machine-fixture",
            "runtime_ready",
            "export_active",
            "retain_managed_namespace",
        ] {
            assert!(!encoded.contains(hidden));
        }
        assert_eq!(before, snapshot(root.path()));
    }
}

#[tokio::test]
async fn invalid_writer_is_rejected_before_normal_cli_initialization() {
    let root = tempfile::tempdir().unwrap();
    write_policy(
        root.path(),
        &serde_json::json!({"fixture-private-key": "fixture-secret"}),
    );
    fs::create_dir(root.path().join("providers")).unwrap();
    let before = snapshot(root.path());
    let error = run_args(args(
        root.path(),
        &["--controller-url", "http://127.0.0.1:1"],
    ))
    .await
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("invalid telemetry configuration")
    );
    assert!(!format!("{error:#}").contains("fixture"));
    assert_eq!(before, snapshot(root.path()));
}

#[tokio::test]
async fn preflight_and_normal_startup_share_closed_policy_rejection() {
    let root = tempfile::tempdir().unwrap();
    for (pointer, value) in [
        ("/schema", serde_json::json!(2)),
        (
            "/service_id",
            // Service IDs are opaque bounded strings, not path components.
            // Slash is valid under the existing shared wire contract; a
            // control character is not. Do not narrow that contract here.
            serde_json::json!("invalid\nfixture-private-owner"),
        ),
        (
            "/machine_id",
            serde_json::json!("invalid/fixture-private-machine"),
        ),
        ("/purposes", serde_json::json!({"binding":true})),
        (
            "/purposes/binding",
            serde_json::json!("fixture-private-value"),
        ),
        (
            "/legacy_fence",
            serde_json::json!("restore-legacy-fixture-private"),
        ),
    ] {
        let mut value_to_write = config();
        *value_to_write.pointer_mut(pointer).unwrap() = value;
        write_policy(root.path(), &value_to_write);
        let before = snapshot(root.path());
        let checked = preflight::inspect(root.path()).expect_err(pointer);
        let normal = run_args(args(
            root.path(),
            &["--controller-url", "http://127.0.0.1:1"],
        ))
        .await
        .unwrap_err();
        assert_eq!(format!("{checked:#}"), format!("{normal:#}"));
        assert!(!format!("{checked:#}").contains("fixture-private"));
        assert_eq!(before, snapshot(root.path()));
    }
}

#[tokio::test]
async fn unsafe_writer_files_are_rejected_without_initialization_or_blocking() {
    for case in [
        "symlink",
        "dangling",
        "hardlink",
        "directory",
        "fifo",
        "public",
        "oversize",
    ] {
        let root = tempfile::tempdir().unwrap();
        let policy = root.path().join(MACHINE_POLICY_FILE);
        let other = root.path().join("other");
        match case {
            "symlink" => {
                fs::write(&other, b"{}").unwrap();
                symlink(&other, &policy).unwrap();
            }
            "dangling" => symlink(&other, &policy).unwrap(),
            "directory" => fs::create_dir(&policy).unwrap(),
            "fifo" => rustix::fs::mkfifoat(
                rustix::fs::CWD,
                &policy,
                rustix::fs::Mode::from_raw_mode(0o600),
            )
            .unwrap(),
            _ => {
                write_policy(root.path(), &config());
                match case {
                    "hardlink" => fs::hard_link(&policy, &other).unwrap(),
                    "public" => {
                        fs::set_permissions(&policy, fs::Permissions::from_mode(0o644)).unwrap()
                    }
                    "oversize" => fs::write(&policy, vec![b' '; 64 * 1024 + 1]).unwrap(),
                    _ => unreachable!(),
                }
            }
        }
        let before = snapshot(root.path());
        assert!(preflight::inspect(root.path()).is_err(), "{case}");
        assert!(
            run_args(args(
                root.path(),
                &["--controller-url", "http://127.0.0.1:1"],
            ))
            .await
            .is_err(),
            "{case}"
        );
        assert_eq!(before, snapshot(root.path()));
        assert!(!root.path().join("plugin-operations").exists());
    }
}

#[tokio::test]
async fn diagnostic_modes_are_exclusive_even_for_programmatic_callers() {
    let root = tempfile::tempdir().unwrap();
    let error = parse_args(
        root.path(),
        &["--check-telemetry-writer-policy", "--provider-usage-status"],
    )
    .unwrap_err();
    assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
    let mut both = args(root.path(), &["--check-telemetry-writer-policy"]);
    both.provider_usage_status = true;
    assert!(run_args(both).await.is_err());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
}
