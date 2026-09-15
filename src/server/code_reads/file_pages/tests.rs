use super::*;
use crate::core::{Hub, SessionOrigin, Status};

mod http;
mod transport;

fn create(hub: &Hub, id: &str) -> CodeReadScope {
    hub.create_local_session(
        id.into(),
        "codex".into(),
        "/work/a".into(),
        "title".into(),
        SessionOrigin::default(),
        false,
    );
    scope(hub, id)
}

fn scope(hub: &Hub, id: &str) -> CodeReadScope {
    CodeReadScope::Session(hub.session_code_scope(id).unwrap())
}

fn page(path: &str, offset: usize, more: bool) -> FileDocument {
    FileDocument {
        path: path.into(),
        revision: "a".repeat(64),
        text: "界ab".into(),
        size: if more { 100 } else { (offset + 5) as u64 },
        truncated: more,
        next_cursor: more.then(|| format!("{}:{}", "a".repeat(64), offset + 5)),
        limited: false,
    }
}

fn token(cache: &PageCursors, owner: &CodeReadScope, path: &str) -> String {
    cache
        .project(owner, path, None, page(path, 0, true))
        .unwrap()
        .next_cursor
        .unwrap()
}

#[test]
fn opaque_cursors_bind_exact_path_session_and_hub() {
    let cache = PageCursors::default();
    let hub = Hub::new();
    let owner = create(&hub, "one");
    let other = create(&hub, "two");
    let foreign = create(&Hub::new(), "one");
    let public = token(&cache, &owner, "a.txt");
    let next = cache
        .resolve(&owner, "a.txt", Some(&public))
        .unwrap()
        .unwrap();
    assert_eq!(next.native_cursor(), format!("{}:5", "a".repeat(64)));
    assert_ne!(next.native_cursor(), public);
    assert!(cache.resolve(&owner, "a.txt", None).unwrap().is_none());
    for (wrong_owner, wrong_path) in [(&other, "a.txt"), (&foreign, "a.txt"), (&owner, "b.txt")] {
        assert_eq!(
            cache
                .resolve(wrong_owner, wrong_path, Some(&public))
                .unwrap_err(),
            CursorError::Expired
        );
    }
    assert_eq!(
        cache
            .resolve(&owner, "a.txt", Some(next.native_cursor()))
            .unwrap_err(),
        CursorError::Expired
    );
    let changed_offset = format!("{}:6", public.split_once(':').unwrap().0);
    assert_eq!(
        cache
            .resolve(&owner, "a.txt", Some(&changed_offset))
            .unwrap_err(),
        CursorError::Expired
    );
    assert_ne!(token(&cache, &owner, "b.txt"), public);
}

#[test]
fn unrelated_metadata_is_stable_but_cwd_aba_and_recreation_never_adopt_cursors() {
    let cache = PageCursors::default();
    let hub = Hub::new();
    let owner = create(&hub, "one");
    let public = token(&cache, &owner, "a.txt");
    hub.rename_session("one", "renamed".into());
    hub.set_status("one", Status::Running, None);
    hub.update_session_cwd("one", "/work/a".into()).unwrap();
    assert_eq!(token(&cache, &scope(&hub, "one"), "a.txt"), public);
    hub.update_session_cwd("one", "/work/b".into()).unwrap();
    hub.update_session_cwd("one", "/work/a".into()).unwrap();
    assert_eq!(
        cache
            .resolve(&scope(&hub, "one"), "a.txt", Some(&public))
            .unwrap_err(),
        CursorError::Expired
    );
    assert!(hub.delete_session("one"));
    let recreated = create(&hub, "one");
    assert_eq!(
        cache
            .resolve(&recreated, "a.txt", Some(&public))
            .unwrap_err(),
        CursorError::Expired
    );
}

#[test]
fn every_advertised_workspace_identity_axis_is_required() {
    let cache = PageCursors::default();
    let owner = CodeReadScope::Workspace {
        service_id: "service".into(),
        machine_id: "machine".into(),
        workspace_id: "workspace".into(),
        cwd: "/work".into(),
    };
    let public = token(&cache, &owner, "a.txt");
    for axis in 0..4 {
        let mut changed = owner.clone();
        let CodeReadScope::Workspace {
            service_id,
            machine_id,
            workspace_id,
            cwd,
        } = &mut changed
        else {
            unreachable!()
        };
        [service_id, machine_id, workspace_id, cwd][axis].push_str("-changed");
        assert_eq!(
            cache.resolve(&changed, "a.txt", Some(&public)).unwrap_err(),
            CursorError::Expired
        );
    }
}

