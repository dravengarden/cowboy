//! Replays of a reset by its request id: a settled answer is replayed from
//! the receipt cache, an unsettled one (failed, busy) runs again against the
//! durable operation state, so the Controller's retry of the same operation
//! settles once the Machine recovers.

use super::*;

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
        Box::leak(Box::new(Ledger::new())),
        Arc::new(FakeUnits::default()) as Arc<dyn UnitControl>,
    )
    .unwrap()
}

#[tokio::test]
async fn an_unsettled_reset_is_not_replayed_from_the_receipt_cache() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let manager = manager(&state);
    let account: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
    let dir = manager.account_dir(&account).unwrap();
    let revision = state_file::read_control(&dir).unwrap().profile_revision;
    // The control record is unreadable: the reset fails before any fence.
    let control = dir.join(state_file::CONTROL_FILE);
    let healthy = std::fs::read(&control).ok();
    std::fs::write(&control, b"not json").unwrap();
    let reset = || Command::Reset {
        account_id: account.clone(),
        expected_profile_revision: revision,
    };
    manager.handle("browser-reset-1".into(), reset()).await;
    assert!(manager.receipt("browser-reset-1").is_none());
    // The Machine recovers; the Controller retries the same operation.
    match healthy {
        Some(bytes) => std::fs::write(&control, bytes).unwrap(),
        None => std::fs::remove_file(&control).unwrap(),
    }
    manager.handle("browser-reset-1".into(), reset()).await;
    let settled = manager.receipt("browser-reset-1").unwrap();
    assert_eq!(settled.reason, Some(Reason::OwnerRequest));
    let after = state_file::read_control(&dir).unwrap();
    assert_eq!(after.profile_revision, revision + 1);
    // A settled answer is replayed as recorded, never run twice.
    manager.handle("browser-reset-1".into(), reset()).await;
    assert_eq!(
        state_file::read_control(&dir).unwrap().profile_revision,
        revision + 1
    );
}

#[tokio::test]
async fn a_later_reset_overwriting_the_last_operation_makes_a_replay_superseded() {
    // Reset A completes, reset B completes and overwrites the remembered last
    // operation; a replay of A (outside the receipt cache) is `superseded`:
    // it runs no further, although it did run once.
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    std::fs::create_dir_all(&state).unwrap();
    let manager = manager(&state);
    let account: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
    let dir = manager.account_dir(&account).unwrap();
    let revision = state_file::read_control(&dir).unwrap().profile_revision;
    let a = manager
        .reset_or_revoke(
            "a",
            account.clone(),
            OperationKind::Reset,
            "a",
            Some(revision),
        )
        .await;
    assert_eq!(a.reason, Some(Reason::OwnerRequest));
    let b = manager
        .reset_or_revoke(
            "b",
            account.clone(),
            OperationKind::Reset,
            "b",
            Some(revision + 1),
        )
        .await;
    assert_eq!(b.reason, Some(Reason::OwnerRequest));
    let replay = manager
        .reset_or_revoke(
            "a",
            account.clone(),
            OperationKind::Reset,
            "a",
            Some(revision),
        )
        .await;
    assert_eq!(replay.reason, Some(Reason::Superseded));
    assert_eq!(
        state_file::read_control(&dir).unwrap().profile_revision,
        revision + 2
    );
}
