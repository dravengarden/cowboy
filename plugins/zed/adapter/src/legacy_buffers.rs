//! Legacy callers repeat path/owner pairs, not native lease handles. Retain
//! the first resolved key before dispatch so Close never re-resolves a file
//! that was deleted or retargeted. The registry lock serializes legacy calls;
//! cancellation preserves its pin, including an unresolved native Open.
use super::*;

mod release;
pub(super) use release::Capture;

pub(super) async fn open_buffer(
    worktree: PathBuf,
    path: PathBuf,
    lease_id: String,
    worktrees: &Worktrees,
    buffers: &Buffers,
    zed: Option<&Zed>,
) -> Result<Response> {
    validate_lease_id(&lease_id)?;
    let requested = (worktree.clone(), path.clone(), lease_id.clone());
    let mut legacy = buffers.legacy.lock().await;
    let (worktree, path) = buffer_key(worktree, path).await?;
    let trees = worktrees.read().await;
    let worktree_id = trees
        .get(&worktree)
        .context("worktree is not open")?
        .remote_id;
    legacy.pin(requested.clone(), (worktree.clone(), path.clone()))?;
    let expected_id = legacy.get(&requested).and_then(|pin| pin.buffer_id);
    let response = open_buffer_at(
        worktree,
        path,
        worktree_id,
        BufferOwner::Legacy(lease_id),
        buffers,
        zed,
        native_open::Admission {
            expected_id,
            ..Default::default()
        },
    )
    .await?;
    if let Response::Buffer { buffer_id, .. } = &response {
        legacy
            .0
            .get_mut(&requested)
            .expect("reserved before dispatch")
            .buffer_id = Some(*buffer_id);
    }
    Ok(response)
}

pub(super) async fn close_buffer(
    worktree: PathBuf,
    path: PathBuf,
    lease_id: String,
    buffers: &Buffers,
    zed: Option<&Zed>,
) -> Result<Response> {
    validate_lease_id(&lease_id)?;
    let requested = (worktree.clone(), path.clone(), lease_id.clone());
    let mut legacy = buffers.legacy.lock().await;
    let mut active = buffers.active.write().await;
    let Some(pin) = legacy.get(&requested).cloned() else {
        if let Some(buffer) = active.get(&(worktree.clone(), path.clone())) {
            sync_owners::ensure_readable(buffer)?;
        }
        return Ok(Response::Buffer {
            api_version: ADAPTER_VERSION,
            worktree,
            path,
            leases: 0,
            buffer_id: 0,
            version: Vec::new(),
        });
    };
    anyhow::ensure!(
        pin.release.is_none(),
        "original legacy release remains fenced"
    );
    // An interrupted Open may have acquired a native object before the
    // owner was committed. Resolve that exact attempt before claiming release.
    if let Some(zed) = zed {
        native_open::recover(&buffers.native_open, &active, zed).await?;
    }
    let key = pin.key;
    let owner = BufferOwner::Legacy(lease_id.clone());
    let (leases, buffer_id, version) = if let Some(buffer) = active.get(&key) {
        if let Some(id) = pin.buffer_id {
            anyhow::ensure!(
                id == buffer.remote_id,
                "original legacy buffer identity changed"
            );
        } else {
            anyhow::ensure!(
                !buffer.lease_ids.contains(&owner),
                "unconfirmed legacy owner cannot be released"
            );
        }
        let result = (
            buffer.lease_ids.len() - usize::from(buffer.lease_ids.contains(&owner)),
            buffer.remote_id,
            buffer.version.clone(),
        );
        if pin.buffer_id.is_some() {
            native_close::release(
                HashSet::from([key.clone()]),
                &owner,
                &mut active,
                zed,
                &buffers.native_open,
                || {},
            )
            .await?;
        }
        result
    } else {
        (0, 0, Vec::new())
    };
    let response = Response::Buffer {
        api_version: ADAPTER_VERSION,
        worktree: key.0.clone(),
        path: key.1.clone(),
        leases,
        buffer_id,
        version,
    };
    legacy.released(&key, &lease_id);
    Ok(response)
}

type RequestKey = (PathBuf, PathBuf, String);
type NativeKey = (PathBuf, PathBuf);

#[derive(Default)]
pub(super) struct Registry(HashMap<RequestKey, Pin>);

#[derive(Clone)]
struct Pin {
    key: NativeKey,
    buffer_id: Option<u64>,
    incarnation: Arc<()>,
    release: Option<u64>,
}

