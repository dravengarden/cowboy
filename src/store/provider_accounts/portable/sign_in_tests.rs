//! A subscription sign-in binds the account identity under the owner lock.
use super::*;
use crate::store::provider_accounts::{Change, Mutation, MutationResult};

const OWNER: &str = "0123456789abcdef0123456789abcdef";

async fn store(root: &std::path::Path) -> Store {
    let url = format!("sqlite://{}", root.join("db.sqlite3").display());
    let store = Store::connect(&url, root.join("artifacts")).await.unwrap();
    store.migrate().await.unwrap();
    store
        .insert_user(&crate::store::ProductUser {
            id: OWNER.into(),
            username: "owner".into(),
            password_algo: "fixture".into(),
            password_hash: "fixture".into(),
            created_at_ms: 0,
            updated_at_ms: 0,
            disabled_at_ms: None,
        })
        .await
        .unwrap();
    let key = {
        use base64::Engine as _;
        let public = x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from([8; 32]));
        base64::engine::general_purpose::STANDARD.encode(public.as_bytes())
    };
    let token = store
        .create_machine_enrollment("ovh", "ovh", 60)
        .await
        .unwrap();
    store
        .consume_machine_enrollment(&token, &format!("ssh-ed25519 {key}"), &key)
        .await
        .unwrap();
    store
}

async fn create(store: &Store, revision: i64, label: &str) -> ProviderAccountId {
    let input = Mutation {
        expected_revision: revision,
        change: Change::Create {
            provider_id: "codex".into(),
            label: label.into(),
        },
    };
    match store
        .mutate_provider_accounts(OWNER, label, &input)
        .await
        .unwrap()
    {
        MutationResult::Applied(receipt) => receipt.account_id,
        other => panic!("{other:?}"),
    }
}

fn identity(subject: &str) -> crate::provider_account::NativeIdentity {
    crate::provider_account::NativeIdentity {
        authority: "chatgpt".into(),
        subject: subject.into(),
        tenant: Some("org".into()),
    }
}

async fn sign_in(
    store: &Store,
    account: &ProviderAccountId,
    who: &crate::provider_account::NativeIdentity,
) -> Result<bool> {
    let mut ran = false;
    store
        .with_portable_account(
            OWNER,
            account,
            Use::SignIn(who),
            |_| true,
            || {
                ran = true;
                async { Ok(()) }
            },
        )
        .await?;
    Ok(ran)
}

#[tokio::test]
async fn the_first_sign_in_binds_the_identity_and_a_different_one_is_refused() {
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path()).await;
    let lark = create(&store, 0, "Lark").await;
    let pika = create(&store, 1, "Pika").await;
    assert!(sign_in(&store, &lark, &identity("lark")).await.unwrap());
    let registry = store.provider_accounts(OWNER).await.unwrap();
    let account = registry.accounts.iter().find(|a| a.id == lark).unwrap();
    assert!(account.portable_credentials && account.identity_fingerprint.is_some());
    assert_eq!(registry.revision, 3);
    // Signing in again as the same identity changes nothing visible.
    assert!(sign_in(&store, &lark, &identity("lark")).await.unwrap());
    assert_eq!(store.provider_accounts(OWNER).await.unwrap().revision, 3);
    // Another identity never enters this account, nor this one another card.
    let error = sign_in(&store, &lark, &identity("fox")).await.unwrap_err();
    assert_eq!(error.to_string(), "native_identity_mismatch");
    let error = sign_in(&store, &pika, &identity("lark")).await.unwrap_err();
    assert_eq!(error.to_string(), "native_identity_already_bound");
    assert_eq!(
        store
            .provider_account_owner(&lark)
            .await
            .unwrap()
            .as_deref(),
        Some(OWNER)
    );
    let unknown: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
    assert_eq!(store.provider_account_owner(&unknown).await.unwrap(), None);
}

#[tokio::test]
async fn vault_generations_start_above_native_ones() {
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path()).await;
    let lark = create(&store, 0, "Pika").await;
    assert_eq!(
        store.native_generation_floor(OWNER, &lark).await.unwrap(),
        0
    );
    for (index, machine) in ["ovh", "ovh", "hawk"].into_iter().enumerate() {
        store
            .claim_native_provider_login(
                OWNER,
                &format!("login-{index}"),
                &crate::store::provider_accounts::logins::Request {
                    account_id: lark.clone(),
                    machine_id: machine.into(),
                    provider_version: "1".into(),
                    generation_digest: "digest".into(),
                    method: "chatgpt-subscription".into(),
                },
            )
            .await
            .unwrap();
    }
    assert_eq!(
        store.native_generation_floor(OWNER, &lark).await.unwrap(),
        2
    );
    // A new Codex sign-in is a vault candidate: it reserves no native profile.
    let claim = store
        .claim_provider_login(
            OWNER,
            "login-vault",
            &crate::store::provider_accounts::logins::Request {
                account_id: lark.clone(),
                machine_id: "ovh".into(),
                provider_version: "1".into(),
                generation_digest: "digest".into(),
                method: "chatgpt-subscription".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(claim.receipt.native_generation, None);
}

#[tokio::test]
async fn claude_sign_ins_stay_native_per_machine() {
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path()).await;
    let input = Mutation {
        expected_revision: 0,
        change: Change::Create {
            provider_id: "claude-code".into(),
            label: "Lark".into(),
        },
    };
    let MutationResult::Applied(created) = store
        .mutate_provider_accounts(OWNER, "create", &input)
        .await
        .unwrap()
    else {
        panic!("create");
    };
    let claim = store
        .claim_provider_login(
            OWNER,
            "login-claude",
            &crate::store::provider_accounts::logins::Request {
                account_id: created.account_id,
                machine_id: "ovh".into(),
                provider_version: "1".into(),
                generation_digest: "digest".into(),
                method: "claude-subscription".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(claim.receipt.native_generation, Some(1));
}
