use super::*;
use crate::store::provider_accounts::Account;

const OWNER: &str = "owner";

fn families() -> BTreeMap<String, String> {
    [
        ("claude-code", "anthropic"),
        ("codex", "openai"),
        ("grok", "xai"),
        ("claude-deepseek", "deepseek"),
    ]
    .into_iter()
    .map(|(provider, family)| (provider.to_owned(), family.to_owned()))
    .collect()
}

fn named(provider: &str, label: &str) -> Account {
    Account {
        id: uuid::Uuid::new_v4().to_string().try_into().unwrap(),
        provider_id: provider.into(),
        label: label.into(),
        enabled: true,
        revoking: false,
        identity_fingerprint: None,
        admission_revision: None,
        browser_enabled: true,
    }
}

struct Fixture {
    registry: Registry,
    families: BTreeMap<String, String>,
    placed: BTreeSet<ProviderAccountId>,
}

impl Fixture {
    fn context(&self, new_share_id: &str) -> ShareContext<'_> {
        ShareContext {
            owner: OWNER,
            registry: &self.registry,
            families: &self.families,
            placed: &self.placed,
            now_ms: 7,
            new_share_id: new_share_id.to_owned(),
        }
    }

    fn create(&self, shares: &mut Shares, label: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        let applied = apply(
            shares,
            &Change::CreateBrowserShare {
                label: label.into(),
            },
            &self.context(&id),
        )
        .unwrap();
        assert_eq!(applied.share_id.as_deref(), Some(id.as_str()));
        assert_eq!(applied.subject, share_key(OWNER, &id));
        id
    }

    fn set(
        &self,
        shares: &mut Shares,
        member: Member,
        share_id: Option<&str>,
    ) -> Result<Applied, &'static str> {
        apply(
            shares,
            &Change::SetBrowserShare {
                member,
                share_id: share_id.map(str::to_owned),
            },
            &self.context("unused"),
        )
    }
}

#[test]
fn a_share_holds_at_most_one_member_per_family() {
    let lark = named("claude-code", "Lark");
    let codex = named("codex", "Work");
    let fixture = Fixture {
        registry: Registry {
            revision: 3,
            accounts: vec![lark.clone(), codex.clone()],
            defaults: BTreeMap::new(),
        },
        families: families(),
        placed: BTreeSet::new(),
    };
    let mut shares = Shares::default();
    let google = fixture.create(&mut shares, "Google");
    // Lynx (anthropic), Pika (openai) and Fox (xai) share one profile.
    for card in ["anthropic", "openai", "xai"] {
        fixture
            .set(&mut shares, Member::card(card), Some(&google))
            .unwrap();
    }
    assert_eq!(shares.members_of(&google).len(), 3);
    assert!(families_distinct(&shares.members_of(&google)));
    // Lark is another Claude account: never linkable with Lynx.
    assert_eq!(
        fixture.set(&mut shares, Member::account(lark.id.clone()), Some(&google)),
        Err("browser_share_family_conflict")
    );
    // A second OpenAI member is refused too.
    assert_eq!(
        fixture.set(
            &mut shares,
            Member::account(codex.id.clone()),
            Some(&google)
        ),
        Err("browser_share_family_conflict")
    );
    assert_eq!(shares.members_of(&google).len(), 3);
    // Lark may have a share of its own, still without Lynx.
    let other = fixture.create(&mut shares, "Other");
    fixture
        .set(&mut shares, Member::account(lark.id.clone()), Some(&other))
        .unwrap();
    assert_eq!(
        fixture.set(&mut shares, Member::card("anthropic"), Some(&other)),
        Err("browser_share_family_conflict")
    );
    // Re-joining the same share is idempotent; moving leaves the old share.
    fixture
        .set(&mut shares, Member::card("openai"), Some(&google))
        .unwrap();
    fixture
        .set(&mut shares, Member::card("openai"), Some(&other))
        .unwrap();
    assert_eq!(shares.members_of(&google).len(), 2);
    assert_eq!(
        shares
            .membership(&Member::card("openai"))
            .map(|row| row.share_id.as_str()),
        Some(other.as_str())
    );
    // Leaving drops only that membership.
    fixture
        .set(&mut shares, Member::card("openai"), None)
        .unwrap();
    assert!(shares.membership(&Member::card("openai")).is_none());
}

