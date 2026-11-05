
use super::{FileTreeEntry, file_tree_revision};

#[test]
fn revision_is_stable_and_covers_visible_tree_state() {
    let entries = vec![FileTreeEntry {
        name: "src".to_owned(),
        path: "src".to_owned(),
        kind: "directory",
        ignored: false,
    }];
    let revision = file_tree_revision("", &entries, false);
    assert_eq!(revision, file_tree_revision("", &entries, false));
    assert_ne!(revision, file_tree_revision("", &entries, true));
    assert_ne!(revision, file_tree_revision("nested", &entries, false));
    let renamed = vec![FileTreeEntry {
        name: "source".to_owned(),
        path: "source".to_owned(),
        kind: "directory",
        ignored: false,
    }];
    assert_ne!(revision, file_tree_revision("", &renamed, false));
    let ignored = vec![FileTreeEntry {
        name: "src".to_owned(),
        path: "src".to_owned(),
        kind: "directory",
        ignored: true,
    }];
    assert_ne!(revision, file_tree_revision("", &ignored, false));
}
