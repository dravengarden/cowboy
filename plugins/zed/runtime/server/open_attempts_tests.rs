// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

fn peer(id: u32) -> proto::PeerId {
    proto::PeerId { owner_id: 0, id }
}

#[test]
fn cowboy_open_attempts_settle_once_and_fail_when_dropped() {
    let state = State::default();
    let opened = state.begin(peer(1), 7);
    assert_eq!(state.outcome(peer(1), 7), Some(Outcome::Pending));
    // Another peer's identical message ID is a different attempt.
    assert_eq!(state.outcome(peer(2), 7), None);
    opened.opened(42);
    assert_eq!(state.outcome(peer(1), 7), Some(Outcome::Opened(42)));
    {
        let _lost = state.begin(peer(1), 8);
    }
    assert_eq!(state.outcome(peer(1), 8), Some(Outcome::Failed));
    assert_eq!(state.outcome(peer(1), 9), None);
}

#[test]
fn cowboy_open_attempts_are_bounded_and_keep_pending_ones() {
    let state = State::default();
    let pending = state.begin(peer(1), 1);
    for id in 2..=MAX_ATTEMPTS as u32 + 1 {
        state.begin(peer(1), id).opened(u64::from(id));
    }
    assert_eq!(state.outcome(peer(1), 1), Some(Outcome::Pending));
    // The oldest settled outcome is forgotten first.
    assert_eq!(state.outcome(peer(1), 2), None);
    assert_eq!(state.outcome(peer(1), 3), Some(Outcome::Opened(3)));
    pending.opened(1);
    assert_eq!(state.outcome(peer(1), 1), Some(Outcome::Opened(1)));
}
