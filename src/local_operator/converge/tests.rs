use super::{
    CatalogRelease, InstalledPlugin, MachineTarget, SupportedPlatform, compare_versions,
    latest_ready, operation_id, plan_machine, rollout_order,
};
use std::cmp::Ordering;

fn release(plugin: &str, version: &str, state: &str) -> CatalogRelease {
    CatalogRelease {
        plugin_id: plugin.to_owned(),
        plugin_version: version.to_owned(),
        artifact_digest: format!("sha256:{version}"),
        release_state: state.to_owned(),
        supported_platforms: vec![
            SupportedPlatform {
                os: "linux".to_owned(),
                architecture: "x86_64".to_owned(),
            },
            SupportedPlatform {
                os: "macos".to_owned(),
                architecture: "aarch64".to_owned(),
            },
        ],
    }
}

fn installed(plugin: &str, version: &str, leases: u64) -> InstalledPlugin {
    InstalledPlugin {
        plugin_id: plugin.to_owned(),
        plugin_version: version.to_owned(),
        active_session_leases: leases,
    }
}

fn hawk() -> MachineTarget {
    MachineTarget {
        id: "hawk".to_owned(),
        platform: "linux".to_owned(),
        architecture: "x86_64".to_owned(),
        connected: true,
    }
}

#[test]
fn versions_compare_ordinally_and_survive_nonsense() {
    assert_eq!(compare_versions("3.1.9", "3.1.10"), Ordering::Less);
    assert_eq!(compare_versions("3.2.0", "3.1.99"), Ordering::Greater);
    assert_eq!(compare_versions("1.2.0", "1.2"), Ordering::Equal);
    assert_eq!(compare_versions("nightly", "0"), Ordering::Equal);
}

#[test]
fn only_a_ready_release_for_this_platform_is_a_target() {
    let mut elsewhere = release("codex", "3.2.0", "ready");
    elsewhere.supported_platforms = vec![SupportedPlatform {
        os: "macos".to_owned(),
        architecture: "aarch64".to_owned(),
    }];
    let mut undeclared = release("zed", "2.0.0", "ready");
    undeclared.supported_platforms.clear();
    let releases = vec![
        release("codex", "3.1.0", "ready"),
        release("codex", "3.1.22", "ready"),
        release("codex", "3.3.0", "pending"),
        elsewhere,
        undeclared,
    ];
    let latest = latest_ready(&releases, "linux", "x86_64");
    assert_eq!(latest["codex"].plugin_version, "3.1.22");
    assert!(
        !latest.contains_key("zed"),
        "an undeclared platform is not evidence"
    );
}

#[test]
fn an_upgrade_resolves_its_digest_from_the_catalog() {
    let releases = vec![release("codex", "3.1.22", "ready")];
    let plan = plan_machine(&hawk(), &releases, &[installed("codex", "3.1.3", 0)], &[]);
    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].to, "3.1.22");
    assert_eq!(plan.steps[0].digest, "sha256:3.1.22");
    assert_eq!(plan.steps[0].operation_id, "hawk-codex-3-1-22-converge");
    assert!(crate::plugin_operation::installation::valid_operation_id(
        &plan.steps[0].operation_id
    ));
}

#[test]
fn a_converged_machine_plans_nothing() {
    let releases = vec![release("codex", "3.1.22", "ready")];
    let plan = plan_machine(&hawk(), &releases, &[installed("codex", "3.1.22", 0)], &[]);
    assert!(plan.steps.is_empty());
    assert!(plan.skipped.is_empty());
}

#[test]
fn a_leased_plugin_is_reported_rather_than_recycled() {
    let releases = vec![release("codex", "3.1.22", "ready")];
    let plan = plan_machine(&hawk(), &releases, &[installed("codex", "3.1.3", 2)], &[]);
    assert!(plan.steps.is_empty());
    assert_eq!(plan.skipped[0].reason, "holds 2 active session lease(s)");
}

