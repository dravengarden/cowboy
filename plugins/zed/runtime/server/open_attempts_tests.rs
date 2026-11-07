// SPDX-License-Identifier: GPL-3.0-or-later
use super::*;

fn peer(id: u32) -> proto::PeerId {
    proto::PeerId { owner_id: 0, id }
}

#[test]
fn cowboy_open_attempts_settle_once_and_fail_when_dropped() {
    let state = State::default();
    let opened = state.begin(peer(1), 7).unwrap();
    assert_eq!(state.outcome(peer(1), 7), Some(Outcome::Pending));
    // Another peer's identical message ID is a different attempt.
    assert_eq!(state.outcome(peer(2), 7), None);
    opened.opened(42);
    assert_eq!(state.outcome(peer(1), 7), Some(Outcome::Opened(42)));
    {
        let _lost = state.begin(peer(1), 8).unwrap();
    }
    assert_eq!(state.outcome(peer(1), 8), Some(Outcome::Failed));
    assert_eq!(state.outcome(peer(1), 9), None);
}

#[test]
fn cowboy_open_attempts_are_bounded_and_keep_pending_ones() {
    let state = State::default();
    let pending = state.begin(peer(1), 1).unwrap();
    for id in 2..=MAX_ATTEMPTS as u32 + 1 {
        state.begin(peer(1), id).unwrap().opened(u64::from(id));
    }
    assert_eq!(state.outcome(peer(1), 1), Some(Outcome::Pending));
    // The oldest settled outcome is forgotten first.
    assert_eq!(state.outcome(peer(1), 2), Some(Outcome::Forgotten));
    assert_eq!(state.outcome(peer(1), 3), Some(Outcome::Opened(3)));
    pending.opened(1);
    assert_eq!(state.outcome(peer(1), 1), Some(Outcome::Opened(1)));
    assert!(state.begin(peer(1), 2).is_err());
    assert_eq!(state.outcome(peer(2), 2), None);
}

#[test]
fn cowboy_open_attempts_refuse_overflow_without_evicting_pending_or_reusing_ids() {
    let state = State::default();
    let pending: Vec<_> = (1..=MAX_ATTEMPTS as u32)
        .map(|id| state.begin(peer(1), id).unwrap())
        .collect();
    assert!(state.begin(peer(1), MAX_ATTEMPTS as u32 + 1).is_err());
    assert_eq!(state.outcome(peer(1), 1), Some(Outcome::Pending));
    assert_eq!(state.outcome(peer(1), MAX_ATTEMPTS as u32 + 1), None);
    assert!(state.begin(peer(1), 1).is_err());
    drop(pending);
    assert_eq!(state.outcome(peer(1), 1), Some(Outcome::Failed));
    assert!(state.begin(peer(1), 1).is_err());
    assert!(state.begin(peer(1), MAX_ATTEMPTS as u32 + 1).is_err());
    state
        .begin(peer(1), MAX_ATTEMPTS as u32 + 2)
        .unwrap()
        .opened(42);
    assert_eq!(state.outcome(peer(1), 1), Some(Outcome::Forgotten));
    assert!(state.begin(peer(1), 1).is_err());
}

#[test]
fn cowboy_open_attempts_bound_peer_history_without_forgetting_old_peers() {
    let state = State::default();
    for id in 1..=MAX_PEERS as u32 {
        state.begin(peer(id), 1).unwrap().opened(u64::from(id));
    }
    assert!(state.begin(peer(MAX_PEERS as u32 + 1), 1).is_err());
    assert_eq!(state.outcome(peer(MAX_PEERS as u32 + 1), 1), None);
    state.begin(peer(1), 2).unwrap().opened(42);
    assert_eq!(state.outcome(peer(1), 1), Some(Outcome::Forgotten));
    assert!(state.begin(peer(1), 1).is_err());
    assert_eq!(state.0.lock().peers.len(), MAX_PEERS);
}

#[gpui::test]
async fn cowboy_open_target_reports_what_an_open_would_return(cx: &mut gpui::TestAppContext) {
    use fs::FakeFs;
    use util::rel_path::rel_path;
    cx.update(|cx| {
        if cx.try_global::<settings::SettingsStore>().is_none() {
            let settings = settings::SettingsStore::test(cx);
            cx.set_global(settings);
        }
    });
    let fs = FakeFs::new(cx.executor());
    fs.insert_tree("/root", serde_json::json!({"a": "a\n", "b": "b\n"}))
        .await;
    let trees = cx.new(|_| WorktreeStore::local(true, fs.clone(), Default::default()));
    let store = cx.new(|cx| BufferStore::local(trees.clone(), cx));
    let (tree, _) = trees
        .update(cx, |trees, cx| {
            trees.find_or_create_worktree("/root", true, cx)
        })
        .await
        .unwrap();
    let path = |name: &str| ProjectPath {
        worktree_id: tree.read_with(cx, |tree, _| tree.id()),
        path: rel_path(name).into(),
    };
    let (a, b) = (path("a"), path("b"));
    assert_eq!(
        store.read_with(cx, |s, _| s.cowboy_open_target(&a)),
        Target::NotOpen
    );
    let loading = store.update(cx, |s, cx| s.open_buffer(a.clone(), cx));
    assert_eq!(
        store.read_with(cx, |s, _| s.cowboy_open_target(&a)),
        Target::Loading
    );
    let buffer = loading.await.unwrap();
    let id = buffer.read_with(cx, |buffer, _| buffer.remote_id().to_proto());
    assert_eq!(
        store.read_with(cx, |s, _| s.cowboy_open_target(&a)),
        Target::Open(id)
    );
    assert_eq!(
        store.read_with(cx, |s, _| s.cowboy_open_target(&b)),
        Target::NotOpen
    );
    // The query matches the open: the same path returns the same buffer.
    let again = store
        .update(cx, |s, cx| s.open_buffer(a.clone(), cx))
        .await
        .unwrap();
    assert_eq!(again, buffer);
}
