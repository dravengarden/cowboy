use super::*;
use serde_json::json;

fn set(hub: &Hub, role: AdminRole) {
    hub.set_setting(PERMISSIONS_SETTING.into(), json!({"default_role":role}));
}

#[test]
fn role_aba_and_promotion_end_the_original_observation_without_polling() {
    let hub = Hub::new();
    set(&hub, AdminRole::Owner);
    let original = hub.observe_product_permissions("User").unwrap();
    let shared = original.clone();
    let concurrent = hub.observe_product_permissions(" user ").unwrap();
    assert!(Arc::ptr_eq(&original.state, &concurrent.state));
    set(&hub, AdminRole::Viewer);
    set(&hub, AdminRole::Owner);
    assert!(!original.current(&hub, "user"));
    assert!(!shared.current(&hub, "user"));
    assert!(!concurrent.current(&hub, "user"));
    let fresh = hub.observe_product_permissions("user").unwrap();
    assert!(fresh.current(&hub, "USER"));
    assert!(!Arc::ptr_eq(&fresh.state, &original.state));
    set(&hub, AdminRole::Viewer);
    let viewer = hub.observe_product_permissions("user").unwrap();
    set(&hub, AdminRole::Operator);
    assert!(
        !viewer.current(&hub, "user"),
        "a queued read cannot gain effects"
    );
}

#[test]
fn unchanged_effective_roles_survive_unrelated_settings_and_other_users() {
    let hub = Hub::new();
    set(&hub, AdminRole::Operator);
    let original = hub.observe_product_permissions("user").unwrap();
    hub.set_setting("unrelated".into(), json!("changed"));
    hub.set_setting(
        PERMISSIONS_SETTING.into(),
        json!({"default_role":"viewer","grants":[
            {"account":"other","role":"owner"},
            {"account":"user","role":"operator"}
        ]}),
    );
    assert!(original.current(&hub, "user"));
    let next = hub.observe_product_permissions("user").unwrap();
    assert!(Arc::ptr_eq(&original.state, &next.state));
    // Removing a redundant grant changes representation, not permissions.
    set(&hub, AdminRole::Operator);
    assert!(original.current(&hub, "user"));
}

#[test]
fn all_settings_mutation_paths_end_affected_observations_before_unlock() {
    let hub = Hub::new();
    set(&hub, AdminRole::Operator);
    let original = hub.observe_product_permissions("user").unwrap();
    hub.load_settings(vec![(
        PERMISSIONS_SETTING.into(),
        json!({"default_role":"viewer"}),
    )]);
    assert!(!original.current(&hub, "user"));
    set(&hub, AdminRole::Operator);
    let next = hub.observe_product_permissions("user").unwrap();
    hub.with_settings_mut(|settings| {
        settings.remove(PERMISSIONS_SETTING);
    });
    assert!(!next.current(&hub, "user"));
    set(&hub, AdminRole::Operator);
    let next = hub.observe_product_permissions("user").unwrap();
    hub.with_settings_mut(|settings| {
        Hub::commit_setting_locked(settings, PERMISSIONS_SETTING.into(), json!("malformed"));
    });
    assert!(!next.current(&hub, "user"));
}

#[test]
fn mutation_unwind_does_not_leave_stale_observations_alive() {
    let hub = Hub::new();
    set(&hub, AdminRole::Operator);
    let original = hub.observe_product_permissions("user").unwrap();
    let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        hub.with_settings_mut(|settings| {
            settings.remove(PERMISSIONS_SETTING);
            panic!("test-only interrupted mutation");
        });
    }));
    assert!(failed.is_err());
    set(&hub, AdminRole::Operator);
    assert!(!original.current(&hub, "user"));
}

#[test]
fn distinct_subjects_are_bounded_without_evicting_live_observations() {
    let hub = Hub::new();
    let mut retained: Vec<_> = (0..MAX_OBSERVATIONS)
        .map(|i| {
            hub.observe_product_permissions(&format!("user-{i}"))
                .unwrap()
        })
        .collect();
    for _ in 0..MAX_OBSERVATIONS {
        assert!(hub.observe_product_permissions("user-0").is_some());
    }
    assert!(hub.observe_product_permissions("overflow").is_none());
    assert!(retained[0].current(&hub, "user-0"));
    retained.pop();
    assert!(hub.observe_product_permissions("fresh").is_some());
    assert!(retained[0].current(&hub, "user-0"));
    assert!(hub.inner.product_permissions.lock().current.len() <= MAX_OBSERVATIONS);
    assert!(hub.inner.product_permissions.lock().retained.len() <= MAX_OBSERVATIONS);
}

#[test]
fn ended_generations_keep_their_budget_until_their_actual_holders_drop() {
    let hub = Hub::new();
    let mut retained = Vec::new();
    for i in 0..MAX_OBSERVATIONS {
        set(
            &hub,
            if i % 2 == 0 {
                AdminRole::Owner
            } else {
                AdminRole::Viewer
            },
        );
        retained.push(hub.observe_product_permissions("user").unwrap());
    }
    set(&hub, AdminRole::Operator);
    assert!(retained.iter().all(|scope| !scope.current(&hub, "user")));
    assert!(hub.observe_product_permissions("user").is_none());
    assert!(hub.observe_product_permissions("other").is_none());
    retained.pop();
    assert!(
        hub.observe_product_permissions("user")
            .unwrap()
            .current(&hub, "user")
    );
    assert!(retained.iter().all(|scope| !scope.current(&hub, "user")));
}

#[test]
fn observations_are_bound_to_one_core_and_bounded_account_identity() {
    let hub = Hub::new();
    let original = hub.observe_product_permissions("user").unwrap();
    assert!(!original.current(&Hub::new(), "user"));
    assert!(!original.current(&hub, "other"));
    assert!(hub.observe_product_permissions("").is_none());
    assert!(hub.observe_product_permissions(" \t").is_none());
    assert!(
        hub.observe_product_permissions(&"x".repeat(MAX_ACCOUNT_BYTES + 1))
            .is_none()
    );
    assert!(
        hub.observe_product_permissions(&"x".repeat(MAX_ACCOUNT_BYTES))
            .is_some()
    );
}
