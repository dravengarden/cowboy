use super::{ContentQuery, StopReason, search};

fn query(text: &str) -> ContentQuery {
    ContentQuery {
        query: text.to_owned(),
        regex: false,
        case_sensitive: None,
        path: None,
        limit: 100,
    }
}

fn worktree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("src/deep")).unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "fn alpha() {}\n// 中文 alpha Beta\n",
    )
    .unwrap();
    std::fs::write(root.join("src/deep/mod.rs"), "let ALPHA = 1;\n").unwrap();
    std::fs::write(root.join("target/out.rs"), "alpha ignored\n").unwrap();
    std::fs::write(root.join(".git/config"), "alpha\n").unwrap();
    std::fs::write(root.join(".hidden"), "alpha\n").unwrap();
    std::fs::write(root.join("blob.bin"), b"alpha\0binary").unwrap();
    dir
}

#[test]
fn literal_smart_case_skips_ignored_hidden_and_binary_files() {
    let dir = worktree();
    let result = search(dir.path(), &query("alpha")).unwrap();
    let mut found: Vec<_> = result
        .matches
        .iter()
        .map(|m| (m.path.as_str(), m.line, m.column))
        .collect();
    found.sort_unstable();
    assert_eq!(
        found,
        vec![
            ("src/deep/mod.rs", 1, 5),
            ("src/lib.rs", 1, 4),
            ("src/lib.rs", 2, 7)
        ]
    );
    assert_eq!(result.stopped, None);
    assert_eq!(result.files_skipped, 1, "the binary file");
    let sensitive = search(dir.path(), &query("ALPHA")).unwrap();
    assert_eq!(
        sensitive.matches.len(),
        1,
        "uppercase makes it case sensitive"
    );
}

#[test]
fn regex_paths_limits_and_invalid_input() {
    let dir = worktree();
    let mut regex = query(r"fn \w+\(");
    regex.regex = true;
    assert_eq!(search(dir.path(), &regex).unwrap().matches.len(), 1);
    let mut scoped = query("alpha");
    scoped.path = Some("src/deep".into());
    let scoped = search(dir.path(), &scoped).unwrap();
    assert_eq!(scoped.matches.len(), 1);
    assert_eq!(scoped.matches[0].path, "src/deep/mod.rs");
    let mut limited = query("alpha");
    limited.limit = 1;
    let limited = search(dir.path(), &limited).unwrap();
    assert_eq!(limited.matches.len(), 1);
    assert_eq!(limited.stopped, Some(StopReason::Limit));
    for bad in ["", "a\nb"] {
        assert!(search(dir.path(), &query(bad)).is_err());
    }
    let mut escape = query("alpha");
    escape.path = Some("../".into());
    assert!(search(dir.path(), &escape).is_err());
    let mut invalid = query("(");
    invalid.regex = true;
    assert!(search(dir.path(), &invalid).is_err());
}

#[test]
fn symlinks_out_of_the_worktree_are_not_followed() {
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.rs"), "alpha secret\n").unwrap();
    let dir = worktree();
    std::os::unix::fs::symlink(outside.path(), dir.path().join("src/link")).unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("secret.rs"),
        dir.path().join("src/file-link.rs"),
    )
    .unwrap();
    let result = search(dir.path(), &query("secret")).unwrap();
    assert!(result.matches.is_empty(), "{:?}", result.matches);
    let mut through = query("secret");
    through.path = Some("src/link".into());
    assert!(search(dir.path(), &through).is_err());
}
