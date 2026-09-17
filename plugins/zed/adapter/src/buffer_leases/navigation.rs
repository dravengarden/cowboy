//! Effect-free reservation followed by atomic transfer of an existing native
//! destination into an ordinary owner. No native open, registration or disk I/O.
use super::*;
use crate::buffer_navigation::NavigationRef;
use crate::content_reads::Content;

impl Registry {
    pub(crate) async fn prepare_navigation(
        &mut self,
        navigation: NavigationRef,
        destination: u32,
        content: Content,
        buffers: &Buffers,
        zed: Option<&Zed>,
    ) -> Result<Response> {
        let zed = zed.context("native navigation unavailable")?;
        // All callers hold the ordinary lease registry first. Navigation never
        // takes that registry, keeping one lock order through handoff/release.
        let mut navigations = buffers.navigations.lock().await;
        let (owner, target) = navigations.destination(&navigation, destination, &content)?;
        let active = buffers.active.read().await;
        crate::sync_owners::ensure_admission(&active)?;
        target.check_owner(owner, &active)?;
        let cache = zed.diagnostics.lock().expect("diagnostic cache poisoned");
        cache.check(target.remote_id, target.revision)?;
        self.insert(Slot {
            worktree: target.key.0.clone(),
            path: target.key.1.clone(),
            origin: Origin::Navigation {
                navigation,
                destination,
                content,
            },
            prepared_at: Instant::now(),
            state: Phase::Prepared,
        })
    }
}

pub(super) async fn open(
    slot: &mut Slot,
    id: u64,
    buffers: &Buffers,
    zed: Option<&Zed>,
) -> Result<()> {
    let Origin::Navigation {
        navigation,
        destination,
        content,
    } = &slot.origin
    else {
        unreachable!()
    };
    let zed = zed.context("native navigation unavailable")?;
    let mut navigations = buffers.navigations.lock().await;
    let (owner, target) = navigations.destination(navigation, *destination, content)?;
    let mut active = buffers.active.write().await;
    ensure!(
        slot.prepared_at.elapsed() < PREPARE_TTL,
        "buffer preparation expired while queued"
    );
    crate::sync_owners::ensure_admission(&active)?;
    target.check_owner(owner, &active)?;
    ensure!(
        target.key == (slot.worktree.clone(), slot.path.clone()),
        "original navigation target changed"
    );
    let cache = zed.diagnostics.lock().expect("diagnostic cache poisoned");
    cache.check(target.remote_id, target.revision)?;
    // No await from the final epoch check through both ownership updates. A
    // cancelled waiter has no effect; an admitted handoff is locally observable
    // by its preallocated lease even when the socket response is lost.
    active
        .get_mut(&target.key)
        .expect("validated original target")
        .lease_ids
        .insert(BufferOwner::Owned(id));
    slot.state = Phase::Open;
    Ok(())
}
