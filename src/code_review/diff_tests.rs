use super::{CodeProvider as _, DiffScope, LocalCodeProvider};
use std::path::Path;
use std::process::Command;

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn repository() -> tempfile::TempDir {
    repository_with_format("sha1")
}

fn repository_with_format(format: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(
        dir.path(),
        &["init", "-q", &format!("--object-format={format}")],
    );
    git(
        dir.path(),
        &["config", "user.email", "cowboy@example.invalid"],
    );
    git(dir.path(), &["config", "user.name", "Cowboy Test"]);
    dir
}

#[test]
fn literal_paths_select_only_the_requested_file_in_every_diff_view() {
    let dir = repository();
    let root = dir.path();
    let paths = [
        "[id].txt",
        "*.txt",
        ":(glob)*.txt",
        "question?.txt",
        "i.txt",
        "question1.txt",
    ];
    for (index, path) in paths.iter().enumerate() {
        std::fs::write(root.join(path), format!("old {index}\n")).unwrap();
    }
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "base"]);
    git(root, &["branch", "base"]);
    for (index, path) in paths.iter().enumerate() {
        std::fs::write(root.join(path), format!("staged {index}\n")).unwrap();
    }
    git(root, &["add", "."]);
    for (index, path) in paths.iter().enumerate() {
        std::fs::write(root.join(path), format!("working {index}\n")).unwrap();
    }
    let provider = LocalCodeProvider::new(root);
    for (index, path) in paths.iter().enumerate() {
        for (scope, before, after, comparison) in [
            (DiffScope::Combined, "old", "working", None),
            (DiffScope::Staged, "old", "staged", None),
            (DiffScope::Unstaged, "staged", "working", None),
            (
                DiffScope::Combined,
                "old",
                "working",
                Some("refs/heads/base"),
            ),
        ] {
            let diff = provider
                .diff_snapshot(path, 3, true, scope, comparison)
                .unwrap();
            assert_eq!(diff.path, *path);
            assert_eq!(
                diff.text.matches("diff --git ").count(),
                1,
                "{path} {scope:?}: {}",
                diff.text
            );
            assert!(
                diff.text.contains(&format!("-{before} {index}\n")),
                "{path} {scope:?}: {}",
                diff.text
            );
            assert!(
                diff.text.contains(&format!("+{after} {index}\n")),
                "{path} {scope:?}: {}",
                diff.text
            );
            assert_eq!((diff.added, diff.removed), (1, 1));
        }
    }
    git(root, &["commit", "-qm", "staged snapshot"]);
    let oid = git(root, &["rev-parse", "HEAD"]);
    for (index, path) in paths.iter().enumerate() {
        let diff = provider.commit_diff(&oid, path).unwrap();
        assert_eq!(
            diff.text.matches("diff --git ").count(),
            1,
            "{path}: {}",
            diff.text
        );
        assert!(diff.text.contains(&format!("-old {index}\n")));
        assert!(diff.text.contains(&format!("+staged {index}\n")));
        assert_eq!((diff.added, diff.removed), (1, 1));
    }
}

#[test]
fn literal_untracked_path_does_not_resolve_to_a_tracked_glob_match() {
    let dir = repository();
    let root = dir.path();
    std::fs::write(root.join("i.txt"), "tracked\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "base"]);
    std::fs::write(root.join("[id].txt"), "untracked\n").unwrap();
    let provider = LocalCodeProvider::new(root);
    for scope in [DiffScope::Combined, DiffScope::Unstaged] {
        let diff = provider
            .diff_snapshot("[id].txt", 3, true, scope, None)
            .unwrap();
        assert!(
            diff.text.contains("+untracked\n"),
            "{scope:?}: {}",
            diff.text
        );
        assert_eq!((diff.added, diff.removed), (1, 0));
    }
}

#[test]
fn unborn_repository_keeps_working_and_staged_snapshots_separate() {
    for format in ["sha1", "sha256"] {
        check_unborn_repository(format);
    }
}

fn check_unborn_repository(format: &str) {
    let dir = repository_with_format(format);
    let root = dir.path();
    std::fs::write(root.join("first.txt"), "staged\n").unwrap();
    let provider = LocalCodeProvider::new(root);
    let objects = git(root, &["count-objects", "-v"]);
    let untracked = provider
        .diff_snapshot("first.txt", 3, true, DiffScope::Combined, None)
        .unwrap();
    assert!(untracked.text.contains("+staged\n"));
    assert!(
        provider
            .diff_snapshot("first.txt", 3, true, DiffScope::Staged, None)
            .unwrap()
            .text
            .is_empty()
    );
    assert_eq!(git(root, &["count-objects", "-v"]), objects);
    assert!(!root.join(".git/index").exists());
    git(root, &["add", "."]);
    std::fs::write(root.join("first.txt"), "working\n").unwrap();
    let index = std::fs::read(root.join(".git/index")).unwrap();
    let objects = git(root, &["count-objects", "-v"]);
    for (scope, expected) in [
        (DiffScope::Combined, "+working\n"),
        (DiffScope::Staged, "+staged\n"),
    ] {
        let diff = provider
            .diff_snapshot("first.txt", 3, true, scope, None)
            .unwrap();
        assert!(diff.text.contains(expected));
        assert_eq!((diff.added, diff.removed), (1, 0));
    }
    let unstaged = provider
        .diff_snapshot("first.txt", 3, true, DiffScope::Unstaged, None)
        .unwrap();
    assert!(unstaged.text.contains("-staged\n"));
    assert!(unstaged.text.contains("+working\n"));
    assert_eq!((unstaged.added, unstaged.removed), (1, 1));
    assert!(provider.head().is_none());
    assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), index);
    assert_eq!(git(root, &["count-objects", "-v"]), objects);
    assert_eq!(git(root, &["show", ":first.txt"]), "staged");
}

#[test]
fn diff_totals_count_content_that_looks_like_patch_headers() {
    let dir = repository();
    let root = dir.path();
    std::fs::write(
        root.join("counter.txt"),
        "--counter;\n--- old heading\nunchanged\n",
    )
    .unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "base"]);
    std::fs::write(
        root.join("counter.txt"),
        "++counter;\n+++ new heading\nunchanged\n",
    )
    .unwrap();
    let provider = LocalCodeProvider::new(root);
    for context in [0, 3] {
        let diff = provider
            .diff_snapshot("counter.txt", context, true, DiffScope::Combined, None)
            .unwrap();
        assert_eq!((diff.added, diff.removed), (2, 2), "{}", diff.text);
    }
    git(root, &["commit", "-qam", "change"]);
    let oid = git(root, &["rev-parse", "HEAD"]);
    let diff = provider.commit_diff(&oid, "counter.txt").unwrap();
    assert_eq!((diff.added, diff.removed), (2, 2));
}
