use super::*;
use std::fs;
use std::os::unix::fs::{PermissionsExt as _, symlink};

fn config() -> serde_json::Value {
    serde_json::json!({
        "schema": 1, "service_id": "service-test", "machine_id": "machine-test",
        "purposes": {"binding": true, "machine_recovery": false, "service_resolution": false},
        "legacy_fence": "retain_managed_namespace"
    })
}

fn write(path: &Path, value: &serde_json::Value) {
    fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

pub(crate) fn policy(
    path: &Path,
    service: &str,
    machine: &str,
    purposes: [bool; 3],
) -> Arc<WriterAdmission> {
    let mut value = config();
    value["service_id"] = service.into();
    value["machine_id"] = machine.into();
    value["purposes"] = serde_json::json!({
        "binding": purposes[0], "machine_recovery": purposes[1], "service_resolution": purposes[2]
    });
    write(path, &value);
    WriterAdmission::load(path).unwrap()
}

#[test]
fn exact_target_and_nominal_purposes_are_independent() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("policy.json");
    for bits in 0..8 {
        let enabled = [bits & 1 != 0, bits & 2 != 0, bits & 4 != 0];
        let policy = policy(&path, "service-test", "machine-test", enabled);
        assert_eq!(
            policy
                .scope::<BindingWrites>("service-test", "machine-test")
                .is_some(),
            enabled[0]
        );
        assert_eq!(
            policy
                .scope::<MachineRecovery>("service-test", "machine-test")
                .is_some(),
            enabled[1]
        );
        assert_eq!(
            policy
                .scope::<ServiceResolution>("service-test", "machine-test")
                .is_some(),
            enabled[2]
        );
        assert!(!policy.allows::<BindingWrites>("service-other", Some("machine-test")));
        assert!(!policy.allows::<BindingWrites>("service-test", Some("other")));
        assert!(
            policy.current(),
            "a mismatching caller cannot revoke host policy"
        );
    }
}

#[test]
fn policy_is_closed_private_and_requires_explicit_fence_acknowledgment() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("policy.json");
    let original = config();
    for (pointer, value) in [
        ("/schema", serde_json::json!(2)),
        ("/service_id", serde_json::json!("service\u{0000}invalid")),
        ("/machine_id", serde_json::json!("machine/path")),
        ("/machine_id", serde_json::json!("")),
        ("/purposes/binding", serde_json::json!(1)),
        ("/purposes", serde_json::json!({"binding": true})),
        (
            "/purposes",
            serde_json::json!({"binding": true, "machine_recovery": false, "service_resolution": false, "export": true}),
        ),
        ("/legacy_fence", serde_json::json!("restore_legacy")),
        (
            "/legacy_fence",
            serde_json::json!({"retain_managed_namespace": null}),
        ),
        ("/legacy_fence", serde_json::Value::Null),
    ] {
        let mut changed = original.clone();
        *changed.pointer_mut(pointer).unwrap() = value;
        write(&path, &changed);
        assert!(WriterAdmission::load(&path).is_err(), "{pointer}");
    }
    for field in [
        "schema",
        "service_id",
        "machine_id",
        "purposes",
        "legacy_fence",
    ] {
        let mut changed = original.clone();
        changed.as_object_mut().unwrap().remove(field);
        write(&path, &changed);
        assert!(WriterAdmission::load(&path).is_err(), "{field}");
    }
    for field in ["endpoint", "token", "actor", "replay", "export"] {
        let mut changed = original.clone();
        changed[field] = "must-not-be-reported".into();
        write(&path, &changed);
        let error = WriterAdmission::load(&path).err().unwrap();
        assert!(!format!("{error:#}").contains("must-not-be-reported"));
    }
    assert!(WriterAdmission::load(Path::new("relative.json")).is_err());
}

#[test]
fn replacing_or_revoking_policy_never_revives_outstanding_scopes() {
    for boundary in [
        "replace",
        "restore_bytes",
        "remove",
        "symlink",
        "hardlink",
        "permissions",
        "corrupt",
        "oversize",
    ] {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("policy.json");
        let saved = root.path().join("saved.json");
        let original = config();
        write(&path, &original);
        let policy = WriterAdmission::load(&path).unwrap();
        let scope = policy
            .scope::<BindingWrites>("service-test", "machine-test")
            .unwrap();
        assert!(scope.check());
        match boundary {
            "replace" => {
                write(&saved, &original);
                fs::rename(&saved, &path).unwrap();
            }
            "restore_bytes" => {
                fs::write(&path, b"{}").unwrap();
                write(&path, &original);
            }
            "remove" => fs::rename(&path, &saved).unwrap(),
            "symlink" => {
                fs::rename(&path, &saved).unwrap();
                symlink(&saved, &path).unwrap();
            }
            "hardlink" => fs::hard_link(&path, &saved).unwrap(),
            "permissions" => fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap(),
            "corrupt" => fs::write(&path, b"{}").unwrap(),
            "oversize" => fs::write(&path, vec![b' '; 64 * 1024 + 1]).unwrap(),
            _ => unreachable!(),
        }
        assert!(!scope.check(), "{boundary}");
        assert!(
            policy
                .scope::<BindingWrites>("service-test", "machine-test")
                .is_none()
        );
        if boundary == "symlink" {
            fs::remove_file(&path).unwrap();
        }
        if boundary == "hardlink" {
            fs::remove_file(&saved).unwrap();
        }
        write(&path, &original);
        assert!(!scope.check());
        assert!(!policy.current());
        assert!(WriterAdmission::load(&path).unwrap().current());
    }
}

#[cfg(feature = "machine-host")]
#[test]
fn only_absence_is_optional_and_reading_does_not_create_state() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(MACHINE_POLICY_FILE);
    assert!(WriterAdmission::load_optional(&path).unwrap().is_none());
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    fs::write(&path, b"{}").unwrap();
    assert!(WriterAdmission::load_optional(&path).is_err());
}
