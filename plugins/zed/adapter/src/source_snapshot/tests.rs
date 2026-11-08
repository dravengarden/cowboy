use super::*;
use std::path::PathBuf;

struct Root(PathBuf);

impl Root {
    fn new() -> Self {
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).unwrap();
        let path =
            std::env::temp_dir().join(format!("cw-source-{:032x}", u128::from_be_bytes(nonce)));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Called only by the opt-in native gate in its isolated mount namespace.
pub(crate) fn bind_mount_is_refused(root: &Path) {
    let scope = root.join("source-scope");
    let outside = root.join("source-outside");
    let target = scope.join("mounted");
    std::fs::create_dir_all(&target).unwrap();
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("file"), "not in the source scope").unwrap();
    assert!(
        std::process::Command::new("mount")
            .arg("--bind")
            .arg(&outside)
            .arg(&target)
            .status()
            .unwrap()
            .success()
    );
    let result = identity(&scope, Path::new("mounted/file"));
    assert!(
        std::process::Command::new("umount")
            .arg(&target)
            .status()
            .unwrap()
            .success()
    );
    assert!(result.is_none());
    println!("native source identity: actual bind mount refused without a pathname fallback");
}

#[tokio::test]
async fn source_checks_normalize_like_native_text_and_never_reload_it() {
    let root = Root::new();
    let path = Path::new("text.rs");
    let raw = "a\r\n汉🙂\rb\n";
    std::fs::write(root.0.join(path), raw).unwrap();
    let mut normalized = raw.to_owned();
    text::LineEnding::normalize(&mut normalized);
    let expected = crate::coordinates::Mirror::new(1, &normalized)
        .unwrap()
        .content()
        .clone();
    assert_eq!(check(&root.0, path, Some(&expected)).await, State::Matching);
    std::fs::write(root.0.join(path), "different").unwrap();
    assert_eq!(check(&root.0, path, Some(&expected)).await, State::Changed);
    assert_eq!(
        std::fs::read_to_string(root.0.join(path)).unwrap(),
        "different"
    );
    assert_eq!(check(&root.0, path, None).await, State::Unavailable);
}

#[test]
fn source_reads_refuse_aliases_special_files_and_over_budget_input() {
    use rustix::fs::{CWD, Mode, mkfifoat};
    use std::os::unix::fs::symlink;
    let root = Root::new();
    let outside = Root::new();
    std::fs::write(outside.0.join("secret"), "outside").unwrap();
    std::fs::write(root.0.join("real"), "inside").unwrap();
    symlink(outside.0.join("secret"), root.0.join("escape")).unwrap();
    symlink(&outside.0, root.0.join("parent")).unwrap();
    symlink("real", root.0.join("alias")).unwrap();
    mkfifoat(CWD, root.0.join("fifo"), Mode::RUSR | Mode::WUSR).unwrap();
    std::fs::write(root.0.join("invalid"), [0xff]).unwrap();
    std::fs::File::create(root.0.join("huge"))
        .unwrap()
        .set_len(MAX_BYTES + 1)
        .unwrap();
    for path in [
        "",
        ".",
        "../secret",
        "/etc/passwd",
        "escape",
        "parent/secret",
        "alias",
        "fifo",
        "invalid",
        "huge",
    ] {
        assert!(identity(&root.0, Path::new(path)).is_none(), "{path}");
    }
    assert!(identity(&root.0, Path::new("real")).is_some());
}

#[test]
fn mutation_or_replacement_during_a_read_never_produces_matching_evidence() {
    let root = Root::new();
    let path = Path::new("file");
    std::fs::write(root.0.join(path), "before").unwrap();
    assert!(
        identity_with(&root.0, path, || {
            std::fs::write(root.0.join(path), "after!").unwrap();
        })
        .is_none()
    );
    assert!(
        identity_with(&root.0, path, || {
            std::fs::rename(root.0.join(path), root.0.join("old")).unwrap();
            std::fs::write(root.0.join(path), "after!").unwrap();
        })
        .is_none()
    );
}

#[tokio::test]
async fn timed_out_io_keeps_its_admission_until_the_actual_job_exits() {
    let readers = Arc::new(Semaphore::new(1));
    let (release, wait) = std::sync::mpsc::channel();
    assert!(
        observe(Arc::clone(&readers), move || {
            let _ = wait.recv();
            None
        })
        .await
        .is_none()
    );
    assert_eq!(readers.available_permits(), 0);
    assert!(
        observe(Arc::clone(&readers), || panic!(
            "saturated reader was dispatched"
        ))
        .await
        .is_none()
    );
    release.send(()).unwrap();
    let permit = tokio::time::timeout(Duration::from_secs(5), readers.acquire())
        .await
        .unwrap()
        .unwrap();
    drop(permit);
}
