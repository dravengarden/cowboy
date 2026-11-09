use super::super::{Authority, Resolved, authorize_account, authorize_shared, instance_id, legacy};
use super::*;
use crate::machine_protocol::machine_browser::{InstanceRef, StateEvent};
use crate::store::provider_accounts::Account;
use crate::store::provider_accounts::browser_shares::{Share, ShareMember, share_key};
use crate::store::provider_accounts::legacy_browsers;

const OWNER: &str = "owner";

fn named(provider: &str, label: &str, browser_enabled: bool) -> Account {
    Account {
        id: uuid::Uuid::new_v4().to_string().try_into().unwrap(),
        provider_id: provider.into(),
        label: label.into(),
        enabled: true,
        revoking: false,
        identity_fingerprint: None,
        admission_revision: None,
        browser_enabled,
    }
}

fn families() -> BTreeMap<String, String> {
    family_map([
        ("claude-code".to_owned(), Some("anthropic".to_owned())),
        ("codex".to_owned(), Some("openai".to_owned())),
        ("grok".to_owned(), Some("xai".to_owned())),
    ])
}

/// Lynx, Pika and Fox (the legacy anthropic, openai and xai cards) share
/// "Google"; Lark (a named Claude account) and Mole keep their own.
fn google(share_id: &str) -> Shares {
    let row = |card: &str| ShareMember {
        member: Member::card(card),
        share_id: share_id.to_owned(),
        family: card.to_owned(),
    };
    Shares {
        shares: vec![Share {
            share_id: share_id.to_owned(),
            browser_key: share_key(OWNER, share_id),
            label: "Google".into(),
            created_ms: 1,
            revision: 4,
            deleting_ms: None,
        }],
        members: vec![row("anthropic"), row("openai"), row("xai")],
    }
}

fn resolve_card(
    shares: &Shares,
    card: &str,
    registry: &Registry,
) -> Result<(ProviderAccountId, Option<ShareView>), &'static str> {
    let families = families();
    resolve_key(
        Some(&Member::card(card)),
        &legacy_browsers::account(OWNER, card),
        shares,
        |member| family_now(member, registry, &families),
    )
}

#[test]
fn provider_families_follow_the_catalog_and_drop_disagreements() {
    let map = family_map([
        ("claude-code".to_owned(), Some("anthropic".to_owned())),
        ("claude-code".to_owned(), Some("anthropic".to_owned())),
        ("codex".to_owned(), Some("openai".to_owned())),
        ("odd".to_owned(), Some("openai".to_owned())),
        ("odd".to_owned(), Some("xai".to_owned())),
        ("plain".to_owned(), None),
    ]);
    assert_eq!(
        map.get("claude-code").map(String::as_str),
        Some("anthropic")
    );
    assert_eq!(map.get("codex").map(String::as_str), Some("openai"));
    assert!(!map.contains_key("odd"));
    assert!(!map.contains_key("plain"));
}

#[test]
fn every_member_resolves_to_the_share_key_and_non_members_keep_theirs() {
    let share_id = uuid::Uuid::new_v4().to_string();
    let shares = google(&share_id);
    let lark = named("claude-code", "Lark", true);
    let registry = Registry {
        revision: 1,
        accounts: vec![lark.clone()],
        defaults: BTreeMap::new(),
    };
    let key = share_key(OWNER, &share_id);
    for card in ["anthropic", "openai", "xai"] {
        let (resolved, view) = resolve_card(&shares, card, &registry).unwrap();
        assert_eq!(resolved, key, "{card}");
        let view = view.unwrap();
        assert_eq!(view.label, "Google");
        assert_eq!(view.members.len(), 3);
        // One instance per Machine for the whole share.
        assert_eq!(
            instance_id(OWNER, &resolved, "ovh"),
            instance_id(OWNER, &key, "ovh")
        );
    }
    // Mole and Lark are no members: their own keys, no share.
    let (mole, view) = resolve_card(&shares, "deepseek", &registry).unwrap();
    assert_eq!(mole, legacy_browsers::account(OWNER, "deepseek"));
    assert!(view.is_none());
    let (own, view) = resolve_key(
        Some(&Member::account(lark.id.clone())),
        &lark.id,
        &shares,
        |member| family_now(member, &registry, &families()),
    )
    .unwrap();
    assert_eq!((own, view), (lark.id.clone(), None));
    // A key known only from a retained placement has no member identity.
    let retained = legacy_browsers::account(OWNER, "gone");
    assert_eq!(
        resolve_key(None, &retained, &shares, |_| None).unwrap().0,
        retained
    );
    // The share key is none of the account or card keys.
    for other in [
        lark.id.clone(),
        legacy_browsers::account(OWNER, "anthropic"),
        legacy_browsers::account(OWNER, "openai"),
        legacy_browsers::account(OWNER, "xai"),
        legacy_browsers::account(OWNER, "deepseek"),
    ] {
        assert_ne!(key, other);
        assert_ne!(
            instance_id(OWNER, &key, "ovh"),
            instance_id(OWNER, &other, "ovh")
        );
    }
}

