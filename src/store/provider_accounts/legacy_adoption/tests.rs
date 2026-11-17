use super::super::{Change, Mutation};
use super::*;
use crate::provider_account::ProviderAccountId;

const OWNER: &str = "owner";

async fn sqlite_store(dir: &tempfile::TempDir) -> Store {
    let store = Store::connect(
        &format!(
            "sqlite://{}?mode=rwc",
            dir.path().join("adoption.db").display()
        ),
        dir.path().join("artifacts"),
    )
    .await
    .unwrap();
    store.migrate().await.unwrap();
    store
}

async fn postgres_store(dir: &tempfile::TempDir) -> Store {
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL").expect("isolated PostgreSQL gate required");
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    store
}

async fn change(store: &Store, owner: &str, operation: &str, change: Change) -> Receipt {
    let revision = store.provider_accounts(owner).await.unwrap().revision;
    let input = Mutation {
        expected_revision: revision,
        change,
    };
    let result = if input.change.is_browser_share() {
        store.mutate_browser_shares(owner, operation, &input).await
    } else {
        store
            .mutate_provider_accounts(owner, operation, &input)
            .await
    };
    match result.unwrap() {
        MutationResult::Applied(receipt) => receipt,
        other => panic!("{operation}: {other:?}"),
    }
}

fn pika() -> Adoption<'static> {
    Adoption {
        card: "openai",
        provider_id: "codex",
        label: "OpenAI · Pika",
    }
}

async fn adopted(store: &Store, owner: &str, operation: &str, adoption: &Adoption<'_>) -> Receipt {
    match store
        .adopt_legacy_card(owner, operation, adoption)
        .await
        .unwrap()
    {
        MutationResult::Applied(receipt) => receipt,
        other => panic!("{operation}: {other:?}"),
    }
}

async fn refused(
    store: &Store,
    owner: &str,
    operation: &str,
    adoption: &Adoption<'_>,
) -> &'static str {
    match store
        .adopt_legacy_card(owner, operation, adoption)
        .await
        .unwrap()
    {
        MutationResult::Invalid(code) => code,
        other => panic!("{operation}: {other:?}"),
    }
}

