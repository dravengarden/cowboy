use super::*;
use crate::provider_service::grants::revocation::Revocation;

async fn commit(store: &Store, grant: &ProviderAuthService, intent: &Receipt) -> Receipt {
    let package = package("grok");
    let proof = grant
        .commit_grant(&package, &bundle(&package), intent)
        .unwrap();
    let Outcome::Applied(receipt) = store
        .finish_verified_provider_grant("owner", intent, &proof)
        .await
        .unwrap()
    else {
        panic!("commit")
    };
    receipt
}

#[tokio::test]
async fn committed_grant_revocation_survives_restart_without_another_write() {
    let (dir, store, service, grant, intent) = promotion::fixture().await;
    let receipt = commit(&store, &grant, &intent).await;
    let recipient = grant.recipient_key.clone().unwrap();
    assert!(grant.seal_revoked_grant(&receipt, &recipient).is_err());
    assert_eq!(
        grant.providers.read().get("grok").unwrap().auth_generation,
        1
    );
    assert!(grant.revoke_committed_grant(&intent).is_err());
    assert!(service.revoke_committed_grant(&receipt).is_err());
    let registry = store.provider_accounts("owner").await.unwrap();
    store
        .mutate_provider_accounts(
            "owner",
            "revoke",
            &Mutation {
                expected_revision: registry.revision,
                change: Change::BeginRevocation {
                    id: receipt.account_id.clone(),
                },
            },
        )
        .await
        .unwrap();
    assert_eq!(
        store
            .with_provider_grant_revocation("owner", "revoke", "login", |receipt| {
                service
                    .existing_revocation_grant(receipt)?
                    .revoke_committed_grant(receipt)
            })
            .await
            .unwrap(),
        Revocation::Applied { generation: 2 }
    );
    let stored = grant.providers.read().get("grok").unwrap().clone();
    let wipe = grant.seal_revoked_grant(&receipt, &recipient).unwrap();
    assert_eq!(wipe.action, ProviderAuthAction::Wipe);
    assert_eq!(wipe.account_id, receipt.account_id);
    assert_eq!(wipe.auth_generation, 2);
    assert!(
        grant
            .seal_revoked_grant(&receipt, "other-recipient")
            .is_err()
    );
    assert!(grant.open_vault(&stored).unwrap().is_empty());
    assert_eq!(
        grant.revoke_committed_grant(&receipt).unwrap(),
        Revocation::AlreadySignedOut { generation: 2 }
    );
    assert_eq!(
        grant.providers.read().get("grok").unwrap().ciphertext,
        stored.ciphertext
    );
    let key = grant.recipient_key.clone().unwrap();
    drop(grant);
    drop(service);
    let service = ProviderAuthService::open(dir.path()).unwrap();
    let grant = service
        .existing_machine_grant(&receipt.account_id, &key)
        .unwrap();
    assert_eq!(
        grant.revoke_committed_grant(&receipt).unwrap(),
        Revocation::AlreadySignedOut { generation: 2 }
    );
    assert_eq!(
        grant.providers.read().get("grok").unwrap().ciphertext,
        stored.ciphertext
    );
    // Local logout does not rewrite the DB's historical committed grant.
    assert_eq!(
        store
            .provider_grant_receipt("owner", "login")
            .await
            .unwrap(),
        Some(receipt.clone())
    );
    grant.providers.write().get_mut("grok").unwrap().ciphertext = "damaged".into();
    assert!(grant.revoke_committed_grant(&receipt).is_err());
    assert!(grant.seal_revoked_grant(&receipt, &recipient).is_err());
}

#[tokio::test]
async fn stale_revocation_cannot_erase_a_new_login_or_another_recipient() {
    let (_dir, store, service, grant, intent) = promotion::fixture().await;
    let receipt = commit(&store, &grant, &intent).await;
    let mut wrong = receipt.clone();
    wrong.recipient_fingerprint = "wrong-recipient".into();
    assert!(grant.revoke_committed_grant(&wrong).is_err());
    let package = package("grok");
    grant
        .commit(&package, &bundle(&package), None, Some(1))
        .unwrap();
    let ciphertext = grant
        .providers
        .read()
        .get("grok")
        .unwrap()
        .ciphertext
        .clone();
    assert_eq!(
        grant.revoke_committed_grant(&receipt).unwrap(),
        Revocation::Superseded
    );
    assert_eq!(
        grant.providers.read().get("grok").unwrap().ciphertext,
        ciphertext
    );
    let other = service
        .account(&receipt.account_id)
        .unwrap()
        .machine_grant(
            &base64::engine::general_purpose::STANDARD
                .encode(PublicKey::from(&StaticSecret::from([8; 32])).as_bytes()),
        )
        .unwrap();
    assert!(other.revoke_committed_grant(&receipt).is_err());
}
