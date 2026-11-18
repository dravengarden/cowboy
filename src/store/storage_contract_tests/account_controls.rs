//! Account controls (`crate::account_controls`) against a real store of each
//! backend: revision-checked state changes with their fences in one
//! transaction and automatic raises with their stop fences.

use super::test_support;
use crate::account_controls::{AccountState, FenceState};
use crate::provider_account::AccountFenceAction;
use crate::store::Store;
use crate::store::account_controls::{Raise, StateChange};

const ACCOUNT: &str = "11111111-1111-4111-8111-111111111111";

fn change(
    state: AccountState,
    expected_revision: Option<i64>,
    fence: Option<(AccountFenceAction, &[String])>,
) -> StateChange<'_> {
    StateChange {
        account_id: ACCOUNT,
        owner_user_id: "owner",
        provider_id: "antigravity",
        state,
        reason: "reason\u{0}with control",
        until_ms: None,
        expected_revision,
        fence,
        now_ms: 10,
    }
}

async fn state_contract(store: &Store) {
    let machines = vec!["ovh".to_owned(), "hawk".to_owned()];
    // The kill switch: the state and its fences commit together.
    let blocked = store
        .set_account_state(&change(
            AccountState::Blocked,
            Some(0),
            Some((AccountFenceAction::Stop, &machines)),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (blocked.revision, blocked.reason.as_str()),
        (1, "reasonwith control")
    );
    // A stale owner revision is refused, an automatic change is not.
    assert!(
        store
            .set_account_state(&change(AccountState::Ok, Some(0), None))
            .await
            .unwrap()
            .is_none()
    );
    let fences = store.account_fences().await.unwrap();
    assert_eq!(fences.len(), 2);
    assert!(fences.iter().all(
        |fence| fence.state == FenceState::Pending && fence.action == AccountFenceAction::Stop
    ));
    let stop = &fences[0];
    // Only the same pending operation is confirmed.
    let mut other = stop.clone();
    other.operation_id = "account-fence-other".into();
    assert!(
        !store
            .record_account_fence(&other, true, "", 11)
            .await
            .unwrap()
    );
    assert!(
        store
            .record_account_fence(stop, true, "", 11)
            .await
            .unwrap()
    );
    assert!(
        !store
            .record_account_fence(stop, true, "", 12)
            .await
            .unwrap()
    );
    let unblocked = store
        .set_account_state(&change(
            AccountState::Ok,
            Some(1),
            Some((AccountFenceAction::Admit, &machines)),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unblocked.revision, 2);
    let fences = store.account_fences().await.unwrap();
    assert!(
        fences
            .iter()
            .all(|fence| fence.action == AccountFenceAction::Admit
                && fence.state == FenceState::Pending
                && fence.operation_id != stop.operation_id)
    );
    let controls = store.account_controls().await.unwrap();
    assert_eq!(controls.len(), 1);
    assert_eq!(controls[0].state, AccountState::Ok);
}

fn raise<'a>(
    state: (AccountState, &'a str, Option<i64>),
    stop_units: bool,
    machines: &'a [String],
) -> Raise<'a> {
    Raise {
        account_id: ACCOUNT,
        owner_user_id: "owner",
        provider_id: "antigravity",
        state,
        stop_units,
        machines,
        now_ms: 20,
    }
}

/// An automatic raise is one transaction: a more severe state (above all the
/// owner's block) is kept, and the stop fences commit with the state.
async fn raise_contract(store: &Store) {
    let machines = vec!["ovh".to_owned()];
    let before = store.account_control(ACCOUNT).await.unwrap().unwrap();
    assert_eq!((before.state, before.revision), (AccountState::Ok, 2));
    // The owner blocks; a backend signal keeps the block but still queues
    // its stop fences.
    store
        .set_account_state(&change(AccountState::Blocked, Some(2), None))
        .await
        .unwrap()
        .unwrap();
    assert!(
        store
            .raise_account_state(&raise(
                (
                    AccountState::NeedsAttention,
                    "policy_violation:read_file",
                    None
                ),
                true,
                &machines,
            ))
            .await
            .unwrap()
    );
    let control = store.account_control(ACCOUNT).await.unwrap().unwrap();
    assert_eq!(
        (control.state, control.revision),
        (AccountState::Blocked, 4)
    );
    let fence = store
        .account_fences()
        .await
        .unwrap()
        .into_iter()
        .find(|fence| fence.machine_id == "ovh")
        .unwrap();
    assert_eq!(
        (fence.action, fence.state),
        (AccountFenceAction::Stop, FenceState::Pending)
    );
    // A lower automatic state never replaces a more severe one.
    store
        .set_account_state(&change(AccountState::Suspended, Some(4), None))
        .await
        .unwrap()
        .unwrap();
    assert!(
        !store
            .raise_account_state(&raise(
                (AccountState::RateLimited, "rate_limited", Some(99)),
                false,
                &machines,
            ))
            .await
            .unwrap()
    );
    let control = store.account_control(ACCOUNT).await.unwrap().unwrap();
    assert_eq!(
        (control.state, control.revision),
        (AccountState::Suspended, 5)
    );
    // An equal or more severe one is written.
    store
        .set_account_state(&change(AccountState::Ok, Some(5), None))
        .await
        .unwrap()
        .unwrap();
    assert!(
        store
            .raise_account_state(&raise(
                (AccountState::RateLimited, "rate_limited", Some(99)),
                false,
                &machines,
            ))
            .await
            .unwrap()
    );
    let control = store.account_control(ACCOUNT).await.unwrap().unwrap();
    assert_eq!(
        (control.state, control.until_ms),
        (AccountState::RateLimited, Some(99))
    );
}

#[tokio::test]
async fn sqlite_account_controls_follow_their_storage_contract() {
    let fixture = test_support::sqlite().await;
    state_contract(&fixture.store).await;
    raise_contract(&fixture.store).await;
    assert!(
        fixture
            .store
            .account_native_machines(ACCOUNT)
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
#[ignore = "run nix develop -c just test-postgres (owns an isolated database)"]
async fn postgres_account_controls_follow_their_storage_contract() {
    let fixture = test_support::postgres().await;
    state_contract(&fixture.store).await;
    raise_contract(&fixture.store).await;
    assert!(
        fixture
            .store
            .account_native_machines(ACCOUNT)
            .await
            .unwrap()
            .is_empty()
    );
}
