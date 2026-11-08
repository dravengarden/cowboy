use super::*;
use crate::code_observer::PageState;

fn fixture(root: &Path) -> (Source, Journal) {
    let mut journal = Journal::new().unwrap();
    let source = Source::new(File::open(root).unwrap(), root.into(), &mut journal).unwrap();
    (source, journal)
}

#[test]
fn kernel_changes_include_ignored_inputs_and_outside_hardlink_writes() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("target")).unwrap();
    let file = root.path().join("target/generated.rs");
    std::fs::write(&file, "before").unwrap();
    let outside = tempfile::tempdir().unwrap();
    let link = outside.path().join("alias");
    std::fs::hard_link(&file, &link).unwrap();
    let (mut source, mut journal) = fixture(root.path());
    let before = journal.head();
    std::fs::write(&link, "after").unwrap();
    assert!(source.drain(&mut journal).unwrap() > 0);
    let page = journal.read(Some(&before)).unwrap();
    assert_eq!(page.state, PageState::Changes);
    assert!(
        page.events
            .iter()
            .any(|event| event.paths.iter().any(|path| path == "target/generated.rs"))
    );
    assert!(
        page.events
            .iter()
            .flat_map(|e| &e.paths)
            .all(|path| !path.contains("alias"))
    );
}

#[test]
fn structural_changes_require_new_scan_and_old_comparisons_stay_invalid() {
    let root = tempfile::tempdir().unwrap();
    let (mut source, mut journal) = fixture(root.path());
    let before = journal.head();
    std::fs::create_dir(root.path().join("generated")).unwrap();
    std::fs::write(root.path().join("generated/file"), "new").unwrap();
    assert!(source.drain(&mut journal).is_err());
    assert_eq!(journal.read(Some(&before)).unwrap().state, PageState::Gap);
    let mut replacement = Source::new(
        File::open(root.path()).unwrap(),
        root.path().into(),
        &mut journal,
    )
    .unwrap();
    assert_eq!(journal.read(Some(&before)).unwrap().state, PageState::Gap);
    let current = journal.head();
    std::fs::write(root.path().join("generated/file"), "changed").unwrap();
    replacement.drain(&mut journal).unwrap();
    assert_eq!(
        journal.read(Some(&current)).unwrap().state,
        PageState::Changes
    );
}

#[test]
fn root_replacement_never_adopts_the_new_directory() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("root");
    std::fs::create_dir(&root).unwrap();
    let (mut source, mut journal) = fixture(&root);
    let before = journal.head();
    std::fs::rename(&root, parent.path().join("old")).unwrap();
    std::fs::create_dir(&root).unwrap();
    assert!(source.drain(&mut journal).is_err());
    assert_eq!(journal.read(Some(&before)).unwrap().state, PageState::Gap);
    assert_eq!(
        journal.read(Some(&before)).unwrap().reason,
        Some(Gap::RootChanged)
    );
}

#[test]
fn scan_reservation_cannot_hide_an_actual_kernel_change() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("file"), "before").unwrap();
    let (mut source, mut journal) = fixture(root.path());
    let scan = journal.begin_scan().unwrap();
    std::fs::write(root.path().join("file"), "after").unwrap();
    source.drain(&mut journal).unwrap();
    assert!(journal.finish_scan(scan).is_err());
}

#[test]
fn overflow_invalidates_the_window_even_without_a_named_path() {
    let root = tempfile::tempdir().unwrap();
    let (mut source, mut journal) = fixture(root.path());
    let before = journal.head();
    let mut event = Vec::new();
    event.extend_from_slice(&(-1_i32).to_ne_bytes());
    event.extend_from_slice(&libc::IN_Q_OVERFLOW.to_ne_bytes());
    event.extend_from_slice(&0_u32.to_ne_bytes());
    event.extend_from_slice(&0_u32.to_ne_bytes());
    assert_eq!(source.event(&event, &mut journal).unwrap(), 16);
    assert!(source.invalid);
    let page = journal.read(Some(&before)).unwrap();
    assert_eq!(page.state, PageState::Gap);
    assert_eq!(page.reason, Some(Gap::QueueOverflow));
    assert!(source.drain(&mut journal).is_err());
}

#[test]
fn external_symlink_targets_are_never_traversed() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("private"), "outside").unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("link")).unwrap();
    let (mut source, mut journal) = fixture(root.path());
    assert_eq!(source.watches.len(), 1);
    assert!(source.open(Path::new("link/private"), false).is_err());
    let before = journal.head();
    std::fs::remove_file(root.path().join("link")).unwrap();
    assert!(source.drain(&mut journal).is_err());
    assert_eq!(journal.read(Some(&before)).unwrap().state, PageState::Gap);
}

#[test]
fn watch_budget_failure_cannot_clear_an_unreconciled_scan() {
    let root = tempfile::tempdir().unwrap();
    let (mut source, mut journal) = fixture(root.path());
    journal.gap(Gap::Budget);
    let before = journal.head();
    source.path_bytes = MAX_PATH_BYTES;
    std::fs::write(root.path().join("file"), "data").unwrap();
    assert!(source.scan().is_err());
    assert_eq!(journal.read(Some(&before)).unwrap().state, PageState::Gap);
}
