use super::{Store, test_support};

/// A schedule is owner-scoped, replaced in place, deferred and claimed only
/// under its own key, and claimed exactly once.
#[allow(clippy::too_many_lines)] // One sequence across replace, defer, claim and cancel.
async fn assert_account_reset_schedules_contract(store: &Store) {
    let (owner, other, account) = ("owner", "other", "acct-1");
    assert_eq!(
        store.account_reset_schedule(owner, account).await.unwrap(),
        None
    );
    store
        .upsert_account_reset_schedule(owner, account, 1_000, "key-a", "sha256:a")
        .await
        .unwrap();
    assert_eq!(
        store.account_reset_schedule(other, account).await.unwrap(),
        None
    );
    assert!(
        store
            .account_reset_schedules(other)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        !store
            .delete_account_reset_schedule(other, account)
            .await
            .unwrap()
    );
    // Not due before its time; due at it.
    assert!(
        store
            .due_account_reset_schedules(999, 10)
            .await
            .unwrap()
            .is_empty()
    );
    let due = store.due_account_reset_schedules(1_000, 10).await.unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].idempotency_key, "key-a");
    assert_eq!(due[0].next_attempt_at_ms, 1_000);
    // A pacing wait moves it without counting; a failure counts.
    assert!(
        store
            .defer_account_reset_schedule(owner, account, "key-a", 2_000, false)
            .await
            .unwrap()
    );
    assert!(
        store
            .defer_account_reset_schedule(owner, account, "key-a", 3_000, true)
            .await
            .unwrap()
    );
    let stored = store
        .account_reset_schedule(owner, account)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (stored.next_attempt_at_ms, stored.attempt_count),
        (3_000, 1)
    );
    assert_eq!(stored.fire_at_ms, 1_000);
    // A replacement resets the attempt state and retires the earlier key.
    store
        .upsert_account_reset_schedule(owner, account, 5_000, "key-b", "sha256:b")
        .await
        .unwrap();
    let stored = store
        .account_reset_schedule(owner, account)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            stored.fire_at_ms,
            stored.next_attempt_at_ms,
            stored.attempt_count
        ),
        (5_000, 5_000, 0)
    );
    assert_eq!(stored.authority, "sha256:b");
    assert!(
        !store
            .defer_account_reset_schedule(owner, account, "key-a", 9_000, true)
            .await
            .unwrap()
    );
    assert!(
        !store
            .claim_account_reset_schedule(owner, account, "key-a")
            .await
            .unwrap()
    );
    // Exactly one claim of the current key wins.
    assert!(
        store
            .claim_account_reset_schedule(owner, account, "key-b")
            .await
            .unwrap()
    );
    assert!(
        !store
            .claim_account_reset_schedule(owner, account, "key-b")
            .await
            .unwrap()
    );
    assert_eq!(
        store.account_reset_schedule(owner, account).await.unwrap(),
        None
    );
    // Cancellation removes it.
    store
        .upsert_account_reset_schedule(owner, account, 7_000, "key-c", "sha256:c")
        .await
        .unwrap();
    assert_eq!(store.account_reset_schedules(owner).await.unwrap().len(), 1);
    assert!(
        store
            .delete_account_reset_schedule(owner, account)
            .await
            .unwrap()
    );
    assert!(
        store
            .due_account_reset_schedules(i64::MAX, 10)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn sqlite_account_reset_schedules_contract() {
    assert_account_reset_schedules_contract(&test_support::sqlite().await.store).await;
}

#[tokio::test]
#[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
async fn postgres_account_reset_schedules_contract() {
    assert_account_reset_schedules_contract(&test_support::postgres().await.store).await;
}
