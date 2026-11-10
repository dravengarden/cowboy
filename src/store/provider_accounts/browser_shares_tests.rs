use super::*;
use crate::store::provider_accounts::Account;
use std::collections::BTreeMap;

const OWNER: &str = "owner";

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
    placed: BTreeSet<ProviderAccountId>,
}

impl Fixture {
    fn context(&self, new_share_id: &str) -> ShareContext<'_> {
        ShareContext {
            owner: OWNER,
            registry: &self.registry,
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
            .mutate_browser_shares(OWNER, operation, &input)
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
            .mutate_browser_shares(OWNER, "create-share", &replay(" Google "))
            .await
            .unwrap(),
        MutationResult::Applied(receipt.clone())
    );
    assert_eq!(
        store
            .mutate_browser_shares(OWNER, "create-share", &replay("X"))
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
    store
        .admit_shared_browser_placement(OWNER, (&lynx, &share), "ovh", "b-share", 2)
        .await
        .unwrap()
        .unwrap();
    // A second placement from before the one-Machine rule is revoked too.
    store
        .upsert_browser_placement(
            OWNER,
            &super::super::browser_placements::Placement {
                account_id: key.clone(),
                machine_id: "hawk".into(),
                instance_id: "b-share".into(),
                profile_revision: 0,
                last_open_ms: 2,
            },
        )
        .await
        .unwrap();
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

#[test]
fn a_family_stays_bound_to_its_first_member_until_the_profile_is_fresh() {
    let lark = named("claude-code", "Lark");
    let fixture = Fixture {
        registry: Registry {
            revision: 1,
            accounts: vec![lark.clone()],
            defaults: BTreeMap::new(),
        },
        placed: BTreeSet::new(),
    };
    let mut shares = Shares::default();
    let google = fixture.create(&mut shares, "Google");
    let lynx = Member::card("anthropic");
    fixture
        .set(&mut shares, lynx.clone(), Some(&google))
        .unwrap();
    // Lynx leaves; the profile may still hold Lynx's Claude cookies.
    fixture.set(&mut shares, lynx.clone(), None).unwrap();
    assert_eq!(
        fixture.set(&mut shares, Member::account(lark.id.clone()), Some(&google)),
        Err("browser_share_family_bound")
    );
    // Lynx itself may come back.
    fixture
        .set(&mut shares, lynx.clone(), Some(&google))
        .unwrap();
    fixture.set(&mut shares, lynx.clone(), None).unwrap();
    // A fresh profile (confirmed reset) forgets the members that left.
    // Lynx had left before the reset began: its binding is released.
    assert!(
        shares.renew_current(&google, &[Member::card("anthropic")], |member| {
            family_of(member, &fixture.registry)
        })
    );
    fixture
        .set(&mut shares, Member::account(lark.id.clone()), Some(&google))
        .unwrap();
    // Renaming is no membership change.
    let revision = shares.share(&google).unwrap().revision;
    apply(
        &mut shares,
        &Change::RenameBrowserShare {
            share_id: google.clone(),
            label: "G".into(),
        },
        &fixture.context("unused"),
    )
    .unwrap();
    assert_eq!(shares.share(&google).unwrap().revision, revision);
}

#[test]
fn a_member_must_be_the_owners_even_to_leave() {
    let fixture = Fixture {
        registry: Registry::default(),
        placed: BTreeSet::new(),
    };
    let mut shares = Shares::default();
    let foreign: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
    assert_eq!(
        fixture.set(&mut shares, Member::account(foreign), None),
        Err("account_not_found")
    );
    // A card is always the owner's own (its key derives from the owner).
    fixture
        .set(&mut shares, Member::card("openai"), None)
        .unwrap();
}

#[tokio::test]
async fn a_left_or_revoked_members_family_stays_bound_until_a_confirmed_reset() {
    let dir = tempfile::tempdir().unwrap();
    family_binding(&sqlite_store(&dir).await).await;
}

#[allow(clippy::too_many_lines)] // One scenario: a family binding across leave, revoke, reset and delete.
async fn family_binding(store: &Store) {
    let lark = named_account(store, "bind-lark", "claude-code").await;
    let share = applied(
        store,
        "bind-create",
        Change::CreateBrowserShare {
            label: "Google".into(),
        },
    )
    .await
    .browser_share_id
    .unwrap();
    let lynx = Member::card("anthropic");
    applied(store, "bind-lynx-join", join(lynx.clone(), &share)).await;
    applied(
        store,
        "bind-lynx-leave",
        Change::SetBrowserShare {
            member: lynx.clone(),
            share_id: None,
        },
    )
    .await;
    assert_eq!(
        invalid(
            store,
            "bind-lark-join",
            join(Member::account(lark.clone()), &share)
        )
        .await,
        "browser_share_family_bound"
    );
    // A reset is renewed only when the share's profile lives on that Machine alone.
    let pika = Member::card("openai");
    applied(
        store,
        "bind-pika-on",
        Change::SetLegacyBrowserEnabled {
            card: "openai".into(),
            enabled: true,
        },
    )
    .await;
    applied(store, "bind-pika-join", join(pika.clone(), &share)).await;
    store
        .admit_shared_browser_placement(OWNER, (&pika, &share), "ovh", "b-share", 1)
        .await
        .unwrap()
        .unwrap();
    // A concurrent first open on another Machine is refused.
    assert_eq!(
        store
            .admit_shared_browser_placement(OWNER, (&pika, &share), "hawk", "b-share", 1)
            .await
            .unwrap(),
        Err(Refusal::ShareElsewhere)
    );
    // Lynx left before this reset began; its own confirmation renews.
    assert!(confirmed_reset(store, &share, "ovh", "bind-reset-1").await);
    applied(
        store,
        "bind-lark-join-2",
        join(Member::account(lark.clone()), &share),
    )
    .await;
    // Lark is revoked: its membership goes, its family binding stays.
    applied(
        store,
        "bind-lark-revoke",
        Change::BeginRevocation { id: lark.clone() },
    )
    .await;
    let shares = store.browser_shares(OWNER).await.unwrap();
    assert!(shares.membership(&Member::account(lark.clone())).is_none());
    assert_eq!(
        invalid(store, "bind-lynx-join-2", join(lynx.clone(), &share)).await,
        "browser_share_family_bound"
    );
    // A reset confirmed on a Machine other than the share's is no renewal.
    assert!(!confirmed_reset(store, &share, "hawk", "bind-reset-2").await);
    // Delete and recreate: a new share, a new profile, no binding.
    applied(
        store,
        "bind-delete",
        Change::DeleteBrowserShare {
            share_id: share.clone(),
        },
    )
    .await;
    assert_eq!(
        store.deleting_browser_shares().await.unwrap(),
        vec![(OWNER.to_owned(), share.clone())]
    );
    let fresh = applied(
        store,
        "bind-recreate",
        Change::CreateBrowserShare {
            label: "Google".into(),
        },
    )
    .await
    .browser_share_id
    .unwrap();
    applied(store, "bind-lynx-fresh", join(lynx, &fresh)).await;
}

#[tokio::test]
#[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
async fn postgres_browser_share_families_stay_bound() {
    let dir = tempfile::tempdir().unwrap();
    family_binding(&postgres_store(&dir).await).await;
}

#[test]
fn a_binding_whose_family_the_table_no_longer_gives_blocks_the_share() {
    // Only a reviewed change of the static table can remap a family; the
    // defensive check still blocks the share. Lark's binding is recorded
    // under a family the table does not give claude-code.
    let lark = named("claude-code", "Lark");
    let test = named("claude-code", "Test");
    let fixture = Fixture {
        registry: Registry {
            revision: 1,
            accounts: vec![lark.clone(), test.clone()],
            defaults: BTreeMap::new(),
        },
        placed: BTreeSet::new(),
    };
    let mut shares = Shares::default();
    let google = fixture.create(&mut shares, "Google");
    shares.bindings.push(Binding {
        share_id: google.clone(),
        family: "claude".into(),
        member: Member::account(lark.id.clone()),
    });
    for member in [Member::account(test.id.clone()), Member::card("openai")] {
        assert_eq!(
            fixture.set(&mut shares, member, Some(&google)),
            Err("browser_share_family_drift")
        );
    }
    // A confirmed reset releasing Lark (it had left) clears the drift.
    assert!(
        shares.renew_current(&google, &[Member::account(lark.id)], |member| {
            family_of(member, &fixture.registry)
        })
    );
    fixture
        .set(&mut shares, Member::account(test.id.clone()), Some(&google))
        .unwrap();
    assert_eq!(
        shares.binding(&google, "anthropic").map(|row| &row.member),
        Some(&Member::account(test.id))
    );
}

#[test]
fn families_come_only_from_the_static_table() {
    let registry = Registry {
        revision: 1,
        accounts: vec![
            named("claude-code", "Lark"),
            named("codex-deepseek", "Mole"),
            named("future-provider", "New"),
        ],
        defaults: BTreeMap::new(),
    };
    let family = |member: Member| family_of(&member, &registry);
    assert_eq!(
        family(Member::account(registry.accounts[0].id.clone())).as_deref(),
        Some("anthropic")
    );
    assert_eq!(
        family(Member::account(registry.accounts[1].id.clone())).as_deref(),
        Some("deepseek")
    );
    assert_eq!(
        family(Member::account(registry.accounts[2].id.clone())),
        None
    );
    assert_eq!(family(Member::card("xai")).as_deref(), Some("xai"));
    // A card the table does not know has no family and never joins.
    assert_eq!(family(Member::card("future-cloud")), None);
    assert_eq!(
        member_family(&Member::card("future-cloud"), &registry),
        Err("browser_share_family_unknown")
    );
}

#[test]
fn a_bound_member_now_of_the_joining_family_blocks_it() {
    // Recorded under `deepseek`, Lark's binding is consistent; but a member
    // whose CURRENT family equals a binding's current family is refused.
    let lark = named("claude-code", "Lark");
    let shares = Shares {
        shares: Vec::new(),
        members: Vec::new(),
        bindings: vec![Binding {
            share_id: "s".into(),
            family: "anthropic".into(),
            member: Member::account(lark.id.clone()),
        }],
    };
    let registry = Registry {
        revision: 1,
        accounts: vec![lark],
        defaults: BTreeMap::new(),
    };
    let current = |member: &Member| family_of(member, &registry);
    assert_eq!(
        binding_refusal(
            &shares,
            "s",
            (&Member::card("anthropic"), "anthropic"),
            current
        ),
        Some("browser_share_family_bound")
    );
    assert_eq!(
        binding_refusal(&shares, "s", (&Member::card("openai"), "openai"), current),
        None
    );
}

#[tokio::test]
async fn revoking_a_member_moves_the_share_identity() {
    let dir = tempfile::tempdir().unwrap();
    let store = sqlite_store(&dir).await;
    let codex = named_account(&store, "rev-codex", "codex").await;
    let share = applied(
        &store,
        "rev-create",
        Change::CreateBrowserShare {
            label: "Google".into(),
        },
    )
    .await
    .browser_share_id
    .unwrap();
    applied(
        &store,
        "rev-join",
        join(Member::account(codex.clone()), &share),
    )
    .await;
    applied(&store, "rev-fox", join(Member::card("xai"), &share)).await;
    let before = store
        .browser_shares(OWNER)
        .await
        .unwrap()
        .share(&share)
        .unwrap()
        .revision;
    applied(&store, "rev-revoke", Change::BeginRevocation { id: codex }).await;
    let after = store
        .browser_shares(OWNER)
        .await
        .unwrap()
        .share(&share)
        .unwrap()
        .revision;
    // A reset captured with `share:<id>:<before>` no longer matches.
    assert!(after > before);
}

async fn confirmed_reset(store: &Store, share: &str, machine: &str, operation: &str) -> bool {
    let revision = store
        .browser_shares(OWNER)
        .await
        .unwrap()
        .share(share)
        .unwrap()
        .revision;
    store
        .begin_share_reset(OWNER, (share, revision), (machine, 4), (operation, 1))
        .await
        .unwrap()
        .unwrap();
    store
        .finish_share_reset(OWNER, (share, operation), true)
        .await
        .unwrap()
        .is_some()
}

#[tokio::test]
#[allow(clippy::too_many_lines)] // One scenario: a reset fence across timeout, restart and outcome.
async fn a_pending_share_reset_fences_membership_until_its_outcome() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!(
        "sqlite://{}?mode=rwc",
        dir.path().join("shares.db").display()
    );
    let store = sqlite_store(&dir).await;
    let lark = named_account(&store, "fence-lark", "claude-code").await;
    let codex = named_account(&store, "fence-codex", "codex").await;
    let share = applied(
        &store,
        "fence-create",
        Change::CreateBrowserShare {
            label: "Google".into(),
        },
    )
    .await
    .browser_share_id
    .unwrap();
    let lynx = Member::card("anthropic");
    applied(&store, "fence-lynx", join(lynx.clone(), &share)).await;
    applied(
        &store,
        "fence-codex-join",
        join(Member::account(codex.clone()), &share),
    )
    .await;
    applied(
        &store,
        "fence-lynx-leave",
        Change::SetBrowserShare {
            member: lynx.clone(),
            share_id: None,
        },
    )
    .await;
    let revision = store
        .browser_shares(OWNER)
        .await
        .unwrap()
        .share(&share)
        .unwrap()
        .revision;
    // A reset captured before a membership change is refused.
    assert_eq!(
        store
            .begin_share_reset(OWNER, (&share, revision - 1), ("ovh", 4), ("op-old", 1))
            .await
            .unwrap(),
        Err("browser_identity_changed")
    );
    store
        .begin_share_reset(OWNER, (&share, revision), ("ovh", 4), ("op-1", 1))
        .await
        .unwrap()
        .unwrap();
    // The reset timed out; the Controller restarts. The fence survives.
    drop(store);
    let store = Store::connect(&url, dir.path().join("artifacts"))
        .await
        .unwrap();
    let pending = store.pending_share_resets().await.unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].2.operation_id, "op-1");
    assert_eq!(pending[0].2.released, vec![lynx.encode()]);
    assert_eq!(
        store
            .begin_share_reset(OWNER, (&share, revision), ("ovh", 4), ("op-2", 1))
            .await
            .unwrap(),
        Err("browser_share_reset_pending")
    );
    // No membership change of the share commits meanwhile.
    for (operation, change) in [
        ("fence-pika", join(Member::card("openai"), &share)),
        (
            "fence-codex-leave",
            Change::SetBrowserShare {
                member: Member::account(codex.clone()),
                share_id: None,
            },
        ),
    ] {
        assert_eq!(
            invalid(&store, operation, change).await,
            "browser_share_reset_pending"
        );
    }
    // A stale operation clears nothing; the reconciled outcome (not this
    // reset's own confirmation) clears the fence without renewing.
    assert!(
        store
            .finish_share_reset(OWNER, (&share, "op-x"), true)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(store.pending_share_resets().await.unwrap().len(), 1);
    assert!(
        store
            .finish_share_reset(OWNER, (&share, "op-1"), false)
            .await
            .unwrap()
            .is_none()
    );
    assert!(store.pending_share_resets().await.unwrap().is_empty());
    // Lynx's binding stays: Lark is still refused.
    assert_eq!(
        invalid(
            &store,
            "fence-lark-join",
            join(Member::account(lark.clone()), &share)
        )
        .await,
        "browser_share_family_bound"
    );
    // A reset that confirmed itself releases Lynx and advances the
    // registry revision (so other clients refresh); Lark may join.
    let before = store.provider_accounts(OWNER).await.unwrap().revision;
    assert!(confirmed_reset(&store, &share, "ovh", "op-3").await);
    assert_eq!(
        store.provider_accounts(OWNER).await.unwrap().revision,
        before + 1
    );
    applied(&store, "fence-lark-2", join(Member::account(lark), &share)).await;
    // Revocation is never fenced; deletion clears a pending reset.
    let revision = store
        .browser_shares(OWNER)
        .await
        .unwrap()
        .share(&share)
        .unwrap()
        .revision;
    store
        .begin_share_reset(OWNER, (&share, revision), ("ovh", 5), ("op-4", 1))
        .await
        .unwrap()
        .unwrap();
    applied(
        &store,
        "fence-revoke",
        Change::BeginRevocation { id: codex },
    )
    .await;
    applied(
        &store,
        "fence-delete",
        Change::DeleteBrowserShare { share_id: share },
    )
    .await;
    assert!(store.pending_share_resets().await.unwrap().is_empty());
}