#[test]
fn resolution_refuses_a_share_that_would_mix_one_family() {
    let share_id = uuid::Uuid::new_v4().to_string();
    let lark = named("claude-code", "Lark", true);
    let registry = Registry {
        revision: 1,
        accounts: vec![lark.clone()],
        defaults: BTreeMap::new(),
    };
    // Should the store ever hold Lark next to Lynx, nobody shares.
    let mut mixed = google(&share_id);
    mixed.members.push(ShareMember {
        member: Member::account(lark.id.clone()),
        share_id: share_id.clone(),
        family: "anthropic".into(),
    });
    for card in ["anthropic", "openai"] {
        assert_eq!(
            resolve_card(&mixed, card, &registry).err(),
            Some("browser_share_family_conflict")
        );
    }
    // A member recorded under another family than its Provider has now.
    let mut drifted = google(&share_id);
    drifted.members.push(ShareMember {
        member: Member::account(lark.id.clone()),
        share_id: share_id.clone(),
        family: "deepseek".into(),
    });
    assert_eq!(
        resolve_card(&drifted, "xai", &registry).err(),
        Some("browser_share_family_conflict")
    );
    // A member whose family is no longer known.
    let mut unknown = google(&share_id);
    let stranger = named("mystery", "?", true);
    unknown.members.push(ShareMember {
        member: Member::account(stranger.id.clone()),
        share_id: share_id.clone(),
        family: "mystery".into(),
    });
    let registry = Registry {
        accounts: vec![stranger],
        ..registry
    };
    assert_eq!(
        resolve_card(&unknown, "openai", &registry).err(),
        Some("browser_share_family_unknown")
    );
    // A share being deleted is never resolved.
    let mut deleting = google(&share_id);
    deleting.shares[0].deleting_ms = Some(9);
    assert_eq!(
        resolve_card(&deleting, "openai", &Registry::default()).err(),
        Some("browser_share_changed")
    );
}

fn card_account(card: &str, enabled: bool) -> BrowserAccount {
    BrowserAccount::Legacy(legacy::LegacyCard {
        card: Some(card.to_owned()),
        bound: true,
        enabled,
    })
}

fn resolved(card: &str, enabled: bool, share_id: &str) -> Resolved {
    let shares = google(share_id);
    let (key, share) = resolve_card(&shares, card, &Registry::default()).unwrap();
    Resolved {
        owner: OWNER.into(),
        id: legacy_browsers::account(OWNER, card),
        account: card_account(card, enabled),
        key,
        share,
    }
}

#[test]
fn a_member_opens_the_share_only_with_its_own_browser_enabled() {
    let share_id = uuid::Uuid::new_v4().to_string();
    let pika = resolved("openai", true, &share_id);
    let fox = resolved("xai", false, &share_id);
    assert_eq!(pika.key, fox.key);
    assert_eq!(
        authorize_account(Authority::Launch, true, &pika.account),
        Ok(())
    );
    // Fox's own browser is off: Fox cannot open or attach to the share,
    // although the share is open for Pika.
    assert_eq!(
        authorize_account(Authority::Launch, true, &fox.account),
        Err("browser_not_enabled")
    );
    assert_eq!(
        member_of(&pika.id, &pika.account),
        Some(Member::card("openai"))
    );
    // Resetting the share's profile also takes the member's own access;
    // stopping it stays possible.
    assert_eq!(authorize_shared(Authority::Reset, &pika.account), Ok(()));
    assert_eq!(
        authorize_shared(Authority::Reset, &fox.account),
        Err("browser_not_enabled")
    );
    assert_eq!(authorize_shared(Authority::Stop, &fox.account), Ok(()));
}

