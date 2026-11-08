use super::*;

fn reconciled() -> Journal {
    let mut journal = Journal::new().unwrap();
    journal.finish_scan(journal.begin_scan().unwrap()).unwrap();
    journal
}

#[test]
fn initial_and_replaced_streams_require_reset_and_never_reuse_cursors() {
    let first = Journal::new().unwrap();
    assert_eq!(first.read(None).unwrap().state, PageState::Reset);
    assert_eq!(
        first.read(Some(&first.head())).unwrap().state,
        PageState::Gap
    );
    let second = reconciled();
    assert_eq!(
        second.read(Some(&first.head())).unwrap().state,
        PageState::Reset
    );
    assert_ne!(first.head().instance, second.head().instance);
}

#[test]
fn every_page_declares_event_only_coverage_even_after_reconciliation() {
    let mut journal = reconciled();
    let before = journal.head();
    for cursor in [None, Some(&before)] {
        let page = journal.read(cursor).unwrap();
        assert_eq!(page.coverage, Coverage::EventStreamOnly);
        assert_eq!(
            serde_json::to_value(page).unwrap()["coverage"],
            "event_stream_only"
        );
    }
    journal.record(vec!["file".into()]).unwrap();
    assert_eq!(
        journal.read(Some(&before)).unwrap().coverage,
        Coverage::EventStreamOnly
    );
    journal.gap(Gap::Coverage);
    assert_eq!(
        journal.read(Some(&before)).unwrap().coverage,
        Coverage::EventStreamOnly
    );
}

#[test]
fn gaps_invalidate_unchanged_files_and_old_scans_cannot_clear_them() {
    let mut journal = reconciled();
    let before = journal.head();
    let scan = journal.begin_scan().unwrap();
    journal.gap(Gap::QueueOverflow);
    assert!(journal.finish_scan(scan).is_err());
    assert_eq!(journal.read(Some(&before)).unwrap().state, PageState::Gap);
    journal.finish_scan(journal.begin_scan().unwrap()).unwrap();
    assert_eq!(journal.read(Some(&before)).unwrap().state, PageState::Gap);
    assert_eq!(
        journal.read(Some(&journal.head())).unwrap().state,
        PageState::Changes
    );
}

#[test]
fn every_change_during_scan_refuses_reconciliation_including_ignored_inputs() {
    let mut journal = Journal::new().unwrap();
    let scan = journal.begin_scan().unwrap();
    journal
        .record(vec![
            "target/generated.rs".into(),
            "node_modules/pkg/index.js".into(),
        ])
        .unwrap();
    assert!(journal.finish_scan(scan).is_err());
    assert_eq!(
        journal.read(Some(&journal.head())).unwrap().state,
        PageState::Gap
    );
    journal.finish_scan(journal.begin_scan().unwrap()).unwrap();
    assert_eq!(
        journal.read(Some(&journal.head())).unwrap().state,
        PageState::Changes
    );
}

#[test]
fn low_water_and_pages_do_not_turn_eviction_into_an_empty_clean_result() {
    let mut journal = reconciled();
    let before = journal.head();
    for n in 0..MAX_EVENTS + 10 {
        journal.record(vec![format!("file-{n}")]).unwrap();
    }
    assert!(journal.entries.len() <= MAX_EVENTS && journal.bytes <= MAX_BYTES);
    assert_eq!(journal.read(Some(&before)).unwrap().state, PageState::Gap);
    let mut cursor = journal.cursor(journal.low_water);
    let mut seen = 0;
    loop {
        let page = journal.read(Some(&cursor)).unwrap();
        assert_eq!(page.state, PageState::Changes);
        assert!(page.events.len() <= PAGE_EVENTS);
        seen += page.events.len();
        cursor = page.through;
        if !page.more {
            break;
        }
    }
    assert_eq!(seen, MAX_EVENTS);
    assert_eq!(cursor, journal.head());
}

#[test]
fn byte_budget_invalid_paths_and_counter_exhaustion_fail_closed() {
    let mut journal = reconciled();
    let original = journal.head();
    for n in 0..200 {
        journal
            .record(
                (0..3)
                    .map(|i| format!("{n}/{i}/{}", "x".repeat(4000)))
                    .collect(),
            )
            .unwrap();
    }
    assert!(journal.bytes <= MAX_BYTES);
    assert!(journal.low_water > 0);
    assert_eq!(journal.read(Some(&original)).unwrap().state, PageState::Gap);
    let page = journal
        .read(Some(&journal.cursor(journal.low_water)))
        .unwrap();
    assert!(page.more && page.events.len() == 1);
    assert!(serde_json::to_vec(&page).unwrap().len() < 128 * 1024);
    for path in [
        "",
        "/outside",
        "../outside",
        "a/../b",
        "a\\b",
        "nul\0path",
        "a//b",
        "a/.",
    ] {
        let before = journal.head();
        assert!(journal.record(vec![path.into()]).is_err());
        assert_eq!(journal.read(Some(&before)).unwrap().state, PageState::Gap);
    }
    journal.position = u64::MAX;
    assert!(journal.record(vec!["file".into()]).is_err());
    assert!(journal.begin_scan().is_err());
    assert_eq!(
        journal.read(Some(&journal.head())).unwrap().state,
        PageState::Gap
    );
    assert_eq!(journal.unresolved, Some(Gap::Exhausted));
}
