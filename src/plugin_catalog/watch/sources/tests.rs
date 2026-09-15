use super::*;
use std::os::unix::{fs::symlink, net::UnixListener};

#[test]
fn missing_roots_are_read_only_and_later_creation_and_replacement_are_visible() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("catalog");
    let scan = || sample(std::slice::from_ref(&root)).unwrap();
    let absent = scan();
    assert!(!root.exists());
    fs::create_dir(&root).unwrap();
    let present = scan();
    assert_ne!(absent, present);
    fs::rename(&root, fixture.path().join("previous")).unwrap();
    fs::create_dir(&root).unwrap();
    assert_ne!(present, scan(), "equal paths do not hide root replacement");
}

#[test]
fn all_public_input_kinds_and_trust_changes_are_hints() {
    let root = tempfile::tempdir().unwrap();
    let scan = || sample(&[root.path().to_owned()]).unwrap();
    let mut previous = scan();
    for name in [
        "a.cowboy-plugin",
        "b.cowboy-provider",
        "a.release.json",
        "a.hostbundle.json",
        "trusted-publishers/fixture.pub",
    ] {
        let path = root.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"synthetic public input").unwrap();
        let created = scan();
        assert_ne!(previous, created, "creation: {name}");
        fs::write(&path, b"synthetic changed public input").unwrap();
        let edited = scan();
        assert_ne!(created, edited, "in-place change: {name}");
        fs::remove_file(&path).unwrap();
        previous = scan();
        assert_ne!(edited, previous, "removal: {name}");
    }
}

#[test]
fn reads_artifacts_receipts_and_private_state_do_not_rebuild_hosts() {
    let root = tempfile::tempdir().unwrap();
    let public = root.path().join("fixture.cowboy-plugin");
    fs::write(&public, b"synthetic").unwrap();
    let scan = || sample(&[root.path().to_owned()]).unwrap();
    let original = scan();
    fs::read(&public).unwrap();
    for name in ["artifacts", "receipts", "state"] {
        let directory = root.path().join(name);
        fs::create_dir(&directory).unwrap();
        let file = fs::File::create(directory.join("ignored.cowboy-plugin")).unwrap();
        file.set_len(1024 * 1024 * 1024).unwrap(); // sparse, never read by the probe
    }
    fs::write(root.path().join("download.tmp"), b"in progress").unwrap();
    assert_eq!(original, scan());
    assert_eq!(original, scan(), "scanning does not create another change");
}

#[test]
fn link_and_target_replacements_are_visible_without_opening_contents() {
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path().join("catalog");
    fs::create_dir(&root).unwrap();
    let target = fixture.path().join("target");
    fs::write(&target, b"synthetic").unwrap();
    let link = root.join("fixture.cowboy-plugin");
    symlink(&target, &link).unwrap();
    let scan = || sample(std::slice::from_ref(&root)).unwrap();
    let original = scan();
    fs::rename(&target, fixture.path().join("old-target")).unwrap();
    fs::write(&target, b"synthetic").unwrap();
    let replaced = scan();
    assert_ne!(original, replaced);
    fs::rename(&link, fixture.path().join("old-link")).unwrap();
    symlink(&target, &link).unwrap();
    assert_ne!(replaced, scan());
    // A socket with an input suffix can be statted but cannot be read as a file.
    // Metadata observation is deliberately not trusted-reader acceptance.
    let _socket = UnixListener::bind(root.join("socket.cowboy-plugin")).unwrap();
    assert_ne!(replaced, scan());
}

#[test]
fn entry_budget_counts_ignored_names_too() {
    let root = tempfile::tempdir().unwrap();
    for index in 0..=MAX_ENTRIES {
        fs::write(root.path().join(format!("ignored-{index}")), b"").unwrap();
    }
    let error = sample(&[root.path().to_owned()]).unwrap_err();
    assert!(error.to_string().contains("entry budget exceeded"));
}

#[test]
fn name_budget_is_shared_across_directories_and_checked_before_inspection() {
    let mut budget = Budget {
        names: MAX_NAME_BYTES,
        ..Budget::default()
    };
    let error = directory(
        Path::new("not-opened"),
        false,
        &mut Sha256::new(),
        &mut budget,
    )
    .unwrap_err();
    assert!(error.to_string().contains("name budget exceeded"));
}