#[allow(clippy::too_many_lines)] // One scenario: create, replay, resume, complete and refuse.
async fn contract(store: &Store) {
    let key = legacy_browsers::account(OWNER, "openai");
    // The card's browser is enabled and in a share with the Fox card.
    change(
        store,
        OWNER,
        "browser",
        Change::SetLegacyBrowserEnabled {
            card: "openai".into(),
            enabled: true,
        },
    )
    .await;
    let share = change(
        store,
        OWNER,
        "share",
        Change::CreateBrowserShare {
            label: "Google".into(),
        },
    )
    .await
    .browser_share_id
    .unwrap();
    for card in ["openai", "xai"] {
        change(
            store,
            OWNER,
            &format!("join-{card}"),
            Change::SetBrowserShare {
                member: Member::card(card),
                share_id: Some(share.clone()),
            },
        )
        .await;
    }
    let before = store
        .browser_shares(OWNER)
        .await
        .unwrap()
        .share(&share)
        .unwrap()
        .revision;

    let receipt = adopted(store, OWNER, "adopt-create", &pika()).await;
    assert_eq!(receipt.account_id, key);
    let account = receipt
        .registry
        .accounts
        .iter()
        .find(|a| a.id == key)
        .unwrap();
    assert_eq!(account.provider_id, "codex");
    assert_eq!(account.label, "OpenAI · Pika");
    assert!(account.available() && account.browser_enabled);
    // Not the default before its credential moved.
    assert!(!receipt.registry.defaults.contains_key("codex"));
    // The card left the share and the account took its place and binding.
    let shares = store.browser_shares(OWNER).await.unwrap();
    assert!(shares.membership(&Member::card("openai")).is_none());
    let row = shares.membership(&Member::account(key.clone())).unwrap();
    assert_eq!(
        (row.share_id.as_str(), row.family.as_str()),
        (share.as_str(), "openai")
    );
    assert_eq!(
        shares.binding(&share, "openai").unwrap().member,
        Member::account(key.clone())
    );
    assert!(shares.membership(&Member::card("xai")).is_some());
    assert!(shares.share(&share).unwrap().revision > before);
    assert!(store.legacy_browser_cards(OWNER).await.unwrap().is_empty());

    // Idempotent replay; reuse with another input is refused.
    assert_eq!(
        adopted(store, OWNER, "adopt-create", &pika()).await,
        receipt
    );
    let other = Adoption {
        label: "Other",
        ..pika()
    };
    assert_eq!(
        refused(store, OWNER, "adopt-create", &other).await,
        "operation_reused"
    );
    // A second operation resumes the unfinished adoption without changes.
    let resumed = adopted(store, OWNER, "adopt-retry", &pika()).await;
    assert_eq!(resumed.registry, receipt.registry);
    // The id is taken by this card's account: never for another Provider.
    let foreign = Adoption {
        provider_id: "codex-deepseek",
        ..pika()
    };
    assert_eq!(
        refused(store, OWNER, "adopt-foreign", &foreign).await,
        "account_id_in_use"
    );

    // Completion switches the default; replay is idempotent.
    let providers = vec!["codex".to_owned()];
    let MutationResult::Applied(done) = store
        .complete_legacy_adoption(OWNER, "adopt-final", &key, &providers)
        .await
        .unwrap()
    else {
        panic!("completion refused")
    };
    assert_eq!(done.registry.defaults["codex"], key);
    assert_eq!(
        store
            .complete_legacy_adoption(OWNER, "adopt-final", &key, &providers)
            .await
            .unwrap(),
        MutationResult::Applied(done.clone())
    );
    assert_eq!(done.registry.resolve("codex", None).unwrap(), key);
    // A later adoption of the same card resumes the same account.
    assert_eq!(
        adopted(store, OWNER, "adopt-again", &pika())
            .await
            .account_id,
        key
    );

    // Refusals: wrong family, a non-card, another owner's view.
    let wrong = Adoption {
        card: "anthropic",
        provider_id: "codex",
        label: "X",
    };
    assert_eq!(
        refused(store, OWNER, "adopt-wrong", &wrong).await,
        "invalid_provider"
    );
    let unknown = Adoption {
        card: "future",
        provider_id: "codex",
        label: "X",
    };
    assert_eq!(
        refused(store, OWNER, "adopt-unknown", &unknown).await,
        "invalid_provider"
    );
    let other_owner = adopted(store, "other", "adopt-create", &pika()).await;
    assert_ne!(other_owner.account_id, key);
    assert_eq!(
        other_owner.account_id,
        legacy_browsers::account("other", "openai")
    );
    let missing: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
    assert_eq!(
        store
            .complete_legacy_adoption(OWNER, "adopt-missing", &missing, &providers)
            .await
            .unwrap(),
        MutationResult::Invalid("account_unavailable")
    );

    // A portable scope: one account is every member's default.
    let mole = Adoption {
        card: "deepseek",
        provider_id: "claude-deepseek",
        label: "DeepSeek · Mole",
    };
    let mole_key = adopted(store, OWNER, "mole-create", &mole).await.account_id;
    let scope = ["claude-deepseek", "codex-deepseek", "deepseek-harness"].map(str::to_owned);
    let MutationResult::Applied(done) = store
        .complete_legacy_adoption(OWNER, "mole-final", &mole_key, &scope)
        .await
        .unwrap()
    else {
        panic!("completion refused")
    };
    let members = scope.to_vec();
    for provider in &scope {
        assert_eq!(done.registry.defaults[provider], mole_key);
        assert_eq!(
            done.registry
                .resolve_in_scope(provider, &members, None)
                .unwrap(),
            mole_key
        );
    }
}

#[tokio::test]
async fn adoption_reuses_the_card_identity_and_moves_browser_share_and_default() {
    let dir = tempfile::tempdir().unwrap();
    contract(&sqlite_store(&dir).await).await;
}

#[tokio::test]
#[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
async fn postgres_legacy_adoption_reuses_the_card_identity() {
    let dir = tempfile::tempdir().unwrap();
    contract(&postgres_store(&dir).await).await;
}
