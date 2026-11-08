//! Commit one original native Open into its checked buffer owner.
use crate::{ADAPTER_VERSION, BufferLease, BufferOwner, Buffers, Response, Zed, native_open};
use anyhow::Result;
use std::collections::HashSet;
use std::path::PathBuf;
pub(crate) async fn open_buffer_at(
    worktree: PathBuf,
    path: PathBuf,
    worktree_id: u64,
    owner: BufferOwner,
    buffers: &Buffers,
    zed: Option<&Zed>,
    admission: native_open::Admission<'_>,
) -> Result<Response> {
    let key = (worktree.clone(), path.clone());
    let mut all = buffers.active.write().await;
    if let Some(id) = admission.expected_id {
        anyhow::ensure!(
            all.get(&key).is_some_and(|buffer| buffer.remote_id == id),
            "original legacy buffer is no longer retained"
        );
    }
    if let Some(zed) = zed {
        native_open::recover(&buffers.native_open, &all, zed).await?;
    }
    native_open::admit(buffers, &all, zed, &key, worktree_id).await?;
    let mut attempt = None;
    if !all.contains_key(&key) {
        let (remote_id, version) = if let Some(zed) = zed {
            // Arm before any native I/O. Cancellation or an error leaves the
            // process-wide fence latched until `recover` learns the outcome.
            let armed = attempt.insert(buffers.native_open.begin()?);
            if let Some(receipt) = admission.recovery {
                armed.track(receipt);
            }
            zed.open_buffer_tracked(worktree_id, &path, |id| armed.dispatching(id))
                .await?
        } else {
            (u64::try_from(all.len() + 1)?, Vec::new())
        };
        all.insert(
            key.clone(),
            BufferLease {
                lease_ids: HashSet::new(),
                remote_id,
                version,
                sync: None,
                closing: false,
            },
        );
    }
    let lease = all.get_mut(&key).expect("buffer was just inserted");
    lease.lease_ids.insert(owner);
    let response = Response::Buffer {
        api_version: ADAPTER_VERSION,
        worktree,
        path,
        leases: lease.lease_ids.len(),
        buffer_id: lease.remote_id,
        version: lease.version.clone(),
    };
    // No await between original-ID owner commit and ending its admission fence.
    if let Some(attempt) = attempt {
        attempt.complete();
    }
    Ok(response)
}
