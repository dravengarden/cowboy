use super::*;
use std::fs;
use std::os::unix::fs::{PermissionsExt as _, symlink};

fn config() -> serde_json::Value {
    let request = crate::machine_protocol::telemetry_export::fixture();
    serde_json::json!({
        "schema": 1, "service_id": request.service_id, "machine_id": request.machine_id,
        "binding": request.binding,
        "signals": {"logs": true, "metrics": true, "traces": false},
        "startup": "activate_exact_binding"
    })
}

fn write(path: &Path, value: &serde_json::Value) {
    fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}

fn payload() -> Export {
    crate::machine_protocol::telemetry_export::fixture().payload
}

#[test]
fn background_policy_requires_explicit_closed_exact_standing_authority() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("policy.json");
    let original = config();
    for (pointer, replacement) in [
        ("/schema", serde_json::json!(2)),
        ("/service_id", serde_json::json!("other-service")),
        ("/machine_id", serde_json::json!("machine/path")),
        ("/machine_id", serde_json::json!("")),
        ("/binding/revision", serde_json::json!(1)),
        ("/binding/revision", serde_json::json!("01")),
        (
            "/binding/policy_epoch",
            serde_json::json!("18446744073709551616"),
        ),
        ("/binding/selection", serde_json::Value::Null),
        (
            "/binding/selection/generation_digest",
            serde_json::json!("latest"),
        ),
        (
            "/binding/selection/installation_revision",
            serde_json::json!("latest"),
        ),
        ("/signals/logs", serde_json::json!(1)),
        ("/signals", serde_json::json!({"logs": true})),
        (
            "/signals",
            serde_json::json!({"logs": false, "metrics": false, "traces": false}),
        ),
        (
            "/signals",
            serde_json::json!({"logs": true, "metrics": true, "traces": true, "extra": true}),
        ),
        ("/startup", serde_json::json!("resume")),
        (
            "/startup",
            serde_json::json!({"activate_exact_binding": null}),
        ),
        (
            "/startup",
            serde_json::json!({"activate_exact_binding": {"extra": true}}),
        ),
    ] {
        let mut value = original.clone();
        *value.pointer_mut(pointer).unwrap() = replacement;
        write(&path, &value);
        assert!(
            BackgroundPolicy::load(&path, "service-test").is_err(),
            "{pointer}"
        );
    }
    for field in [
        "schema",
        "service_id",
        "machine_id",
        "binding",
        "signals",
        "startup",
    ] {
        let mut value = original.clone();
        value.as_object_mut().unwrap().remove(field);
        write(&path, &value);
        assert!(
            BackgroundPolicy::load(&path, "service-test").is_err(),
            "{field}"
        );
    }
    for field in [
        "endpoint",
        "token",
        "actor",
        "retry",
        "replay",
        "allow_writes",
    ] {
        let mut value = original.clone();
        value[field] = "must-not-be-reported".into();
        write(&path, &value);
        let error = BackgroundPolicy::load(&path, "service-test").err().unwrap();
        assert!(!format!("{error:#}").contains("must-not-be-reported"));
    }
    write(&path, &original);
    let policy = BackgroundPolicy::load(&path, "service-test").unwrap();
    assert!(policy.current());
    assert!(BackgroundPolicy::load(Path::new("relative.json"), "service-test").is_err());
    // Integer types preserve values above JS's safe range without rounding.
    let mut exact = original;
    exact["binding"]["revision"] = "9007199254740993".into();
    exact["binding"]["policy_epoch"] = "18446744073709551615".into();
    write(&path, &exact);
    assert!(BackgroundPolicy::load(&path, "service-test").is_ok());
}

#[test]
fn background_policy_replacement_and_unsafe_files_stop_the_original_activation() {
    for boundary in [
        "replace",
        "in_place_restore",
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
        let policy = BackgroundPolicy::load(&path, "service-test").unwrap();
        let (attempt, permit) = policy.admit(payload()).unwrap();
        match boundary {
            "replace" => {
                write(&saved, &original);
                fs::rename(&saved, &path).unwrap();
            }
            "in_place_restore" => {
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
        assert!(!permit.check(&attempt), "{boundary}");
        assert!(!policy.current(), "{boundary}");
        assert!(policy.admit(payload()).is_err(), "{boundary}");
        // An independently validated explicit reload is a new activation. The
        // old Arc and its outstanding grant stay stopped after repair.
        if boundary == "symlink" {
            fs::remove_file(&path).unwrap();
        }
        if boundary == "hardlink" {
            fs::remove_file(&saved).unwrap();
        }
        write(&path, &original);
        assert!(!policy.current());
        assert!(!permit.check(&attempt));
        assert!(
            BackgroundPolicy::load(&path, "service-test")
                .unwrap()
                .current()
        );
    }
}

#[test]
fn background_permits_bind_full_batch_and_original_budget_without_cross_purpose_conversion() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("policy.json");
    write(&path, &config());
    let policy = BackgroundPolicy::load(&path, "service-test").unwrap();
    let (attempt, permit) = policy.admit(payload()).unwrap();
    assert!(permit.check(&attempt));
    assert!(permit.remaining() <= ATTEMPT_BUDGET);
    let (next, _) = policy.admit(payload()).unwrap();
    assert_ne!(attempt.attempt_id, next.attempt_id);
    assert!(!permit.check(&next));
    assert!(
        !permit.check(&attempt),
        "mismatch permanently consumes this permit"
    );
    assert!(
        policy.current(),
        "one denied attempt does not revoke standing policy"
    );
    for pointer in [
        "/service_id",
        "/machine_id",
        "/binding/policy_epoch",
        "/payload/signal",
        "/expires_at_ms",
    ] {
        let (attempt, permit) = policy.admit(payload()).unwrap();
        let mut other = serde_json::to_value(&attempt).unwrap();
        *other.pointer_mut(pointer).unwrap() = match pointer {
            "/binding/policy_epoch" => "2".into(),
            "/payload/signal" => "metrics".into(),
            "/expires_at_ms" => 1.into(),
            _ => "foreign".into(),
        };
        assert!(
            !permit.check(&serde_json::from_value(other).unwrap()),
            "{pointer}"
        );
    }
    let mut disabled = payload();
    disabled.signal = Signal::Traces;
    assert!(policy.admit(disabled).is_err());
    let (attempt, permit) = policy.admit(payload()).unwrap();
    permit.budget.expire_for_test();
    assert!(!permit.check(&attempt));
    assert!(permit.remaining().is_zero());
}

#[test]
fn background_and_legacy_cli_modes_are_mutually_exclusive() {
    use clap::Parser as _;
    assert!(
        crate::cli::Cli::try_parse_from([
            "cowboy",
            "serve",
            "--telemetry-plugin-config",
            "/private/legacy.json",
            "--telemetry-managed-export-policy",
            "/private/managed.json",
        ])
        .is_err()
    );
    assert!(
        crate::cli::Cli::try_parse_from([
            "cowboy",
            "serve",
            "--telemetry-managed-export-policy",
            "/private/managed.json",
        ])
        .is_ok()
    );
}