#[test]
fn expiry_eviction_and_new_controller_state_cannot_revive_a_token() {
    let cache = PageCursors {
        max_entries: 1,
        ..PageCursors::default()
    };
    let owner = create(&Hub::new(), "one");
    let old = token(&cache, &owner, "a.txt");
    assert_eq!(token(&cache, &owner, "a.txt"), old);
    token(&cache, &owner, "b.txt");
    assert_eq!(
        cache.resolve(&owner, "a.txt", Some(&old)).unwrap_err(),
        CursorError::Expired
    );
    let replacement = token(&cache, &owner, "a.txt");
    assert_ne!(replacement, old);
    cache.entries.lock().front_mut().unwrap().touched = Instant::now() - TTL;
    assert_eq!(
        cache
            .resolve(&owner, "a.txt", Some(&replacement))
            .unwrap_err(),
        CursorError::Expired
    );
    let next = token(&cache, &owner, "a.txt");
    assert_ne!(next, replacement);
    assert_eq!(
        PageCursors::default()
            .resolve(&owner, "a.txt", Some(&next))
            .unwrap_err(),
        CursorError::Expired
    );
}

#[test]
fn count_identity_byte_limits_and_lru_are_enforced() {
    let cache = PageCursors {
        max_entries: 2,
        ..PageCursors::default()
    };
    let owner = create(&Hub::new(), "one");
    let first = token(&cache, &owner, "a.txt");
    let second = token(&cache, &owner, "b.txt");
    cache.resolve(&owner, "a.txt", Some(&first)).unwrap();
    token(&cache, &owner, "c.txt");
    assert!(cache.resolve(&owner, "a.txt", Some(&first)).is_ok());
    assert_eq!(
        cache.resolve(&owner, "b.txt", Some(&second)).unwrap_err(),
        CursorError::Expired
    );
    assert_eq!(cache.entries.lock().len(), 2);

    let bounded = PageCursors {
        max_string_bytes: 400,
        ..PageCursors::default()
    };
    for index in 0..20 {
        token(&bounded, &owner, &format!("{index}.txt"));
    }
    assert!(
        bounded
            .entries
            .lock()
            .iter()
            .map(|entry| entry.string_bytes)
            .sum::<usize>()
            <= 400
    );
    let large_owner = CodeReadScope::Workspace {
        service_id: "service".into(),
        machine_id: "machine".into(),
        workspace_id: "workspace".into(),
        cwd: "x".repeat(401),
    };
    assert_eq!(
        bounded
            .project(&large_owner, "a.txt", None, page("a.txt", 0, true))
            .unwrap_err(),
        CursorError::Capacity
    );
    assert_eq!(
        bounded
            .project(&owner, &"x".repeat(401), None, page("a.txt", 0, true))
            .unwrap_err(),
        CursorError::Capacity
    );
    let disabled = PageCursors {
        max_entries: 0,
        ..PageCursors::default()
    };
    assert_eq!(
        disabled
            .project(&owner, "a.txt", None, page("a.txt", 0, true))
            .unwrap_err(),
        CursorError::Capacity
    );
}

#[test]
fn malformed_public_tokens_never_become_adapter_input() {
    let cache = PageCursors::default();
    let owner = create(&Hub::new(), "one");
    for bad in [
        "".into(),
        "x:1".into(),
        format!("{}:0", "a".repeat(64)),
        format!("{}:+1", "a".repeat(64)),
        format!("{}:01", "a".repeat(64)),
        format!("{}:{MAX_FILE_BYTES}", "a".repeat(64)),
        "x".repeat(10_000),
    ] {
        assert_eq!(
            cache.resolve(&owner, "a.txt", Some(&bad)).unwrap_err(),
            CursorError::Invalid
        );
    }
    assert!(cache.entries.lock().is_empty());
}