#[test]
fn members_whose_family_is_unknown_or_revoking_are_refused() {
    let unknown = named("mystery", "Unknown");
    let mut revoking = named("codex", "Gone");
    revoking.revoking = true;
    let fixture = Fixture {
        registry: Registry {
            revision: 1,
            accounts: vec![unknown.clone(), revoking.clone()],
            defaults: BTreeMap::new(),
        },
        families: families(),
        placed: BTreeSet::new(),
    };
    let mut shares = Shares::default();
    let share = fixture.create(&mut shares, "Google");
    assert_eq!(
        fixture.set(&mut shares, Member::account(unknown.id), Some(&share)),
        Err("browser_share_family_unknown")
    );
    assert_eq!(
        fixture.set(&mut shares, Member::account(revoking.id), Some(&share)),
        Err("account_revoking")
    );
    let missing: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
    assert_eq!(
        fixture.set(&mut shares, Member::account(missing), Some(&share)),
        Err("account_not_found")
    );
    assert_eq!(
        fixture.set(
            &mut shares,
            Member::account(ProviderAccountId::default()),
            Some(&share)
        ),
        Err("invalid_argument")
    );
    assert_eq!(
        fixture.set(&mut shares, Member::card("anthropic"), Some("nope")),
        Err("browser_share_not_found")
    );
}

#[test]
fn share_keys_never_equal_account_or_card_keys() {
    let mut keys = BTreeSet::new();
    for owner in ["owner", "other"] {
        for card in ["anthropic", "deepseek", "openai", "xai"] {
            assert!(keys.insert(legacy_browsers::account(owner, card)));
        }
        for _ in 0..16 {
            let share = uuid::Uuid::new_v4().to_string();
            let key = share_key(owner, &share);
            assert_eq!(key, share_key(owner, &share));
            assert!(!key.is_default());
            assert!(keys.insert(key.clone()), "share key collides");
            // A share id itself is never its key.
            assert_ne!(key.as_str(), share);
            // The card and share domains differ even for equal inputs.
            assert_ne!(key, legacy_browsers::account(owner, &share));
        }
    }
    for _ in 0..64 {
        let registry: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        assert!(!keys.contains(&registry));
    }
    assert!(share_id_valid(&uuid::Uuid::new_v4().to_string()));
    assert!(!share_id_valid("Share"));
    assert_eq!(
        Member::decode(&Member::card("xai").encode()),
        Some(Member::card("xai"))
    );
    assert_eq!(Member::decode("account:default"), None);
    assert_eq!(Member::decode("card:Bad Card"), None);
}

#[test]
fn the_member_wire_form_is_exactly_account_id_or_card() {
    let account: Member = serde_json::from_value(
        serde_json::json!({"account_id": "11111111-2222-4333-8444-555555555555"}),
    )
    .unwrap();
    assert!(matches!(account, Member::Account(_)));
    let card: Member = serde_json::from_value(serde_json::json!({"card": "openai"})).unwrap();
    assert_eq!(card, Member::card("openai"));
    for invalid in [
        serde_json::json!({"card": "openai", "account_id": "11111111-2222-4333-8444-555555555555"}),
        serde_json::json!({"card": "openai", "family": "openai"}),
        serde_json::json!({}),
    ] {
        assert!(serde_json::from_value::<Member>(invalid).is_err());
    }
    let change: Change = serde_json::from_value(serde_json::json!({
        "type": "set_browser_share", "member": {"card": "xai"}, "share_id": null
    }))
    .unwrap();
    assert!(change.is_browser_share());
}

async fn sqlite_store(dir: &tempfile::TempDir) -> Store {
    let store = Store::connect(
        &format!(
            "sqlite://{}?mode=rwc",
            dir.path().join("shares.db").display()
        ),
        dir.path().join("artifacts"),
    )
    .await
    .unwrap();
    store.migrate().await.unwrap();
    store
}

async fn mutate(store: &Store, operation: &str, change: Change) -> MutationResult {
    let revision = store
        .provider_accounts(OWNER)
        .await
        .map_or(0, |registry| registry.revision);
    let input = Mutation {
        expected_revision: revision,
        change,
    };
    if input.change.is_browser_share() {
        store
            .mutate_browser_shares(OWNER, operation, &input, &families())
            .await
            .unwrap()
    } else {
        store
            .mutate_provider_accounts(OWNER, operation, &input)
            .await
            .unwrap()
    }
}

