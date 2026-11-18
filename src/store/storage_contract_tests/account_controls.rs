//! Account controls (`crate::account_controls`) against a real store of each
//! backend: revision-checked state changes with their fences in one
//! transaction, the atomic single-turn reservation and its state fences.

use super::test_support;
use crate::account_controls::{AccountState, FenceState, Reservation, ReservationState};
use crate::provider_account::AccountFenceAction;
use crate::store::Store;
use crate::store::account_controls::{Settle, Settled, StateChange, TurnKey};

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
        delivery_id: String::new(),
        worker_epoch: String::new(),
        machine_id: String::new(),
        after_seq: 7,
        state: ReservationState::Reserved,
        created_at_ms: 1,
        updated_at_ms: 1,
        terminal: None,
    }
}

fn key<'a>(session: &'a str, turn: &'a str, epoch: &'a str) -> TurnKey<'a> {
    TurnKey {
        session_id: session,
        turn_id: turn,
        worker_epoch: epoch,
        only: None,
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
    // Only the holder's turn is marked (matching account, session and its
    // Controller-generated ID), and only from `reserved`.
    assert!(
        !store
            .mark_account_turn_dispatched(ACCOUNT, &key("sess-b", "t2", "e"), ("c", "ovh"), 2)
            .await
            .unwrap()
    );
    // Another session naming the holder's turn ID claims nothing.
    assert!(
        !store
            .mark_account_turn_dispatched(ACCOUNT, &key("sess-b", "t1", "e"), ("c", "ovh"), 2)
            .await
            .unwrap()
    );
    assert!(
        store
            .mark_account_turn_dispatched(ACCOUNT, &key("sess-a", "t1", "epoch"), ("c1", "ovh"), 2)
            .await
            .unwrap()
    );
    assert!(
        !store
            .mark_account_turn_dispatched(ACCOUNT, &key("sess-a", "t1", "epoch"), ("c1", "ovh"), 3)
            .await
            .unwrap()
    );
    let stored = store.account_turns().await.unwrap();
    assert_eq!(stored[0].state, ReservationState::MayHaveDispatched);
    assert_eq!(
        (
            stored[0].worker_epoch.as_str(),
            stored[0].delivery_id.as_str(),
            stored[0].machine_id.as_str(),
            stored[0].after_seq
        ),
        ("epoch", "c1", "ovh", 7)
    );
    terminal_contract(store).await;
}

/// A terminal record lands only on exactly its delivery and worker epoch;
/// the turn is then released only by its own session.
async fn terminal_contract(store: &Store) {
    for (session, delivery, epoch) in [
        ("sess-b", "c1", "epoch"),
        ("sess-a", "c2", "epoch"),
        ("sess-a", "c1", "older-epoch"),
    ] {
        assert!(
            !store
                .record_account_turn_terminal(session, delivery, epoch, "EndTurn", 4)
                .await
                .unwrap()
        );
    }
    assert!(store.account_turns().await.unwrap()[0].terminal.is_none());
    assert!(
        store
            .record_account_turn_terminal("sess-a", "c1", "epoch", "EndTurn", 4)
            .await
            .unwrap()
    );
    // The first terminal record of the delivery is kept.
    assert!(
        !store
            .record_account_turn_terminal("sess-a", "c1", "epoch", "Cancelled", 5)
            .await
            .unwrap()
    );
    let terminal = store.account_turns().await.unwrap()[0]
        .terminal
        .clone()
        .unwrap();
    assert_eq!(
        (
            terminal.worker_epoch.as_str(),
            terminal.stop_reason.as_str()
        ),
        ("epoch", "EndTurn")
    );
    // A may-have-dispatched turn never settles as not dispatched, and
    // another session never releases it.
    let mut reserved_only = key("sess-a", "t1", "");
    reserved_only.only = Some(ReservationState::Reserved);
    assert!(
        !store
            .release_account_turn(ACCOUNT, &reserved_only)
            .await
            .unwrap()
    );
    assert!(
        !store
            .release_account_turn(ACCOUNT, &key("sess-b", "t1", ""))
            .await
            .unwrap()
    );
    assert!(
        store
            .release_account_turn(ACCOUNT, &key("sess-a", "t1", ""))
            .await
            .unwrap()
    );
    let next = store
        .reserve_account_turn(&turn("sess-b", "t2"))
        .await
        .unwrap();
    assert_eq!(next.session_id, "sess-b");
    let mut reserved_only = key("sess-b", "t2", "");
    reserved_only.only = Some(ReservationState::Reserved);
    assert!(
        store
            .release_account_turn(ACCOUNT, &reserved_only)
            .await
            .unwrap()
    );
    assert!(store.account_turns().await.unwrap().is_empty());
}