impl Registry {
    pub(super) fn pin(&mut self, request: RequestKey, key: NativeKey) -> Result<()> {
        anyhow::ensure!(
            !self.0.iter().any(|((_, _, owner), pin)| owner == &request.2
                && pin.key == key
                && pin.release.is_some()),
            "original legacy release remains fenced"
        );
        if let Some(original) = self.0.get(&request) {
            anyhow::ensure!(
                original.key == key,
                "legacy buffer target changed before release"
            );
        } else {
            anyhow::ensure!(self.0.len() < 4096, "legacy buffer registry is full");
            self.0.insert(
                request,
                Pin {
                    key,
                    buffer_id: None,
                    incarnation: Arc::default(),
                    release: None,
                },
            );
        }
        Ok(())
    }

    fn get(&self, request: &RequestKey) -> Option<&Pin> {
        self.0.get(request)
    }

    pub(super) fn released(&mut self, key: &NativeKey, owner: &str) {
        // Aliases owned by the same legacy owner share one native holding.
        self.0
            .retain(|(_, _, held_owner), held| held_owner != owner || &held.key != key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Temp(PathBuf);
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[tokio::test]
    async fn close_uses_original_target_after_symlink_replacement_and_deletion() {
        let mut nonce = [0; 16];
        getrandom::fill(&mut nonce).unwrap();
        let temp = Temp(
            std::env::temp_dir().join(format!("cw-legacy-{:032x}", u128::from_be_bytes(nonce))),
        );
        std::fs::create_dir(&temp.0).unwrap();
        let root = temp.0.canonicalize().unwrap();
        std::fs::write(root.join("original.rs"), "old").unwrap();
        std::fs::write(root.join("replacement.rs"), "new").unwrap();
        std::os::unix::fs::symlink("original.rs", root.join("alias.rs")).unwrap();
        let worktrees: Worktrees = Arc::default();
        let buffers: Buffers = Arc::default();
        respond(
            Request::EnsureWorktree {
                path: root.clone(),
                trusted: true,
            },
            &worktrees,
            &buffers,
            None,
        )
        .await
        .unwrap();
        for (path, owner) in [("alias.rs", "agent"), ("replacement.rs", "pane")] {
            open_buffer(
                root.clone(),
                path.into(),
                owner.into(),
                &worktrees,
                &buffers,
                None,
            )
            .await
            .unwrap();
        }
        std::fs::remove_file(root.join("alias.rs")).unwrap();
        std::os::unix::fs::symlink("replacement.rs", root.join("alias.rs")).unwrap();
        assert!(
            open_buffer(
                root.clone(),
                "alias.rs".into(),
                "agent".into(),
                &worktrees,
                &buffers,
                None
            )
            .await
            .is_err()
        );
        std::fs::remove_file(root.join("original.rs")).unwrap();
        close_buffer(
            root.clone(),
            "alias.rs".into(),
            "agent".into(),
            &buffers,
            None,
        )
        .await
        .unwrap();
        let active = buffers.active.read().await;
        assert!(!active.contains_key(&(root.clone(), "original.rs".into())));
        assert!(
            active[&(root.clone(), "replacement.rs".into())]
                .lease_ids
                .contains(&BufferOwner::Legacy("pane".into()))
        );
        drop(active);
        std::fs::remove_file(root.join("replacement.rs")).unwrap();
        close_buffer(
            root.clone(),
            "replacement.rs".into(),
            "pane".into(),
            &buffers,
            None,
        )
        .await
        .unwrap();
        assert!(buffers.active.read().await.is_empty());
        assert!(buffers.legacy.lock().await.0.is_empty());
    }

    #[tokio::test]
    async fn replacement_native_identity_cannot_be_readopted_or_closed() {
        let root = std::env::current_dir().unwrap().canonicalize().unwrap();
        let worktrees: Worktrees = Arc::default();
        let buffers: Buffers = Arc::default();
        respond(
            Request::EnsureWorktree {
                path: root.clone(),
                trusted: true,
            },
            &worktrees,
            &buffers,
            None,
        )
        .await
        .unwrap();
        open_buffer(
            root.clone(),
            "Cargo.toml".into(),
            "agent".into(),
            &worktrees,
            &buffers,
            None,
        )
        .await
        .unwrap();
        buffers
            .active
            .write()
            .await
            .get_mut(&(root.clone(), "Cargo.toml".into()))
            .unwrap()
            .remote_id += 1;
        assert!(
            open_buffer(
                root.clone(),
                "Cargo.toml".into(),
                "agent".into(),
                &worktrees,
                &buffers,
                None
            )
            .await
            .is_err()
        );
        assert!(
            close_buffer(root, "Cargo.toml".into(), "agent".into(), &buffers, None)
                .await
                .is_err()
        );
        assert_eq!(buffers.active.read().await.len(), 1);
        assert_eq!(buffers.legacy.lock().await.0.len(), 1);
    }
}
