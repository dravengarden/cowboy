//! Machine-owned continuous identity for advertised workspace roots.
//!
//! The Machine is the only party that observes the object behind a root, so it
//! is the only party that may mint an identity for it. The Controller stores
//! and echoes the opaque value; it cannot derive, renew or widen one. This is a
//! read fence, not authority, a lease, a filesystem proof for anyone else, or a
//! cancellation of work the Code adapter has already started.

use std::collections::HashMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context as _;

use crate::machine_protocol::{MachineWorkspace, WorkspaceRootIdentity};

/// Why a carried identity was refused. Only `Changed` reports an observation
/// the Machine has actually ended, so only it may retire Controller state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RootIdentityRefusal {
    /// The carry itself is unusable: wrong adapter, missing or relative root,
    /// oversized value, or a root this Machine does not advertise.
    Unusable(&'static str),
    /// The advertised root no longer holds the object that minted it.
    Changed(&'static str),
}

impl RootIdentityRefusal {
    pub(super) fn detail(&self) -> &'static str {
        match self {
            Self::Unusable(detail) | Self::Changed(detail) => detail,
        }
    }

    pub(super) fn is_changed(&self) -> bool {
        matches!(self, Self::Changed(_))
    }
}

/// Each tracked root holds one open directory handle, so this is also an open
/// file-descriptor budget. Only `observe_roots` inserts; an adapter request can
/// never grow this map. A configuration beyond this size advertises no identity
/// for the excess, which the Controller refuses rather than reads unfenced.
const MAX_TRACKED_ROOTS: usize = 256;

/// The exact object an incarnation was minted for. Device and inode numbers
/// alone would be unsound because the kernel reuses them: a directory deleted
/// and recreated at the same path can land on the same inode. The retained
/// handle below pins the original inode for as long as the identity exists, so
/// a replacement is necessarily a different object.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ObservedObject {
    device: u64,
    inode: u64,
    /// Present on filesystems that record it; an extra axis, never the only one.
    created: Option<std::time::SystemTime>,
    directory: bool,
}

#[derive(Debug)]
struct TrackedRoot {
    /// Kept open only to reserve the inode. It is never read or written, and
    /// grants this Machine no access it did not already have.
    _handle: std::fs::File,
    object: ObservedObject,
    incarnation: String,
}

/// Process-local and never persisted. A restarted Machine mints fresh
/// identities, which correctly ends every Controller observation it had.
#[derive(Debug, Default)]
pub(super) struct RootIdentities {
    roots: HashMap<PathBuf, TrackedRoot>,
}

/// The advertisement path and the adapter path observe the same registry.
pub(super) type SharedRootIdentities = Arc<parking_lot::Mutex<RootIdentities>>;

/// Open the root and describe the exact object behind it. The caller decides
/// whether to retain the handle; both paths must observe through the handle so
/// the description cannot belong to a different object than the one pinned.
fn observe(path: &Path) -> Option<(std::fs::File, ObservedObject)> {
    use std::os::unix::fs::MetadataExt as _;
    let handle = std::fs::File::open(path).ok()?;
    let metadata = handle.metadata().ok()?;
    let object = ObservedObject {
        device: metadata.dev(),
        inode: metadata.ino(),
        created: metadata.created().ok(),
        directory: metadata.is_dir(),
    };
    Some((handle, object))
}

fn mint() -> anyhow::Result<String> {
    let mut random = [0_u8; 16];
    std::fs::File::open("/dev/urandom")
        .context("opening OS randomness")?
        .read_exact(&mut random)
        .context("reading OS randomness")?;
    let mut value = String::with_capacity(32);
    for byte in random {
        value.push_str(&format!("{byte:02x}"));
    }
    Ok(value)
}

impl RootIdentities {
    /// Refresh every advertised root and return the identities to advertise.
    /// A root that cannot be observed right now has no identity, so the
    /// Controller refuses to build a read scope for it rather than guessing.
    pub(super) fn observe_roots(
        &mut self,
        workspaces: &[MachineWorkspace],
    ) -> Vec<WorkspaceRootIdentity> {
        let mut advertised = Vec::with_capacity(workspaces.len());
        let mut live = HashMap::with_capacity(workspaces.len());
        for workspace in workspaces {
            let path = PathBuf::from(&workspace.canonical_path);
            if live.len() >= MAX_TRACKED_ROOTS {
                tracing::warn!("advertised workspace root budget exceeded");
                break;
            }
            let Some((handle, object)) = observe(&path) else {
                continue;
            };
            if !object.directory {
                continue;
            }
            // Keep the existing identity, and its original pinning handle,
            // only for the same object. Anything else — delete/recreate, a
            // replacement worktree, a different mount at the same path — is a
            // new root and a new identity.
            let tracked = match self.roots.remove(&path) {
                Some(tracked) if tracked.object == object => tracked,
                _ => match mint() {
                    Ok(incarnation) => TrackedRoot {
                        _handle: handle,
                        object,
                        incarnation,
                    },
                    Err(error) => {
                        tracing::warn!(
                            workspace_id = workspace.id,
                            %error,
                            "workspace root identity could not be minted"
                        );
                        continue;
                    }
                },
            };
            advertised.push(WorkspaceRootIdentity {
                workspace_id: workspace.id.clone(),
                incarnation: tracked.incarnation.clone(),
            });
            live.insert(path, tracked);
        }
        // Roots that left the configuration lose their identity entirely; a
        // later re-add observes the object again and mints a new one.
        self.roots = live;
        advertised
    }