#[test]
fn invalid_or_nonprogressing_backend_pages_are_not_issued() {
    let cache = PageCursors::default();
    let owner = create(&Hub::new(), "one");
    for change in 0..12 {
        let mut invalid = page("a.txt", 0, true);
        match change {
            0 => invalid.next_cursor = Some(format!("{}:5", "b".repeat(64))),
            1 => invalid.next_cursor = Some(format!("{}:6", "a".repeat(64))),
            2 => invalid.text.clear(),
            3 => invalid.size = 4,
            4 => invalid.size = 5,
            5 => invalid.next_cursor = None,
            6 => invalid.truncated = false,
            7 => invalid.text = "x".repeat(FILE_PAGE_BYTES + 1),
            8 => invalid.revision = "z".repeat(64),
            9 => invalid.revision = "a".repeat(63),
            10 => invalid.limited = true,
            11 => invalid.size = MAX_FILE_BYTES as u64 + 1,
            _ => unreachable!(),
        }
        assert_eq!(
            cache.project(&owner, "a.txt", None, invalid).unwrap_err(),
            CursorError::InvalidPage
        );
    }
    assert!(cache.entries.lock().is_empty());
}

#[test]
fn a_resolved_continuation_cannot_switch_context_path_or_revision() {
    let cache = PageCursors::default();
    let hub = Hub::new();
    let owner = create(&hub, "one");
    let other = create(&hub, "two");
    let public = token(&cache, &owner, "a.txt");
    let continuation = cache
        .resolve(&owner, "a.txt", Some(&public))
        .unwrap()
        .unwrap();
    let next_page = page("a.txt", 5, true);
    assert_eq!(
        cache
            .project(&other, "a.txt", Some(&continuation), next_page.clone())
            .unwrap_err(),
        CursorError::Expired
    );
    assert_eq!(
        cache
            .project(&owner, "b.txt", Some(&continuation), next_page.clone())
            .unwrap_err(),
        CursorError::Expired
    );
    let mut changed = next_page;
    changed.revision = "b".repeat(64);
    assert_eq!(
        cache
            .project(&owner, "a.txt", Some(&continuation), changed)
            .unwrap_err(),
        CursorError::Changed
    );
    assert_eq!(
        cache
            .project(&owner, "a.txt", Some(&continuation), page("a.txt", 0, true))
            .unwrap_err(),
        CursorError::InvalidPage
    );
    let last = cache
        .project(
            &owner,
            "a.txt",
            Some(&continuation),
            page("a.txt", 5, false),
        )
        .unwrap();
    assert!(last.next_cursor.is_none());
}

#[test]
fn empty_files_and_limited_utf8_tails_have_closed_terminal_pages() {
    let cache = PageCursors::default();
    let owner = create(&Hub::new(), "one");
    let empty = FileDocument {
        path: "empty.txt".into(),
        revision: "a".repeat(64),
        text: String::new(),
        size: 0,
        truncated: false,
        next_cursor: None,
        limited: false,
    };
    assert_eq!(
        cache
            .project(&owner, "empty.txt", None, empty.clone())
            .unwrap(),
        empty
    );
    let mut invalid = empty;
    invalid.truncated = true;
    assert_eq!(
        cache
            .project(&owner, "empty.txt", None, invalid)
            .unwrap_err(),
        CursorError::InvalidPage
    );

    let previous = Continuation {
        owner: owner.clone(),
        path: "large.txt".into(),
        native: NativeCursor::parse(&format!(
            "{}:{}",
            "a".repeat(64),
            MAX_FILE_BYTES - FILE_PAGE_BYTES
        ))
        .unwrap(),
    };
    for trimmed in 0..=4 {
        let last = FileDocument {
            path: "large.txt".into(),
            revision: "a".repeat(64),
            text: "x".repeat(FILE_PAGE_BYTES - trimmed),
            size: MAX_FILE_BYTES as u64 + 100,
            truncated: true,
            next_cursor: None,
            limited: true,
        };
        let result = cache.project(&owner, "large.txt", Some(&previous), last);
        if trimmed < 4 {
            assert!(result.unwrap().next_cursor.is_none());
        } else {
            assert_eq!(result.unwrap_err(), CursorError::InvalidPage);
        }
    }
    assert!(cache.entries.lock().is_empty());
}

#[test]
fn concurrent_first_pages_share_one_live_binding() {
    let cache = PageCursors::default();
    let owner = create(&Hub::new(), "one");
    let barrier = std::sync::Barrier::new(8);
    let tokens = std::thread::scope(|threads| {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                threads.spawn(|| {
                    barrier.wait();
                    token(&cache, &owner, "a.txt")
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(tokens.iter().all(|token| token == &tokens[0]));
    assert_eq!(cache.entries.lock().len(), 1);
}
