use super::super::{CodeProvider as _, DiffScope, LocalCodeProvider};
use std::path::Path;
use std::process::Command;

fn git(root: &Path, args: &[&str]) {
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
}

/// Each configured program records that it ran by creating a marker file.
fn hostile_repository(markers: &Path) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "cowboy@example.invalid"]);
    git(root, &["config", "user.name", "Cowboy Test"]);
    std::fs::write(root.join("f.txt"), "one\n").unwrap();
    std::fs::write(root.join(".gitattributes"), "f.txt filter=evil diff=evil\n").unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-qm", "initial"]);
    let marker = |name: &str| markers.join(name).display().to_string();
    for (key, value) in [
        (
            "core.fsmonitor",
            format!("touch {}; true", marker("fsmonitor")),
        ),
        (
            "filter.evil.clean",
            format!("touch {}; cat", marker("clean")),
        ),
        (
            "filter.evil.smudge",
            format!("touch {}; cat", marker("smudge")),
        ),
        (
            "filter.evil.process",
            format!("touch {}; false", marker("process")),
        ),
        (
            "diff.evil.textconv",
            format!("touch {}; cat", marker("textconv")),
        ),
        (
            "diff.evil.command",
            format!("touch {}; true", marker("external")),
        ),
        ("diff.external", marker("external")),
        ("log.showSignature", "true".to_owned()),
        ("gpg.program", marker("gpg")),
    ] {
        git(root, &["config", key, &value]);
    }
    let hooks = root.join(".git/hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    let hook = hooks.join("reference-transaction");
    std::fs::write(&hook, format!("#!/bin/sh\ntouch {}\n", marker("hook"))).unwrap();
    std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    std::fs::write(root.join("f.txt"), "two\n").unwrap();
    dir
}

fn ran(markers: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(markers)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn review_reads_start_no_repository_configured_program() {
    let markers = tempfile::tempdir().unwrap();
    let dir = hostile_repository(markers.path());
    let provider = LocalCodeProvider::new(dir.path());

    provider.manifest().unwrap();
    let changes = provider.changes(None).unwrap();
    assert!(changes.changes.iter().any(|change| change.path == "f.txt"));
    for scope in [DiffScope::Combined, DiffScope::Staged, DiffScope::Unstaged] {
        provider
            .diff_snapshot("f.txt", 3, true, scope, None)
            .unwrap();
    }
    let diff = provider
        .diff_snapshot("f.txt", 3, true, DiffScope::Combined, None)
        .unwrap();
    assert!(diff.text.contains("+two"), "{}", diff.text);
    let repository = provider.repository(None).unwrap();
    let head = &repository.commits[0].oid;
    provider.commit(head).unwrap();
    provider.commit_diff(head, "f.txt").unwrap();

    assert_eq!(ran(markers.path()), Vec::<String>::new());
}

#[test]
fn review_fetch_runs_no_repository_hook() {
    let markers = tempfile::tempdir().unwrap();
    let source = hostile_repository(markers.path());
    let clone = tempfile::tempdir().unwrap();
    git(
        clone.path(),
        &["clone", "-q", &source.path().display().to_string(), "."],
    );
    let hooks = clone.path().join(".git/hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    let hook = hooks.join("reference-transaction");
    std::fs::write(
        &hook,
        format!(
            "#!/bin/sh\ntouch {}\n",
            markers.path().join("hook").display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

    super::git_fetch(
        clone.path(),
        "origin",
        "refs/heads/*:refs/cowboy/review/origin/*",
    )
    .unwrap();

    assert_eq!(ran(markers.path()), Vec::<String>::new());
}