    /// Refuse before any read when the live object is not the one that minted
    /// the carried incarnation. This runs per request, so it does not depend
    /// on an inventory refresh having observed the replacement first.
    pub(super) fn verify(
        &mut self,
        root: &str,
        incarnation: &str,
    ) -> Result<(), RootIdentityRefusal> {
        let path = PathBuf::from(root);
        if !path.is_absolute() {
            return Err(RootIdentityRefusal::Unusable(
                "workspace root identity requires an absolute root",
            ));
        }
        let Some(tracked) = self.roots.get_mut(&path) else {
            return Err(RootIdentityRefusal::Unusable(
                "workspace root is not an advertised Machine root",
            ));
        };
        if tracked.incarnation != incarnation {
            return Err(RootIdentityRefusal::Changed(
                "workspace root identity is no longer current",
            ));
        }
        let Some((_handle, object)) = observe(&path) else {
            // Mint nothing for an absent root: an entry with no object would
            // silently accept the next object created at the same path.
            self.roots.remove(&path);
            return Err(RootIdentityRefusal::Changed(
                "workspace root is no longer observable",
            ));
        };
        if object != tracked.object || !object.directory {
            // Retire the carried identity immediately. The next advertisement
            // mints a fresh one; this one can never be revived.
            self.roots.remove(&path);
            return Err(RootIdentityRefusal::Changed(
                "workspace root identity is no longer current",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(id: &str, path: &Path) -> MachineWorkspace {
        MachineWorkspace {
            id: id.into(),
            display_name: id.into(),
            canonical_path: path.display().to_string(),
        }
    }

    #[test]
    fn an_unchanged_root_keeps_exactly_one_identity() {
        let root = tempfile::tempdir().expect("temp");
        let roots = [workspace("main", root.path())];
        let mut identities = RootIdentities::default();
        let first = identities.observe_roots(&roots);
        let second = identities.observe_roots(&roots);
        assert_eq!(first.len(), 1);
        assert_eq!(first, second);
        assert!(first[0].is_well_formed());
        // Unrelated content changes are not root replacement.
        std::fs::write(root.path().join("file.txt"), b"x").expect("write");
        assert_eq!(identities.observe_roots(&roots), first);
        assert!(
            identities
                .verify(&roots[0].canonical_path, &first[0].incarnation)
                .is_ok()
        );
    }

    #[test]
    fn a_replaced_root_cannot_keep_or_revive_its_identity() {
        let parent = tempfile::tempdir().expect("temp");
        let path = parent.path().join("root");
        std::fs::create_dir(&path).expect("create");
        let roots = [workspace("main", &path)];
        let mut identities = RootIdentities::default();
        let original = identities.observe_roots(&roots);
        let incarnation = original[0].incarnation.clone();
        std::fs::remove_dir_all(&path).expect("remove");
        std::fs::create_dir(&path).expect("recreate");
        // The Machine refuses before re-advertising anything: the per-request
        // check does not wait for an inventory refresh.
        let refusal = identities
            .verify(&roots[0].canonical_path, &incarnation)
            .expect_err("replaced root must refuse");
        assert!(refusal.is_changed());
        assert!(refusal.detail().contains("no longer current"));
        let replaced = identities.observe_roots(&roots);
        assert_eq!(replaced.len(), 1);
        assert_ne!(replaced[0].incarnation, incarnation);
        // The retired value stays retired even though the path is live again.
        assert!(
            identities
                .verify(&roots[0].canonical_path, &incarnation)
                .is_err()
        );
        assert!(
            identities
                .verify(&roots[0].canonical_path, &replaced[0].incarnation)
                .is_ok()
        );
    }

    /// Device and inode numbers are reused: without the retained handle this
    /// exact sequence hands a recreated directory the old inode number, and
    /// the fence would silently pass.
    #[test]
    fn the_retained_handle_is_what_makes_inode_comparison_sound() {
        use std::os::unix::fs::MetadataExt as _;
        let parent = tempfile::tempdir().expect("temp");
        let path = parent.path().join("root");
        std::fs::create_dir(&path).expect("create");
        let original = std::fs::metadata(&path).expect("stat").ino();
        // Unpinned: the kernel is free to hand back the same inode number.
        std::fs::remove_dir(&path).expect("remove");
        std::fs::create_dir(&path).expect("recreate");
        let unpinned = std::fs::metadata(&path).expect("stat").ino();
        let mut identities = RootIdentities::default();
        assert_eq!(
            identities.observe_roots(&[workspace("main", &path)]).len(),
            1
        );
        std::fs::remove_dir(&path).expect("remove");
        std::fs::create_dir(&path).expect("recreate");
        let pinned = std::fs::metadata(&path).expect("stat").ino();
        assert_ne!(
            pinned, unpinned,
            "the tracked handle must reserve {unpinned}"
        );
        assert!(original == unpinned || pinned != original);
    }

    #[test]
    fn a_removed_root_has_no_identity_and_no_tracked_entry() {
        let parent = tempfile::tempdir().expect("temp");
        let path = parent.path().join("root");
        std::fs::create_dir(&path).expect("create");
        let roots = [workspace("main", &path)];
        let mut identities = RootIdentities::default();
        let original = identities.observe_roots(&roots);
        std::fs::remove_dir(&path).expect("remove");
        assert!(identities.observe_roots(&roots).is_empty());
        assert!(
            identities
                .verify(&roots[0].canonical_path, &original[0].incarnation)
                .is_err()
        );
        // A file at the same path is not a workspace root either.
        std::fs::write(&path, b"x").expect("write");
        assert!(identities.observe_roots(&roots).is_empty());
    }

    #[test]
    fn unadvertised_relative_and_foreign_roots_are_refused() {
        let root = tempfile::tempdir().expect("temp");
        let other = tempfile::tempdir().expect("temp");
        let roots = [workspace("main", root.path())];
        let mut identities = RootIdentities::default();
        let advertised = identities.observe_roots(&roots);
        // An unusable carry is a Controller/protocol error, never evidence
        // that this Machine ended an observation.
        for unusable in [
            identities.verify("relative/path", "x").unwrap_err(),
            identities
                .verify(
                    &other.path().display().to_string(),
                    &advertised[0].incarnation,
                )
                .unwrap_err(),
        ] {
            assert!(!unusable.is_changed());
        }
        // A wrong value for an advertised root did end an observation.
        assert!(
            identities
                .verify(&roots[0].canonical_path, "not-the-minted-value")
                .unwrap_err()
                .is_changed()
        );
    }

    #[test]
    fn dropping_a_root_from_the_configuration_ends_its_identity() {
        let first = tempfile::tempdir().expect("temp");
        let second = tempfile::tempdir().expect("temp");
        let both = [workspace("a", first.path()), workspace("b", second.path())];
        let mut identities = RootIdentities::default();
        let advertised = identities.observe_roots(&both);
        assert_eq!(advertised.len(), 2);
        let dropped = advertised
            .iter()
            .find(|entry| entry.workspace_id == "b")
            .expect("b")
            .clone();
        let kept = advertised
            .iter()
            .find(|entry| entry.workspace_id == "a")
            .expect("a")
            .clone();
        let only_first = [both[0].clone()];
        assert_eq!(identities.observe_roots(&only_first), vec![kept.clone()]);
        assert!(
            identities
                .verify(&both[1].canonical_path, &dropped.incarnation)
                .is_err()
        );
        // The unrelated root is independent and keeps its original identity.
        assert!(
            identities
                .verify(&both[0].canonical_path, &kept.incarnation)
                .is_ok()
        );
        // Re-adding the dropped root mints a new identity, never the old one.
        let readded = identities.observe_roots(&both);
        assert_ne!(
            readded
                .iter()
                .find(|entry| entry.workspace_id == "b")
                .expect("b")
                .incarnation,
            dropped.incarnation
        );
    }

    #[test]
    fn minted_identities_are_unique_and_well_formed() {
        let first = tempfile::tempdir().expect("temp");
        let second = tempfile::tempdir().expect("temp");
        let mut identities = RootIdentities::default();
        let advertised = identities
            .observe_roots(&[workspace("a", first.path()), workspace("b", second.path())]);
        assert_eq!(advertised.len(), 2);
        assert_ne!(advertised[0].incarnation, advertised[1].incarnation);
        for entry in &advertised {
            assert!(entry.is_well_formed());
            assert_eq!(entry.incarnation.len(), 32);
        }
    }
}
