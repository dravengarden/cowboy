//! Bounded source equality observation. Never reload, save, or acquire a buffer.
use std::path::Path;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use serde::Serialize;
use tokio::sync::Semaphore;

use crate::content_reads::Content;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum State {
    Matching,
    Changed,
    #[default]
    Unavailable,
}

static READERS: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(2)));
#[cfg(target_os = "linux")]
const MAX_BYTES: u64 = 4 * 1024 * 1024;

pub(crate) async fn check(root: &Path, path: &Path, expected: Option<&Content>) -> State {
    let Some(expected) = expected.cloned() else {
        return State::Unavailable;
    };
    let root = root.to_owned();
    let path = path.to_owned();
    let Some(observed) = observe(Arc::clone(&READERS), move || identity(&root, &path)).await else {
        return State::Unavailable;
    };
    if observed == expected {
        State::Matching
    } else {
        State::Changed
    }
}

async fn observe(
    readers: Arc<Semaphore>,
    read: impl FnOnce() -> Option<Content> + Send + 'static,
) -> Option<Content> {
    let permit = readers.try_acquire_owned().ok()?;
    // A timeout does not cancel blocking filesystem I/O. The actual job keeps
    // its permit until it exits, so repeated timeouts cannot grow a work queue.
    let job = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        read()
    });
    tokio::time::timeout(Duration::from_secs(1), job)
        .await
        .ok()?
        .ok()?
}

#[cfg(target_os = "linux")]
fn identity(root: &Path, path: &Path) -> Option<Content> {
    identity_with(root, path, || {})
}

#[cfg(not(target_os = "linux"))]
fn identity(_root: &Path, _path: &Path) -> Option<Content> {
    None
}

#[cfg(target_os = "linux")]
fn identity_with(root: &Path, path: &Path, after_read: impl FnOnce()) -> Option<Content> {
    use rustix::fs::{Mode, OFlags, ResolveFlags, open, openat2};
    use sha2::{Digest as _, Sha256};
    use std::fmt::Write as _;
    use std::fs::{File, Metadata};
    use std::io::Read as _;
    use std::os::unix::fs::MetadataExt as _;
    use std::path::Component;

    if path.as_os_str().is_empty()
        || path.as_os_str().len() > 4096
        || !path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
    {
        return None;
    }
    let directory = open(
        root,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .ok()?;
    let read = || {
        openat2(
            &directory,
            path,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_XDEV,
        )
        .ok()
        .map(File::from)
    };
    let file = read()?;
    let before = file.metadata().ok()?;
    if !before.is_file() || before.len() > MAX_BYTES {
        return None;
    }
    let stamp = |meta: &Metadata| {
        (
            meta.dev(),
            meta.ino(),
            meta.mode(),
            meta.len(),
            meta.mtime(),
            meta.mtime_nsec(),
            meta.ctime(),
            meta.ctime_nsec(),
        )
    };
    let mut bytes = Vec::new();
    (&file).take(MAX_BYTES + 1).read_to_end(&mut bytes).ok()?;
    after_read();
    let current = read()?;
    let mut verification = Vec::new();
    (&current)
        .take(MAX_BYTES + 1)
        .read_to_end(&mut verification)
        .ok()?;
    if bytes.len() as u64 != before.len()
        || verification != bytes
        || stamp(&file.metadata().ok()?) != stamp(&before)
        || stamp(&current.metadata().ok()?) != stamp(&before)
    {
        return None;
    }
    let mut text = String::from_utf8(bytes).ok()?;
    text::LineEnding::normalize(&mut text);
    let mut sha256 = String::with_capacity(64);
    for byte in Sha256::digest(text.as_bytes()) {
        write!(sha256, "{byte:02x}").expect("write to string");
    }
    Some(Content {
        sha256,
        utf8_bytes: u32::try_from(text.len()).ok()?,
    })
}

#[cfg(all(test, target_os = "linux"))]
pub(crate) mod tests;
