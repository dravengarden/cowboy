//! Controller-owned identity for a root the Controller reads itself.
//!
//! The party that touches the filesystem owns the root's continuous identity.
//! For a colocated Machine, and for a standalone `local` Session, that party
//! is the Controller. This observes one directory object; it is not a lease,
//! a grant, a Machine incarnation, or a cancellation of work already started.

use std::path::Path;
use std::time::Duration;

/// The exact object a local read observation was taken against.
///
/// Device and inode alone are unsound: a directory deleted and recreated at
/// the same path reuses its inode immediately on the deployed filesystem, as
/// `a_recreated_root_reuses_its_inode_and_is_caught_by_creation_time` shows.
/// The Machine prevents that reuse by retaining an open handle; the Controller
/// cannot afford one descriptor per advertised root, so it detects reuse by
/// creation time instead and refuses any root that cannot report one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct LocalRootIdentity {
    device: u64,
    inode: u64,
    created: Duration,
}

impl LocalRootIdentity {
    /// `None` when the path is absent, is not a directory, or cannot report a
    /// creation time. Every caller treats that as "do not execute locally".
    pub(crate) fn observe(root: &str) -> Option<Self> {
        use std::os::unix::fs::MetadataExt as _;
        let path = Path::new(root);
        if !path.is_absolute() {
            return None;
        }
        let metadata = std::fs::metadata(path).ok()?;
        if !metadata.is_dir() {
            return None;
        }
        let created = metadata
            .created()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?;
        Some(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            created,
        })
    }

    /// Re-observe the same path and require the same object. A path that has
    /// become unobservable is a change, never a pass.
    pub(crate) fn is_current(self, root: &str) -> bool {
        Self::observe(root) == Some(self)
    }
}

/// What a read observation recorded about the root it may execute against.
///
/// Resolving a route is not the place to refuse: an unobservable root is
/// recorded as such and refused at the single gate that would actually read
/// it. That keeps "this route exists" and "this root is readable here"
/// separate, and it is why an absent path does not silently become a remote
/// route or a readable one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum LocalRoot {
    /// A remote Machine owns this root. The Controller must not stat it.
    Remote,
    /// The Controller would read it, but could not observe the object.
    Unobservable,
    Observed(LocalRootIdentity),
}

impl LocalRoot {
    pub(crate) fn observe(root: &str) -> Self {
        LocalRootIdentity::observe(root).map_or(Self::Unobservable, Self::Observed)
    }

    /// Whether the recorded observation still describes the live path.
    /// A root that appeared, vanished or was replaced all end the observation.
    pub(crate) fn is_current(self, root: &str) -> bool {
        match self {
            Self::Remote => true,
            Self::Unobservable => LocalRootIdentity::observe(root).is_none(),
            Self::Observed(identity) => identity.is_current(root),
        }
    }

    /// The exact object this route may read, or a typed refusal. Never reads
    /// a root the Controller could not observe.
    pub(crate) fn readable(self, root: &str) -> Result<(), String> {
        match self {
            Self::Remote => Err("local read requested for a remote root".into()),
            Self::Unobservable => Err("local root cannot be observed".into()),
            Self::Observed(identity) if identity.is_current(root) => Ok(()),
            Self::Observed(_) => Err("local root was replaced".into()),
        }
    }
}

/// Real directories for tests whose fixtures used to name synthetic paths.
/// A local read route is only resolvable when its root actually exists, so
/// these must be observable objects rather than strings.
#[cfg(test)]
pub(crate) fn test_root(name: &str) -> String {
    static SHARED: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let root = SHARED
        .get_or_init(|| tempfile::tempdir().expect("shared test root"))
        .path()
        .join(name);
    std::fs::create_dir_all(&root).expect("test root");
    root.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact sequence the fence must catch, on the deployed filesystem.
    /// Device and inode are reused; only the creation time separates them.
    #[test]
    fn a_recreated_root_reuses_its_inode_and_is_caught_by_creation_time() {
        let parent = tempfile::tempdir().expect("temp");
        let path = parent.path().join("root");
        std::fs::create_dir(&path).expect("create");
        let root = path.display().to_string();
        let original = LocalRootIdentity::observe(&root).expect("observe");
        assert!(original.is_current(&root));
        std::fs::remove_dir_all(&path).expect("remove");
        std::fs::create_dir(&path).expect("recreate");
        let replaced = LocalRootIdentity::observe(&root).expect("observe");
        assert_ne!(original, replaced);
        assert!(!original.is_current(&root));
        assert!(replaced.is_current(&root));
        // The inode really was reused, so this fence cannot rest on it.
        if original.device == replaced.device && original.inode == replaced.inode {
            assert_ne!(original.created, replaced.created);
        }
    }

    #[test]
    fn unobservable_relative_and_non_directory_roots_have_no_identity() {
        let parent = tempfile::tempdir().expect("temp");
        let file = parent.path().join("file");
        std::fs::write(&file, b"x").expect("write");
        assert!(LocalRootIdentity::observe("relative/path").is_none());
        assert!(LocalRootIdentity::observe(&file.display().to_string()).is_none());
        assert!(
            LocalRootIdentity::observe(&parent.path().join("absent").display().to_string())
                .is_none()
        );
    }

    #[test]
    fn a_removed_root_is_a_change_not_a_pass() {
        let parent = tempfile::tempdir().expect("temp");
        let path = parent.path().join("root");
        std::fs::create_dir(&path).expect("create");
        let root = path.display().to_string();
        let original = LocalRootIdentity::observe(&root).expect("observe");
        std::fs::remove_dir(&path).expect("remove");
        assert!(!original.is_current(&root));
        // A file at the same path is not the root either.
        std::fs::write(&path, b"x").expect("write");
        assert!(!original.is_current(&root));
    }

    #[test]
    fn unrelated_roots_and_content_changes_are_independent() {
        let parent = tempfile::tempdir().expect("temp");
        let first = parent.path().join("first");
        let second = parent.path().join("second");
        std::fs::create_dir(&first).expect("create");
        std::fs::create_dir(&second).expect("create");
        let a = first.display().to_string();
        let b = second.display().to_string();
        let original = LocalRootIdentity::observe(&a).expect("observe");
        assert_ne!(Some(original), LocalRootIdentity::observe(&b));
        // Writing inside a root does not replace the root.
        std::fs::write(first.join("file.txt"), b"x").expect("write");
        std::fs::create_dir(first.join("child")).expect("create");
        assert!(original.is_current(&a));
        // Replacing the neighbour leaves this one alone.
        std::fs::remove_dir_all(&second).expect("remove");
        std::fs::create_dir(&second).expect("recreate");
        assert!(original.is_current(&a));
    }
}
