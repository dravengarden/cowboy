//! Account controls (`crate::account_controls`) against a real store of each
//! backend: revision-checked state changes with their fences in one
//! transaction, the atomic single-turn reservation and its state fences.

use super::test_support;
use crate::account_controls::{AccountState, FenceState, Reservation, ReservationState};
use crate::provider_account::AccountFenceAction;
use crate::store::Store;
use crate::store::account_controls::StateChange;

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

fn turn(session: &str, turn_id: &str) -> Reservation {
    Reservation {
        account_id: ACCOUNT.into(),
        owner_user_id: "owner".into(),
        session_id: session.into(),
        turn_id: turn_id.into(),
        worker_epoch: String::new(),
        after_seq: 7,
        state: ReservationState::Reserved,
        created_at_ms: 1,
        updated_at_ms: 1,
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

async fn reservation_contract(store: &Store) {
    let first = store
        .reserve_account_turn(&turn("sess-a", "t1"))
        .await
        .unwrap();
    assert_eq!(
        (first.session_id.as_str(), first.turn_id.as_str()),
        ("sess-a", "t1")
    );
    // Another session's turn finds the slot held.
    let held = store
        .reserve_account_turn(&turn("sess-b", "t2"))
        .await
        .unwrap();
    assert_eq!(
        (held.session_id.as_str(), held.turn_id.as_str()),
        ("sess-a", "t1")
    );
    // Only the holder's turn is marked, and only from `reserved`.
    assert!(
        !store
            .mark_account_turn_dispatched(ACCOUNT, "t2", "e", 2)
            .await
            .unwrap()
    );
    assert!(
        store
            .mark_account_turn_dispatched(ACCOUNT, "t1", "epoch", 2)
            .await
            .unwrap()
    );
    assert!(
        !store
            .mark_account_turn_dispatched(ACCOUNT, "t1", "epoch", 3)
            .await
            .unwrap()
    );
    let stored = store.account_turns().await.unwrap();
    assert_eq!(stored[0].state, ReservationState::MayHaveDispatched);
    assert_eq!(
        (stored[0].worker_epoch.as_str(), stored[0].after_seq),
        ("epoch", 7)
    );
    // A may-have-dispatched turn never settles as not dispatched.
    assert!(
        !store
            .release_account_turn(ACCOUNT, "t1", Some(ReservationState::Reserved))
            .await
            .unwrap()
    );
    assert!(
        store
            .release_account_turn(ACCOUNT, "t1", None)
            .await
            .unwrap()
    );
    let next = store
        .reserve_account_turn(&turn("sess-b", "t2"))
        .await
        .unwrap();
    assert_eq!(next.session_id, "sess-b");
    assert!(
        store
            .release_account_turn(ACCOUNT, "t2", Some(ReservationState::Reserved))
            .await
            .unwrap()
    );
    assert!(store.account_turns().await.unwrap().is_empty());
}

#[tokio::test]
async fn sqlite_account_controls_follow_their_storage_contract() {
    let fixture = test_support::sqlite().await;
    state_contract(&fixture.store).await;
    reservation_contract(&fixture.store).await;
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
    reservation_contract(&fixture.store).await;
    assert!(
        fixture
            .store
            .account_native_machines(ACCOUNT)
            .await
            .unwrap()
            .is_empty()
    );
}
