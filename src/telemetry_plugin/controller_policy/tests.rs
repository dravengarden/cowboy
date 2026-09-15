use super::*;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _, symlink};
use std::path::Path;

fn write(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

fn fixture() -> (tempfile::TempDir, ServeArgs, String) {
    let root = tempfile::tempdir().unwrap();
    let data = root.path().join("service");
    let service = crate::service_identity::load_or_create(&data).unwrap();
    let mut args = ServeArgs::test_plugin_check(&data);
    args.plugin_catalog_dir = Some(root.path().join("absent-catalog"));
    args.database_url = Some(format!("sqlite://{}", data.join("untouched.db").display()));
    // An actual connection/migration would fail, as well as change evidence.
    write(
        &data.join("untouched.db"),
        b"not a database; must not be opened",
    );
    (root, args, service.as_str().to_owned())
}

fn writer(service: &str) -> Value {
    json!({
        "schema":1, "service_id":service, "machine_id":"fixture-machine",
        "purposes":{"binding":true,"machine_recovery":false,"service_resolution":false},
        "legacy_fence":"retain_managed_namespace"
    })
}

fn background(service: &str) -> Value {
    json!({
        "schema":1, "service_id":service, "machine_id":"fixture-machine",
        "binding":crate::machine_protocol::telemetry_export::fixture().binding,
        "signals":{"logs":true,"metrics":true,"traces":true},
        "startup":"activate_exact_binding"
    })
}

// Compare full fixture trees and metadata, but never print policy/database bytes.
fn fingerprint(root: &Path) -> BTreeMap<String, String> {
    fn visit(path: &Path, root: &Path, files: &mut BTreeMap<String, String>) {
        let m = path.symlink_metadata().unwrap();
        let bytes = if m.is_file() {
            fs::read(path).unwrap()
        } else {
            Vec::new()
        };
        files.insert(
            path.strip_prefix(root).unwrap().display().to_string(),
            format!(
                "{}:{}:{}:{}:{}:{}:{}:{}",
                m.dev(),
                m.ino(),
                m.mode(),
                m.nlink(),
                m.uid(),
                m.ctime(),
                m.ctime_nsec(),
                crate::admin::hex_sha256(&bytes)
            ),
        );
        if m.is_dir() {
            for entry in fs::read_dir(path).unwrap() {
                visit(&entry.unwrap().path(), root, files);
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

#[tokio::test]
async fn host_preflight_validates_managed_policies_without_opening_store_or_creating_state() {
    for (has_writer, has_background) in [(true, false), (false, true), (true, true)] {
        let (root, mut args, service) = fixture();
        let policy = root.path().join("writer.json");
        write(&policy, &serde_json::to_vec(&writer(&service)).unwrap());
        args.telemetry_writer_policy = has_writer.then_some(policy);
        let policy = root.path().join("background.json");
        write(&policy, &serde_json::to_vec(&background(&service)).unwrap());
        args.telemetry_managed_export_policy = has_background.then_some(policy);
        let before = fingerprint(root.path());
        let report = serde_json::to_value(inspect(&args).unwrap()).unwrap();
        assert_eq!(
            report["writer_policy"],
            if has_writer {
                "configuration_valid"
            } else {
                "unconfigured"
            }
        );
        assert_eq!(
            report["managed_background_policy"],
            if has_background {
                "configuration_valid"
            } else {
                "unconfigured"
            }
        );
        assert_eq!(report["legacy_selection"], "unconfigured");
        let serialized = report.to_string();
        for hidden in [
            &service,
            "fixture-machine",
            "writer.json",
            "background.json",
            "sha256:",
            "sqlite:",
        ] {
            assert!(!serialized.contains(hidden));
        }
        // Exercise the actual early-return branch, not just its helper.
        tokio::time::timeout(
            std::time::Duration::from_secs(1),
            crate::server::serve(args),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(before, fingerprint(root.path()));
    }
}

#[tokio::test]
async fn host_preflight_and_startup_share_exact_policy_rejection_without_echoing_values() {
    for managed in [false, true] {
        for fault in [
            "unknown",
            "schema",
            "owner",
            "target",
            "shape",
            "public",
            "link",
            "hardlink",
            "missing",
            "oversized",
            "no_store",
            "conflict",
        ] {
            let (root, mut args, service) = fixture();
            let mut document = if managed {
                background(&service)
            } else {
                writer(&service)
            };
            match fault {
                "unknown" => {
                    document["must-not-be-reported"] = "private-fixture-value".into();
                }
                "schema" => document["schema"] = 2.into(),
                "owner" => document["service_id"] = "svc-00000000000000000000000000000000".into(),
                "target" => document["machine_id"] = "private-fixture-value/invalid".into(),
                "shape" => {
                    if managed {
                        document["binding"]["revision"] = 1.into();
                    } else {
                        document["purposes"]["binding"] = 1.into();
                    }
                }
                _ => {}
            }
            let policy = root.path().join("policy.json");
            write(&policy, &serde_json::to_vec(&document).unwrap());
            if managed {
                args.telemetry_managed_export_policy = Some(policy.clone());
            } else {
                args.telemetry_writer_policy = Some(policy.clone());
            }
            match fault {
                "public" => {
                    fs::set_permissions(&policy, fs::Permissions::from_mode(0o644)).unwrap()
                }
                "link" => {
                    fs::rename(&policy, root.path().join("target")).unwrap();
                    symlink(root.path().join("target"), &policy).unwrap();
                }
                "hardlink" => fs::hard_link(&policy, root.path().join("other-link")).unwrap(),
                "missing" => fs::rename(&policy, root.path().join("removed-policy")).unwrap(),
                "oversized" => write(&policy, &vec![b' '; 64 * 1024 + 1]),
                "no_store" => args.database_url = None,
                "conflict" => {
                    if !managed {
                        let background_path = root.path().join("background.json");
                        write(
                            &background_path,
                            &serde_json::to_vec(&background(&service)).unwrap(),
                        );
                        args.telemetry_managed_export_policy = Some(background_path);
                    }
                    args.telemetry_plugin_config = Some(root.path().join("unread-legacy"));
                }
                _ => {}
            }
            let before = fingerprint(root.path());
            let startup_error = ControllerPolicy::load(&args, &service)
                .err()
                .unwrap()
                .to_string();
            let inspect_error = inspect(&args).unwrap_err().to_string();
            assert_eq!(startup_error, inspect_error, "{managed}/{fault}");
            let error = crate::server::serve(args).await.unwrap_err().to_string();
            assert_eq!(error, inspect_error, "{managed}/{fault}");
            assert!(
                !error.contains("private-fixture-value") && !error.contains("must-not-be-reported")
            );
            assert_eq!(before, fingerprint(root.path()));
        }
    }
}

#[tokio::test]
async fn host_preflight_does_not_invent_an_identity_or_follow_an_unsafe_identity_file() {
    for fault in [
        "absent_root",
        "absent",
        "invalid",
        "oversized",
        "link",
        "directory",
        "fifo",
    ] {
        let (root, mut args, service) = fixture();
        let policy = root.path().join("writer.json");
        write(&policy, &serde_json::to_vec(&writer(&service)).unwrap());
        args.telemetry_writer_policy = Some(policy);
        let path = args.data_dir.join("service-id");
        fs::rename(&path, root.path().join("saved-identity")).unwrap();
        match fault {
            "absent_root" => args.data_dir = root.path().join("never-created"),
            "absent" => {}
            "invalid" => write(&path, b"must-not-be-reported"),
            "oversized" => write(&path, &[b' '; 129]),
            "link" => symlink(root.path().join("saved-identity"), &path).unwrap(),
            "directory" => fs::create_dir(&path).unwrap(),
            "fifo" => rustix::fs::mknodat(
                rustix::fs::CWD,
                &path,
                rustix::fs::FileType::Fifo,
                rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
                0,
            )
            .unwrap(),
            _ => unreachable!(),
        }
        let before = fingerprint(root.path());
        let error = crate::server::serve(args).await.unwrap_err().to_string();
        assert!(!error.contains("must-not-be-reported"));
        assert_eq!(before, fingerprint(root.path()));
    }
}

#[tokio::test]
async fn host_preflight_declares_unchecked_legacy_and_never_contacts_database() {
    let (root, mut args, _) = fixture();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    args.database_url = Some(format!(
        "postgresql://fixture@{}/never-connect",
        listener.local_addr().unwrap()
    ));
    // A broken legacy file can be intentionally irrelevant behind a durable
    // fence. Preflight cannot inspect that fence without opening the store.
    args.telemetry_plugin_config = Some(root.path().join("does-not-exist"));
    let report = serde_json::to_value(inspect(&args).unwrap()).unwrap();
    assert_eq!(report["legacy_selection"], "not_checked");
    assert_eq!(report["writer_policy"], "unconfigured");
    assert_eq!(report["managed_background_policy"], "unconfigured");
    let before = fingerprint(root.path());
    crate::server::serve(args).await.unwrap();
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    assert_eq!(before, fingerprint(root.path()));
}