fn settle<'a>(
    turn: Option<TurnKey<'a>>,
    state: Option<(AccountState, &'a str, Option<i64>)>,
    stop_units: bool,
    machines: &'a [String],
) -> Settle<'a> {
    Settle {
        account_id: ACCOUNT,
        owner_user_id: "owner",
        provider_id: "antigravity",
        turn,
        release: true,
        state,
        stop_units,
        machines,
        now_ms: 20,
    }
}

/// Settlement is one transaction: the reservation must still be the same
/// turn, a more severe state (above all the owner's block) is kept, and the
/// stop fences and the release commit with the state.
async fn settlement_contract(store: &Store) {
    let machines = vec!["ovh".to_owned()];
    store
        .reserve_account_turn(&turn("sess-a", "t1"))
        .await
        .unwrap();
    assert!(
        store
            .mark_account_turn_dispatched(ACCOUNT, &key("sess-a", "t1", "e1"), ("c1", "ovh"), 2)
            .await
            .unwrap()
    );
    // A different epoch is another turn's settlement: nothing is written.
    let stale = store
        .settle_account(&settle(
            Some(key("sess-a", "t1", "e0")),
            Some((AccountState::UnknownRefusal, "unknown_refusal", None)),
            false,
            &machines,
        ))
        .await
        .unwrap();
    assert_eq!(stale, Settled::Stale);
    assert_eq!(store.account_turns().await.unwrap().len(), 1);
    let before = store.account_control(ACCOUNT).await.unwrap().unwrap();
    assert_eq!((before.state, before.revision), (AccountState::Ok, 2));
    // The owner blocks meanwhile; an automatic settlement keeps the block
    // but still queues its stop fences and frees the slot.
    store
        .set_account_state(&change(AccountState::Blocked, Some(2), None))
        .await
        .unwrap()
        .unwrap();
    let applied = store
        .settle_account(&settle(
            Some(key("sess-a", "t1", "e1")),
            Some((
                AccountState::NeedsAttention,
                "policy_violation:read_file",
                None,
            )),
            true,
            &machines,
        ))
        .await
        .unwrap();
    assert_eq!(
        applied,
        Settled::Applied {
            state_written: true,
            released: true
        }
    );
    let control = store.account_control(ACCOUNT).await.unwrap().unwrap();
    assert_eq!(control.state, AccountState::Blocked);
    assert_eq!(control.revision, 4);
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
    assert!(store.account_turns().await.unwrap().is_empty());
    // A lower automatic state never replaces a more severe one.
    store
        .set_account_state(&change(AccountState::Suspended, Some(4), None))
        .await
        .unwrap()
        .unwrap();
    let applied = store
        .settle_account(&settle(
            None,
            Some((AccountState::RateLimited, "rate_limited", Some(99))),
            false,
            &machines,
        ))
        .await
        .unwrap();
    assert_eq!(
        applied,
        Settled::Applied {
            state_written: false,
            released: false
        }
    );
    let control = store.account_control(ACCOUNT).await.unwrap().unwrap();
    assert_eq!(
        (control.state, control.revision),
        (AccountState::Suspended, 5)
    );
}

#[tokio::test]
async fn sqlite_account_controls_follow_their_storage_contract() {
    let fixture = test_support::sqlite().await;
    state_contract(&fixture.store).await;
    reservation_contract(&fixture.store).await;
    settlement_contract(&fixture.store).await;
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
    settlement_contract(&fixture.store).await;
    assert!(
        fixture
            .store
            .account_native_machines(ACCOUNT)
            .await
            .unwrap()
            .is_empty()
    );
}
