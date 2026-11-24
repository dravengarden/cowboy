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

fn git_line(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// A repository with one commit of `f.txt`, which carries `attributes`.
fn repository(root: &Path, attributes: &str) {
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "cowboy@example.invalid"]);
    git(root, &["config", "user.name", "Cowboy Test"]);
    std::fs::write(root.join("f.txt"), "one\n").unwrap();
    std::fs::write(root.join(".gitattributes"), attributes).unwrap();
    git(root, &["add", "-A"]);
    git(root, &["commit", "-qm", "initial"]);
}

/// A program that records that it ran by creating a marker file, then fails.
fn marking_program(markers: &Path, name: &str) -> String {
    let program = markers.parent().unwrap().join(format!("{name}.sh"));
    std::fs::write(
        &program,
        format!(
            "#!/bin/sh\ntouch {}\nexit 1\n",
            markers.join(name).display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(
        &program,
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .unwrap();
    program.display().to_string()
}

/// Each configured program records that it ran by creating a marker file.
fn hostile_repository(markers: &Path) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    repository(root, "f.txt filter=evil diff=evil\n");
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

/// A directory for marker files, beside which marking programs may be written.
fn markers() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let markers = dir.path().join("ran");
    std::fs::create_dir(&markers).unwrap();
    (dir, markers)
}

#[test]
fn review_reads_start_no_repository_configured_program() {
    let (_scratch, markers) = markers();
    let dir = hostile_repository(&markers);
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
    // The blob id Git would store for the worktree file, without its filter.
    let unfiltered = git_line(dir.path(), &["hash-object", "--no-filters", "--", "f.txt"]);
    assert_eq!(provider.file_blob("f.txt").unwrap().oid, unfiltered);

    assert_eq!(ran(&markers), Vec::<String>::new());
}

/// A repository marked as a partial clone names the remote, and through it the
/// programs, that Git asks for an object it does not have.
#[test]
fn review_reads_fetch_no_missing_object() {
    let (_scratch, markers) = markers();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    repository(root, "");
    std::fs::write(root.join("f.txt"), "two\n").unwrap();
    git(root, &["commit", "-qam", "second"]);
    let head = git_line(root, &["rev-parse", "HEAD"]);
    let blob = git_line(root, &["rev-parse", "HEAD:f.txt"]);
    std::fs::remove_file(root.join(".git/objects").join(&blob[..2]).join(&blob[2..])).unwrap();
    let promisor = tempfile::tempdir().unwrap();
    git(promisor.path(), &["init", "-q", "--bare"]);
    for (key, value) in [
        ("extensions.partialClone", "evil".to_owned()),
        ("remote.evil.url", promisor.path().display().to_string()),
        ("remote.evil.promisor", "true".to_owned()),
        (
            "remote.evil.uploadpack",
            format!(
                "touch {}; git-upload-pack",
                markers.join("uploadpack").display()
            ),
        ),
        (
            "core.sshCommand",
            format!("touch {}; false", markers.join("ssh").display()),
        ),
    ] {
        git(root, &["config", key, &value]);
    }
    let provider = LocalCodeProvider::new(root);

    assert!(provider.blob(&blob).is_err());
    provider.commit(&head).unwrap();
    assert!(provider.commit_diff(&head, "f.txt").is_err());
    provider.changes(None).unwrap();
    let _ = provider.diff_snapshot("f.txt", 3, true, DiffScope::Combined, None);

    assert_eq!(ran(&markers), Vec::<String>::new());
}

/// A nested repository brings its own configuration. Review reports one whose
/// checked-out commit moved, and never compares its worktree or renders its
/// changes, which Git would do in a child process under that configuration.
#[test]
fn review_reads_run_no_nested_repository_program() {
    let (_scratch, markers) = markers();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    repository(root, "");
    for name in ["moved", "edited"] {
        let nested = root.join(name);
        std::fs::create_dir(&nested).unwrap();
        repository(&nested, "f.txt filter=inner diff=inner\n");
    }
    git(root, &["add", "moved", "edited"]);
    git(root, &["commit", "-qm", "nested repositories"]);
    // The outer repository records a second commit of `moved`, which then
    // moves once more; `edited` only gets an uncommitted change.
    let moved = root.join("moved");
    for text in ["two\n", "three\n"] {
        std::fs::write(moved.join("f.txt"), text).unwrap();
        git(&moved, &["commit", "-qam", "next"]);
        if text == "two\n" {
            git(root, &["commit", "-qam", "advance moved"]);
        }
    }
    let head = git_line(root, &["rev-parse", "HEAD"]);
    git(root, &["config", "diff.submodule", "diff"]);
    for name in ["moved", "edited"] {
        let nested = root.join(name);
        let marker = |kind: &str| markers.join(format!("{name}-{kind}")).display().to_string();
        for (key, value) in [
            (
                "filter.inner.clean",
                format!("touch {}; cat", marker("clean")),
            ),
            (
                "diff.inner.textconv",
                format!("touch {}; cat", marker("textconv")),
            ),
            (
                "core.fsmonitor",
                format!("touch {}; true", marker("fsmonitor")),
            ),
        ] {
            git(&nested, &["config", key, &value]);
        }
        std::fs::write(nested.join("f.txt"), "uncommitted\n").unwrap();
    }
    let provider = LocalCodeProvider::new(root);

    let changes = provider.changes(None).unwrap();
    let paths: Vec<&str> = changes
        .changes
        .iter()
        .map(|change| change.path.as_str())
        .collect();
    assert_eq!(paths, ["moved"]);
    for scope in [DiffScope::Combined, DiffScope::Staged, DiffScope::Unstaged] {
        let _ = provider.diff_snapshot("moved", 3, true, scope, None);
    }
    provider.commit(&head).unwrap();
    let diff = provider.commit_diff(&head, "moved").unwrap();
    assert!(diff.text.contains("Subproject commit"), "{}", diff.text);

    assert_eq!(ran(&markers), Vec::<String>::new());
}

#[test]
fn review_fetch_runs_no_repository_configured_program() {
    let (_scratch, markers) = markers();
    let source = hostile_repository(&markers);
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
        format!("#!/bin/sh\ntouch {}\n", markers.join("hook").display()),
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    // Git asks the alternate-refs lister only when the repository has an alternate.
    let alternate = tempfile::tempdir().unwrap();
    git(alternate.path(), &["init", "-q", "--bare"]);
    std::fs::write(
        clone.path().join(".git/objects/info/alternates"),
        format!("{}\n", alternate.path().join("objects").display()),
    )
    .unwrap();
    let marker = |name: &str| markers.join(name).display().to_string();
    for (key, value) in [
        (
            "remote.origin.uploadpack",
            format!("touch {}; git-upload-pack", marker("uploadpack")),
        ),
        (
            "core.alternateRefsCommand",
            format!("touch {}; true", marker("alternate-refs")),
        ),
        ("core.sshCommand", format!("touch {}; false", marker("ssh"))),
        ("core.gitProxy", marking_program(&markers, "proxy")),
        ("core.askPass", marking_program(&markers, "askpass")),
        (
            "credential.helper",
            format!("!touch {}", marker("credential")),
        ),
    ] {
        git(clone.path(), &["config", key, &value]);
    }
    let fetch = || {
        super::git_fetch(
            clone.path(),
            "origin",
            "+refs/heads/*:refs/cowboy/review/origin/*",
        )
    };

    fetch().unwrap();
    assert_eq!(ran(&markers), Vec::<String>::new());

    // Nothing answers on this port, so a transport that reaches for another
    // host fails at once unless a repository program takes its place.
    for url in ["ssh://git@127.0.0.1:1/x.git", "git://127.0.0.1:1/x.git"] {
        git(clone.path(), &["config", "remote.origin.url", url]);
        assert!(fetch().is_err(), "{url}");
    }
    assert_eq!(ran(&markers), Vec::<String>::new());
}

fn settings(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn only_user_and_system_configuration_is_trusted() {
    let listed = b"system\0core.sshcommand\nssh -4\0\
                   global\0credential.https://example.test.helper\n\0\
                   global\0credential.https://example.test.helper\n!helper\0\
                   local\0core.sshcommand\nevil\0\
                   worktree\0credential.helper\n!evil\0\
                   command\0core.askpass\nevil\0\
                   global\0core.askpass\0";
    assert_eq!(
        super::trusted_settings(listed),
        settings(&[
            ("core.sshcommand", "ssh -4"),
            ("credential.https://example.test.helper", ""),
            ("credential.https://example.test.helper", "!helper"),
            ("core.askpass", ""),
        ])
    );
    assert!(super::trusted_settings(b"").is_empty());
}

#[test]
fn a_fetch_pins_every_transport_program() {
    // Nothing is configured anywhere: Git's own defaults, stated explicitly.
    let plain = super::transport(&[], "origin", |_| None);
    assert_eq!(
        plain,
        super::Transport {
            overrides: settings(&[
                ("credential.helper", ""),
                ("core.alternaterefscommand", "true"),
            ]),
            upload_pack: "git-upload-pack".to_owned(),
            environment: vec![
                ("GIT_SSH_COMMAND", "ssh".to_owned()),
                ("GIT_ASKPASS", String::new()),
                ("GIT_PROXY_COMMAND", String::new()),
            ],
            protocols: "https:ssh:git:file",
        }
    );

    // User and system configuration keep their say, in Git's order: the last
    // value of a setting, the first `uploadpack` of this remote only, and
    // every credential helper after the reset.
    let trusted = settings(&[
        ("core.sshcommand", "ssh -4"),
        ("core.sshcommand", "ssh -6"),
        ("core.askpass", "/trusted/askpass"),
        ("core.alternaterefscommand", "/trusted/refs"),
        ("credential.helper", "cache"),
        ("credential.https://example.test.helper", ""),
        ("credential.https://example.test.helper", "!helper"),
        ("remote.other.uploadpack", "other-pack"),
        ("remote.origin.uploadpack", "first-pack"),
        ("remote.origin.uploadpack", "second-pack"),
        ("core.gitproxy", "proxy for example.test"),
    ]);
    let configured = super::transport(&trusted, "origin", |_| None);
    assert_eq!(
        configured.overrides,
        settings(&[
            ("credential.helper", ""),
            ("credential.helper", "cache"),
            ("credential.https://example.test.helper", ""),
            ("credential.https://example.test.helper", "!helper"),
            ("core.alternaterefscommand", "/trusted/refs"),
        ])
    );
    assert_eq!(configured.upload_pack, "first-pack");
    assert_eq!(
        configured.environment,
        [
            ("GIT_SSH_COMMAND", "ssh -6".to_owned()),
            ("GIT_ASKPASS", "/trusted/askpass".to_owned()),
        ]
    );
    // A trusted proxy rule cannot be pinned for every host.
    assert_eq!(configured.protocols, "https:ssh:file");

    // What the Controller's environment already decides is left alone, and
    // the fallbacks Git itself would use stay in effect.
    let inherited = super::transport(&trusted, "origin", |name| {
        matches!(
            name,
            "GIT_SSH_COMMAND" | "GIT_ASKPASS" | "GIT_PROXY_COMMAND"
        )
        .then(|| "inherited".to_owned())
    });
    assert!(inherited.environment.is_empty());
    assert_eq!(inherited.protocols, "https:ssh:git:file");
    let fallbacks = super::transport(&[], "origin", |name| match name {
        "GIT_SSH" => Some("/opt/it's ssh".to_owned()),
        "SSH_ASKPASS" => Some("/usr/bin/ssh-askpass".to_owned()),
        _ => None,
    });
    assert_eq!(
        fallbacks.environment,
        [
            ("GIT_SSH_COMMAND", r"'/opt/it'\''s ssh'".to_owned()),
            ("GIT_ASKPASS", "/usr/bin/ssh-askpass".to_owned()),
            ("GIT_PROXY_COMMAND", String::new()),
        ]
    );
}