#[test]
fn a_member_sees_and_names_the_share_instance_as_its_own_card() {
    let share_id = uuid::Uuid::new_v4().to_string();
    let pika = resolved("openai", true, &share_id);
    let event = StateEvent {
        request_id: None,
        instance: InstanceRef {
            account_id: pika.key.clone(),
            instance_id: instance_id(OWNER, &pika.key, "ovh"),
            incarnation: 3,
        },
        state: BrowserState::Detached,
        reason: None,
        geometry: None,
        scale: None,
        deadline_ms: None,
        attachment_id: None,
        snapshot_generation: None,
        snapshot_at_ms: None,
        profile_revision: Some(2),
        unsaved_state_lost: false,
        observed_at_ms: 1,
    };
    let shown = pika.present(&event);
    assert_eq!(shown.instance.account_id, pika.id);
    // The shown instance commands the share's instance on the Machine.
    assert_eq!(pika.inbound(&shown.instance).unwrap(), event.instance);
    // Another card's identity is never accepted for it.
    let fox = resolved("xai", true, &share_id);
    assert_eq!(
        pika.inbound(&fox.present(&event).instance),
        Err("invalid_argument")
    );
    let value = pika.state_json("ovh", &event);
    assert_eq!(value["share"]["label"], "Google");
    assert_eq!(value["share"]["members"].as_array().unwrap().len(), 3);
    assert_eq!(value["state"]["instance"]["account_id"], json!(pika.id));
}

fn receipt(account_id: ProviderAccountId) -> Receipt {
    Receipt {
        account_id,
        ..Receipt::default()
    }
}

#[test]
fn only_deleting_a_share_revokes_its_browser() {
    let share_id = uuid::Uuid::new_v4().to_string();
    let key = share_key(OWNER, &share_id);
    let lark = named("claude-code", "Lark", true);
    let lynx = Member::card("anthropic");
    let lynx_key = lynx.own_key(OWNER);
    let member_changes = [
        (
            Change::SetBrowserShare {
                member: lynx.clone(),
                share_id: None,
            },
            lynx_key.clone(),
        ),
        (
            Change::SetBrowserShare {
                member: lynx.clone(),
                share_id: Some(share_id.clone()),
            },
            lynx_key.clone(),
        ),
        (
            Change::SetLegacyBrowserEnabled {
                card: "anthropic".into(),
                enabled: false,
            },
            lynx_key.clone(),
        ),
        (
            Change::SetBrowserEnabled {
                id: lark.id.clone(),
                enabled: false,
            },
            lark.id.clone(),
        ),
        (
            Change::SetEnabled {
                id: lark.id.clone(),
                enabled: false,
            },
            lark.id.clone(),
        ),
        (
            Change::BeginRevocation {
                id: lark.id.clone(),
            },
            lark.id.clone(),
        ),
    ];
    for (change, own) in member_changes {
        let effects = change_effects(OWNER, &change, &receipt(own.clone()));
        assert!(!effects.is_empty(), "{change:?}");
        for effect in &effects {
            match effect {
                // Only the member's own browser and its own viewers.
                Effect::EndMemberViewers(member) | Effect::HibernateOwn(member, _) => {
                    assert_eq!(member, &own, "{change:?}");
                    assert_ne!(member, &key);
                }
                Effect::EndKeyViewers(_) | Effect::RevokeShare(_) => {
                    panic!("{change:?} touched the share: {effect:?}")
                }
            }
        }
    }
    // Deleting the share revokes exactly the share's browser.
    assert_eq!(
        change_effects(
            OWNER,
            &Change::DeleteBrowserShare {
                share_id: share_id.clone()
            },
            &receipt(key.clone())
        ),
        vec![
            Effect::EndKeyViewers(key),
            Effect::RevokeShare(share_id.clone())
        ]
    );
    // Creating and renaming touch no browser.
    for change in [
        Change::CreateBrowserShare {
            label: "Google".into(),
        },
        Change::RenameBrowserShare {
            share_id: share_id.clone(),
            label: "G".into(),
        },
    ] {
        assert!(change_effects(OWNER, &change, &Receipt::default()).is_empty());
    }
    // The Machine operation of a deletion is stable across retries.
    assert_eq!(
        machine_operation_id(OWNER, &share_id),
        machine_operation_id(OWNER, &share_id)
    );
    assert_ne!(
        machine_operation_id(OWNER, &share_id),
        machine_operation_id("other", &share_id)
    );
}
