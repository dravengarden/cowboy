//! Copy only the stopped disposable fixture, never live or production storage.
use super::*;
use std::io::Read as _;
use std::os::unix::fs::FileTypeExt as _;

pub(super) fn stopped_fixture(from: &Path, to: &Path) -> Result<(), Failure> {
    copy(from, to).map_err(|_| Failure::Setup)
}

fn copy(from: &Path, to: &Path) -> Result<()> {
    ensure!(
        from.is_absolute() && to.is_absolute() && from != to,
        "distinct private roots required"
    );
    ensure!(
        std::fs::read_dir(to)?.next().is_none(),
        "empty destination required"
    );
    let source = from.canonicalize()?;
    let mut pending = vec![PathBuf::new()];
    let mut count = 0_u32;
    let mut bytes = 0_u64;
    while let Some(relative) = pending.pop() {
        for entry in std::fs::read_dir(from.join(&relative))? {
            count += 1;
            ensure!(count <= 4096, "fixture copy capacity exceeded");
            let entry = entry?;
            let relative = relative.join(entry.file_name());
            let source_path = from.join(&relative);
            let destination = to.join(&relative);
            let metadata = source_path.symlink_metadata()?;
            if metadata.is_dir() {
                std::fs::create_dir(&destination)?;
                std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(0o700))?;
                pending.push(relative);
            } else if metadata.is_file() {
                let file = std::fs::OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                    .open(&source_path)?;
                let mut contents = Vec::new();
                file.take(16 * 1024 * 1024 + 1).read_to_end(&mut contents)?;
                bytes += u64::try_from(contents.len())?;
                ensure!(
                    contents.len() <= 16 * 1024 * 1024 && bytes <= 128 * 1024 * 1024,
                    "fixture bytes exceeded"
                );
                private_write(&destination, &contents)?;
            } else if metadata.file_type().is_symlink() {
                let target = std::fs::read_link(&source_path)?;
                if relative == Path::new("tools/ssh-keygen") {
                    ensure!(
                        target.starts_with("/nix/store") && target.canonicalize()? == target,
                        "immutable fixture helper required"
                    );
                } else {
                    ensure!(
                        target.is_relative() && source_path.canonicalize()?.starts_with(&source),
                        "fixture link escaped its root"
                    );
                }
                std::os::unix::fs::symlink(target, destination)?;
            } else {
                ensure!(
                    metadata.file_type().is_socket()
                        && [Path::new("broker.sock"), Path::new("usage.sock")]
                            .contains(&relative.as_path()),
                    "unexpected fixture special file"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn stopped_fixture_copy_preserves_only_private_owned_files_and_safe_links() {
    let from = tempfile::tempdir().unwrap();
    let to = tempfile::tempdir().unwrap();
    std::fs::create_dir(from.path().join("data")).unwrap();
    private_write(&from.path().join("data/record"), b"fixture-only").unwrap();
    std::os::unix::fs::symlink("data/record", from.path().join("active")).unwrap();
    stopped_fixture(from.path(), to.path()).unwrap();
    assert_eq!(
        std::fs::read(to.path().join("active")).unwrap(),
        b"fixture-only"
    );
    assert_eq!(
        std::fs::metadata(to.path().join("data/record"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(stopped_fixture(from.path(), to.path()).is_err());
    let bad = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink("/etc/passwd", bad.path().join("escape")).unwrap();
    assert!(stopped_fixture(bad.path(), tempfile::tempdir().unwrap().path()).is_err());
}
