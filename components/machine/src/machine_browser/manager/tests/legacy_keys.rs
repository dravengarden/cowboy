//! Legacy default card keys: browser-only account directories that never
//! touch the legacy default's native credentials or another key's profile.

use super::*;

/// A legacy card browser key as the Controller derives it (UUID version 8).
const CARD: &str = "0b1c2d3e-4f50-8a61-b2c3-d4e5f6a7b8c9";

fn manager(state: &Path) -> Arc<BrowserManager> {
    let store = MachinePluginStore::new(
        state,
        crate::machine_protocol::Platform::Linux,
        "x86_64".into(),
    )
    .unwrap();
    BrowserManager::build(
        state,
        Arc::new(store),
        None,
        || None,
        Arc::new(FakeUnits::default()) as Arc<dyn UnitControl>,
    )
    .unwrap()
}

fn seed_snapshot(account_dir: &Path, marker: &str) {
    let generation = state_file::snapshots_root(account_dir).join(state_file::generation_name(1));
    std::fs::create_dir_all(&generation).unwrap();
    std::fs::write(generation.join("Cookies"), marker).unwrap();
}

#[tokio::test]
async fn a_card_key_resets_only_its_own_browser_and_never_native_credentials() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let manager = manager(&state);
    let card: ProviderAccountId = CARD.to_owned().try_into().unwrap();
    let named: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
    // The legacy default's native credentials live directly under provider-auth.
    let native = state.join("provider-auth/providers/claude-code/materialized");
    std::fs::create_dir_all(&native).unwrap();
    std::fs::write(native.join("credentials"), "native").unwrap();

    let card_dir = manager.account_dir(&card).unwrap();
    let named_dir = manager.account_dir(&named).unwrap();
    assert_eq!(card_dir, state.join("provider-auth/accounts").join(CARD));
    assert_ne!(card_dir, named_dir);
    // Browser state never creates a credential scope for the key.
    assert!(!card_dir.join("providers").exists());
    seed_snapshot(&card_dir, "card");
    seed_snapshot(&named_dir, "named");

    let event = manager
        .reset_or_revoke(
            "reset-card",
            card.clone(),
            OperationKind::Reset,
            "op-card",
            None,
        )
        .await;
    assert_eq!(event.state, State::Hibernated);
    assert_eq!(event.reason, Some(Reason::OwnerRequest));
    assert_eq!(event.instance.account_id, card);
    assert!(!state_file::snapshots_root(&card_dir).exists());
    // The control record survives; the other key's profile and the native
    // credentials are untouched.
    assert!(
        state_file::read_control(&card_dir)
            .unwrap()
            .profile_revision
            > 0
    );
    let named_cookies = state_file::snapshots_root(&named_dir)
        .join(state_file::generation_name(1))
        .join("Cookies");
    assert_eq!(std::fs::read_to_string(named_cookies).unwrap(), "named");
    assert_eq!(
        std::fs::read_to_string(native.join("credentials")).unwrap(),
        "native"
    );
    // The shared default identity has no browser.
    assert!(manager.account_dir(&ProviderAccountId::default()).is_err());
}
