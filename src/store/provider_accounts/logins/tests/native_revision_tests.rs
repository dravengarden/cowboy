use super::*;

pub(super) async fn unrelated_changes(store: &Store, completion: &NativeCompletion) -> Registry {
    let original = store.provider_accounts("login-owner").await.unwrap();
    let MutationResult::Applied(created) = store
        .mutate_provider_accounts(
            "login-owner",
            "second-account",
            &Mutation {
                expected_revision: original.revision,
                change: Change::Create {
                    provider_id: "claude-code".into(),
                    label: "Second Max".into(),
                },
            },
        )
        .await
        .unwrap()
    else {
        panic!("create second account");
    };
    let mut registry = created.registry;
    let mut identity = completion.profile.identity.clone();
    identity.subject = "second-person".into();
    let changes = [
        Change::BindIdentity {
            id: created.account_id.clone(),
            provider_id: "claude-code".into(),
            identity,
        },
        Change::Rename {
            id: completion.profile.account_id.clone(),
            label: "Renamed first".into(),
        },
        Change::SetDefault {
            provider_id: "claude-code".into(),
            id: completion.profile.account_id.clone(),
        },
        Change::SetDefault {
            provider_id: "claude-code".into(),
            id: crate::provider_account::ProviderAccountId::default(),
        },
        Change::SetEnabled {
            id: created.account_id,
            enabled: false,
        },
    ];
    for (index, change) in changes.into_iter().enumerate() {
        // The first account remains admitted even after creating another one.
        store
            .with_native_login_binding("login-owner", "bind", completion, || Ok(()))
            .await
            .unwrap();
        let MutationResult::Applied(receipt) = store
            .mutate_provider_accounts(
                "login-owner",
                &format!("unrelated-{index}"),
                &Mutation {
                    expected_revision: registry.revision,
                    change,
                },
            )
            .await
            .unwrap()
        else {
            panic!("apply independent change");
        };
        registry = receipt.registry;
        store
            .with_native_login_binding("login-owner", "bind", completion, || Ok(()))
            .await
            .unwrap();
        assert_eq!(
            store
                .bind_native_login_identity(
                    "login-owner",
                    "bind",
                    completion,
                    original.revision - 1
                )
                .await
                .unwrap(),
            registry
        );
    }
    registry
}

#[test]
fn legacy_bindings_stay_conservative_and_revision_bounds_fail_closed() {
    let mut account: crate::store::provider_accounts::Account = serde_json::from_value(serde_json::json!({
        "id":"00000000-0000-0000-0000-000000000001", "provider_id":"claude-code", "label":"legacy", "enabled":true
    })).unwrap();
    assert!(account.admits_binding(2, 2));
    assert!(!account.admits_binding(2, 3));
    account.admission_revision = Some(2);
    assert!(account.admits_binding(2, 3));
    assert!(!account.admits_binding(1, 3));
    assert!(!account.admits_binding(4, 3));
    account.admission_revision = Some(-1);
    assert!(!account.admits_binding(2, 3));
}