async fn applied(store: &Store, operation: &str, change: Change) -> Receipt {
    match mutate(store, operation, change).await {
        MutationResult::Applied(receipt) => receipt,
        other => panic!("{operation}: {other:?}"),
    }
}

async fn invalid(store: &Store, operation: &str, change: Change) -> &'static str {
    match mutate(store, operation, change).await {
        MutationResult::Invalid(code) => code,
        other => panic!("{operation}: {other:?}"),
    }
}

async fn named_account(store: &Store, operation: &str, provider: &str) -> ProviderAccountId {
    let id = applied(
        store,
        operation,
        Change::Create {
            provider_id: provider.into(),
            label: operation.into(),
        },
    )
    .await
    .account_id;
    applied(
        store,
        &format!("{operation}-browser"),
        Change::SetBrowserEnabled {
            id: id.clone(),
            enabled: true,
        },
    )
    .await;
    id
}

fn join(member: Member, share: &str) -> Change {
    Change::SetBrowserShare {
        member,
        share_id: Some(share.to_owned()),
    }
}

async fn postgres_store(dir: &tempfile::TempDir) -> Store {
    let url = std::env::var("COWBOY_TEST_POSTGRES_URL").expect("isolated PostgreSQL gate required");
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    store.migrate().await.unwrap();
    store
}

#[tokio::test]
async fn share_changes_use_the_registry_writer_and_its_family_rule() {
    let dir = tempfile::tempdir().unwrap();
    registry_writer_and_family_rule(&sqlite_store(&dir).await).await;
}

#[tokio::test]
#[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
async fn postgres_browser_shares_follow_the_registry_writer_and_revocation() {
    let dir = tempfile::tempdir().unwrap();
    let store = postgres_store(&dir).await;
    registry_writer_and_family_rule(&store).await;
    share_deletion(&store).await;
}

async fn registry_writer_and_family_rule(store: &Store) {
    let lark = named_account(store, "lark", "claude-code").await;
    let receipt = applied(
        store,
        "create-share",
        Change::CreateBrowserShare {
            label: " Google ".into(),
        },
    )
    .await;
    let share = receipt.browser_share_id.clone().unwrap();
    assert_eq!(receipt.account_id, share_key(OWNER, &share));
    // Replay returns the same receipt; another change under it is refused.
    let replay = |label: &str| Mutation {
        expected_revision: receipt.registry.revision - 1,
        change: Change::CreateBrowserShare {
            label: label.into(),
        },
    };
    assert_eq!(
        store
            .mutate_browser_shares(OWNER, "create-share", &replay(" Google "), &families())
            .await
            .unwrap(),
        MutationResult::Applied(receipt.clone())
    );
    assert_eq!(
        store
            .mutate_browser_shares(OWNER, "create-share", &replay("X"), &families())
            .await
            .unwrap(),
        MutationResult::Invalid("operation_reused")
    );
    assert_eq!(store.browser_shares(OWNER).await.unwrap().shares.len(), 1);
    for card in ["anthropic", "openai", "xai"] {
        applied(
            store,
            &format!("join-{card}"),
            join(Member::card(card), &share),
        )
        .await;
    }
    assert_eq!(
        invalid(
            store,
            "join-lark",
            join(Member::account(lark.clone()), &share)
        )
        .await,
        "browser_share_family_conflict"
    );
    // A stale revision is a conflict, like every account change.
    let stale = store
        .mutate_browser_shares(
            OWNER,
            "stale",
            &Mutation {
                expected_revision: 0,
                change: Change::RenameBrowserShare {
                    share_id: share.clone(),
                    label: "G".into(),
                },
            },
            &families(),
        )
        .await
        .unwrap();
    assert!(matches!(stale, MutationResult::Conflict(_)));
    let shares = store.browser_shares(OWNER).await.unwrap();
    assert_eq!(shares.shares[0].label, "Google");
    assert_eq!(shares.members_of(&share).len(), 3);
    assert!(
        store
            .browser_shares("other")
            .await
            .unwrap()
            .shares
            .is_empty()
    );
    // Share changes never reach the plain registry writer.
    assert_eq!(
        match store
            .mutate_provider_accounts(
                OWNER,
                "wrong-writer",
                &Mutation {
                    expected_revision: store.provider_accounts(OWNER).await.unwrap().revision,
                    change: Change::DeleteBrowserShare {
                        share_id: share.clone()
                    },
                },
            )
            .await
            .unwrap()
        {
            MutationResult::Invalid(code) => code,
            other => panic!("{other:?}"),
        },
        "invalid_change"
    );
}

