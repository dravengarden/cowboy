// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;
use proto::cowboy_worktree_attempt_response::Outcome as Reply;

fn peer(id: u32) -> proto::PeerId {
    proto::PeerId { owner_id: 0, id }
}

#[test]
fn cowboy_worktree_attempts_keep_kind_and_original_id_and_drops_are_uncertain() {
    let state = State::default();
    for (message, kind) in [
        (1, Kind::Add),
        (2, Kind::Trust),
        (3, Kind::Restrict),
        (4, Kind::Remove),
    ] {
        let attempt = state.begin(peer(1), message, kind).unwrap();
        assert_eq!(
            state.result(peer(1), message).outcome,
            Reply::Pending as i32
        );
        assert_eq!(
            state.result(peer(2), message).outcome,
            Reply::Unknown as i32
        );
        attempt.complete(77);
        let result = state.result(peer(1), message);
        assert_eq!(
            (result.kind, result.outcome, result.worktree_id),
            (kind as i32, Reply::Completed as i32, 77)
        );
        assert!(state.begin(peer(1), message, kind).is_err());
    }
    drop(state.begin(peer(1), 5, Kind::Add).unwrap());
    assert_eq!(state.result(peer(1), 5).outcome, Reply::Uncertain as i32);
    assert!(state.begin(peer(1), 0, Kind::Add).is_err());
    assert!(state.begin(peer(1), 6, Kind::Unspecified).is_err());
}

#[test]
fn cowboy_worktree_attempts_never_evict_pending_and_consume_capacity_refusals() {
    let state = State::default();
    let pending: Vec<_> = (1..=CAPACITY as u32)
        .map(|id| state.begin(peer(1), id, Kind::Add).unwrap())
        .collect();
    assert!(
        state
            .begin(peer(1), CAPACITY as u32 + 1, Kind::Add)
            .is_err()
    );
    assert_eq!(state.result(peer(1), 1).outcome, Reply::Pending as i32);
    drop(pending);
    assert!(
        state
            .begin(peer(1), CAPACITY as u32 + 1, Kind::Add)
            .is_err()
    );
    state
        .begin(peer(1), CAPACITY as u32 + 2, Kind::Add)
        .unwrap()
        .complete(99);
    assert_eq!(state.result(peer(1), 1).outcome, Reply::Forgotten as i32);
    assert_eq!(state.result(peer(1), 2).outcome, Reply::Uncertain as i32);
    assert_eq!(state.0.lock().entries.len(), CAPACITY);
}

#[test]
fn cowboy_worktree_attempts_bound_peer_history_without_losing_old_ids() {
    let state = State::default();
    for id in 1..=PEERS as u32 {
        state.begin(peer(id), 1, Kind::Remove).unwrap().complete(77);
    }
    assert!(state.begin(peer(PEERS as u32 + 1), 1, Kind::Add).is_err());
    assert!(state.begin(peer(1), 1, Kind::Add).is_err());
    assert_eq!(state.result(peer(1), 1).kind, Kind::Remove as i32);
    assert_eq!(state.0.lock().peers.len(), PEERS);
}