#[test]
fn convergence_never_downgrades() {
    let releases = vec![release("codex", "3.1.3", "ready")];
    let plan = plan_machine(&hawk(), &releases, &[installed("codex", "3.1.22", 0)], &[]);
    assert!(plan.steps.is_empty());
    assert!(plan.skipped[0].reason.contains("ahead of Catalog"));
}

#[test]
fn a_plugin_without_a_catalog_release_is_named_not_silently_ignored() {
    let plan = plan_machine(&hawk(), &[], &[installed("victoria", "1.1.0", 0)], &[]);
    assert!(plan.steps.is_empty());
    assert_eq!(plan.skipped[0].plugin, "victoria");
    assert!(plan.skipped[0].reason.contains("linux/x86_64"));
}

#[test]
fn convergence_installs_nothing_the_machine_does_not_already_run() {
    // A Plugin the Machine never installed is not "behind"; installing it is a
    // separate decision with its own confirmation.
    let releases = vec![release("gemini", "3.1.19", "ready")];
    let plan = plan_machine(&hawk(), &releases, &[installed("codex", "3.1.22", 0)], &[]);
    assert!(plan.steps.is_empty());
    assert_eq!(plan.skipped.len(), 1);
    assert_eq!(plan.skipped[0].plugin, "codex");
}

#[test]
fn a_plugin_filter_bounds_the_run() {
    let releases = vec![
        release("codex", "3.1.22", "ready"),
        release("gemini", "3.1.19", "ready"),
    ];
    let installed = vec![
        installed("codex", "3.1.3", 0),
        installed("gemini", "3.1.0", 0),
    ];
    let plan = plan_machine(&hawk(), &releases, &installed, &["codex".to_owned()]);
    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].plugin, "codex");
    assert!(plan.skipped.is_empty());
}

#[test]
fn the_requested_order_is_the_rollout_order() {
    let falcon = MachineTarget {
        id: "falcon".to_owned(),
        platform: "linux".to_owned(),
        architecture: "x86_64".to_owned(),
        connected: true,
    };
    let offline = MachineTarget {
        id: "macbook-air".to_owned(),
        platform: "macos".to_owned(),
        architecture: "aarch64".to_owned(),
        connected: false,
    };
    let registry = vec![falcon.clone(), hawk(), offline];
    let (ordered, skipped) = rollout_order(
        &registry,
        &[
            "hawk".to_owned(),
            "falcon".to_owned(),
            "macbook-air".to_owned(),
            "sparrow".to_owned(),
        ],
    );
    assert_eq!(
        ordered
            .iter()
            .map(|machine| machine.id.as_str())
            .collect::<Vec<_>>(),
        vec!["hawk", "falcon"],
        "the canary is whichever Machine was asked for first"
    );
    assert_eq!(skipped.len(), 2);
    assert!(
        skipped
            .iter()
            .any(|entry| entry.reason.contains("not connected"))
    );
    assert!(
        skipped
            .iter()
            .any(|entry| entry.reason.contains("no such registered Machine"))
    );
}

#[test]
fn without_a_request_every_connected_machine_converges() {
    let offline = MachineTarget {
        id: "macbook-air".to_owned(),
        platform: "macos".to_owned(),
        architecture: "aarch64".to_owned(),
        connected: false,
    };
    let (ordered, skipped) = rollout_order(&[hawk(), offline], &[]);
    assert_eq!(ordered.len(), 1);
    assert_eq!(skipped.len(), 1);
}

#[test]
fn an_operation_identity_is_stable_and_accepted() {
    let id = operation_id("macbook-air", "claude-code", "3.1.28");
    assert_eq!(id, "macbook-air-claude-code-3-1-28-converge");
    assert_eq!(id, operation_id("macbook-air", "claude-code", "3.1.28"));
    assert!(crate::plugin_operation::installation::valid_operation_id(
        &id
    ));
    // A hostile identifier cannot smuggle characters the Controller refuses.
    assert!(crate::plugin_operation::installation::valid_operation_id(
        &operation_id("m/../etc", "a b", "1.0")
    ));
}