#[tokio::test]
async fn the_family_rule_is_also_a_table_constraint() {
    let dir = tempfile::tempdir().unwrap();
    let store = sqlite_store(&dir).await;
    let StorageBackend::Sqlite(db) = &store.backend else {
        panic!("sqlite")
    };
    let insert = |member: &str, share: &str, family: &str| {
        sqlx::query("INSERT INTO provider_browser_share_members (owner_user_id,member,share_id,family) VALUES ($1,$2,$3,$4)")
            .bind(OWNER).bind(member.to_owned()).bind(share.to_owned()).bind(family.to_owned())
            .execute(&db.pool)
    };
    insert("card:anthropic", "s", "anthropic").await.unwrap();
    // Lark next to Lynx in one share: rejected by the unique key.
    assert!(
        insert(
            "account:11111111-2222-4333-8444-555555555555",
            "s",
            "anthropic"
        )
        .await
        .is_err()
    );
    // One member, one share.
    assert!(insert("card:anthropic", "t", "anthropic").await.is_err());
    insert("card:openai", "s", "openai").await.unwrap();
}

#[tokio::test]
async fn shared_placements_admit_only_current_enabled_members() {
    let dir = tempfile::tempdir().unwrap();
    let store = sqlite_store(&dir).await;
    let codex = named_account(&store, "codex", "codex").await;
    let share = applied(
        &store,
        "create",
        Change::CreateBrowserShare {
            label: "Google".into(),
        },
    )
    .await
    .browser_share_id
    .unwrap();
    let key = share_key(OWNER, &share);
    let lynx = Member::card("anthropic");
    let pika = Member::account(codex.clone());
    // Not a member yet.
    assert_eq!(
        store
            .admit_shared_browser_placement(OWNER, (&lynx, &share), "ovh", "b-1", 1)
            .await
            .unwrap(),
        Err(Refusal::ShareChanged)
    );
    applied(&store, "join-lynx", join(lynx.clone(), &share)).await;
    applied(&store, "join-pika", join(pika.clone(), &share)).await;
    // Lynx's own browser is off: Lynx cannot open even though it is a member.
    assert_eq!(
        store
            .admit_shared_browser_placement(OWNER, (&lynx, &share), "ovh", "b-1", 1)
            .await
            .unwrap(),
        Err(Refusal::BrowserNotEnabled)
    );
    // Pika's browser is enabled: the share's key is placed.
    store
        .admit_shared_browser_placement(OWNER, (&pika, &share), "ovh", "b-1", 1)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        store.browser_placements(OWNER, &key).await.unwrap().len(),
        1
    );
    assert!(
        store
            .browser_placements(OWNER, &codex)
            .await
            .unwrap()
            .is_empty()
    );
    // Disabling Pika's own browser withdraws only Pika's admission; the
    // share's placement (its profile) stays.
    applied(
        &store,
        "pika-off",
        Change::SetBrowserEnabled {
            id: codex.clone(),
            enabled: false,
        },
    )
    .await;
    assert_eq!(
        store
            .admit_shared_browser_placement(OWNER, (&pika, &share), "ovh", "b-1", 2)
            .await
            .unwrap(),
        Err(Refusal::BrowserNotEnabled)
    );
    assert_eq!(
        store.browser_placements(OWNER, &key).await.unwrap().len(),
        1
    );
    // A member that left is refused for the share.
    applied(
        &store,
        "lynx-leaves",
        Change::SetBrowserShare {
            member: lynx.clone(),
            share_id: None,
        },
    )
    .await;
    assert_eq!(
        store
            .admit_shared_browser_placement(OWNER, (&lynx, &share), "ovh", "b-1", 3)
            .await
            .unwrap(),
        Err(Refusal::ShareChanged)
    );
}

#[tokio::test]
async fn revoking_a_member_never_captures_the_share_and_drops_its_membership() {
    let dir = tempfile::tempdir().unwrap();
    let store = sqlite_store(&dir).await;
    let codex = named_account(&store, "codex", "codex").await;
    // The account's own browser was placed before it joined.
    store
        .admit_browser_placement(OWNER, (&codex, None), "ovh", "b-own", 1)
        .await
        .unwrap()
        .unwrap();
    let share = applied(
        &store,
        "create",
        Change::CreateBrowserShare {
            label: "Google".into(),
        },
    )
    .await
    .browser_share_id
    .unwrap();
    let key = share_key(OWNER, &share);
    let pika = Member::account(codex.clone());
    applied(&store, "join-pika", join(pika.clone(), &share)).await;
    applied(&store, "join-fox", join(Member::card("xai"), &share)).await;
    store
        .admit_shared_browser_placement(OWNER, (&pika, &share), "ovh", "b-share", 2)
        .await
        .unwrap()
        .unwrap();
    let receipt = applied(
        &store,
        "revoke",
        Change::BeginRevocation { id: codex.clone() },
    )
    .await;
    let plan = receipt.revocation.unwrap();
    // Only the account's own profile is revoked, never the share's.
    assert_eq!(plan.browsers.len(), 1);
    assert_eq!(plan.browsers[0].instance_id, "b-own");
    assert_eq!(receipt.account_id, codex);
    let shares = store.browser_shares(OWNER).await.unwrap();
    assert!(shares.membership(&pika).is_none());
    assert_eq!(shares.members_of(&share).len(), 1);
    assert_eq!(
        store.browser_placements(OWNER, &key).await.unwrap().len(),
        1
    );
}

#[tokio::test]
async fn deleting_a_share_revokes_exactly_its_own_placements() {
    let dir = tempfile::tempdir().unwrap();
    share_deletion(&sqlite_store(&dir).await).await;
}

#[allow(clippy::too_many_lines)] // One scenario: a share's whole deletion lifecycle.
async fn share_deletion(store: &Store) {
    applied(
        store,
        "lynx-on",
        Change::SetLegacyBrowserEnabled {
            card: "anthropic".into(),
            enabled: true,
        },
    )
    .await;
    let lynx = Member::card("anthropic");
    let own = lynx.own_key(OWNER);
    store
        .admit_browser_placement(OWNER, (&own, Some("anthropic")), "ovh", "b-own", 1)
        .await
        .unwrap()
        .unwrap();
    let create = |operation: &'static str| Change::CreateBrowserShare {
        label: operation.into(),
    };
    // A share that never opened disappears at once.
    let empty = applied(store, "empty", create("empty"))
        .await
        .browser_share_id
        .unwrap();
    applied(
        store,
        "empty-delete",
        Change::DeleteBrowserShare {
            share_id: empty.clone(),
        },
    )
    .await;
    assert!(
        store
            .browser_shares(OWNER)
            .await
            .unwrap()
            .share(&empty)
            .is_none()
    );
    let share = applied(store, "create", create("Google"))
        .await
        .browser_share_id
        .unwrap();
    let key = share_key(OWNER, &share);
    applied(store, "join", join(lynx.clone(), &share)).await;
    for machine in ["ovh", "hawk"] {
        store
            .admit_shared_browser_placement(OWNER, (&lynx, &share), machine, "b-share", 2)
            .await
            .unwrap()
            .unwrap();
    }
    let receipt = applied(
        store,
        "delete",
        Change::DeleteBrowserShare {
            share_id: share.clone(),
        },
    )
    .await;
    assert_eq!(receipt.account_id, key);
    let shares = store.browser_shares(OWNER).await.unwrap();
    assert!(shares.share(&share).unwrap().deleting_ms.is_some());
    assert!(shares.membership(&lynx).is_none());
    // A deleting share admits no member, no open and no rename.
    assert_eq!(
        invalid(store, "rejoin", join(lynx.clone(), &share)).await,
        "browser_share_not_found"
    );
    // Evidence from each Machine removes exactly the share key's placement.
    assert!(
        !store
            .record_browser_share_revocation(OWNER, &share, "ovh", || true)
            .await
            .unwrap()
    );
    assert!(
        store
            .record_browser_share_revocation(OWNER, &share, "hawk", || false)
            .await
            .is_err()
    );
    assert!(
        store
            .record_browser_share_revocation(OWNER, &share, "hawk", || true)
            .await
            .unwrap()
    );
    assert!(
        store
            .browser_placements(OWNER, &key)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .browser_shares(OWNER)
            .await
            .unwrap()
            .share(&share)
            .is_none()
    );
    // The member's own profile was never touched.
    assert_eq!(
        store.browser_placements(OWNER, &own).await.unwrap().len(),
        1
    );
}
