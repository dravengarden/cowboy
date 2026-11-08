//! Agent-owned mirror refresh for `bufferLanguage`.
//!
//! A buffer that only automatic agent acquisitions own never receives edits,
//! so its native mirror is the text read at Open. When the bounded source
//! observation differs, the agent may reload that mirror through the existing
//! bounded native reload route. Any other owner (an interactive lease, legacy
//! owner or navigation), a synchronization reservation or an unresolved close
//! refuses the reload. This never saves, opens, retries or grants
//! synchronization authority; a refused or unknown reload leaves the ordinary
//! observation, which still reports the source mismatch.
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{BufferLease, BufferOwner, Buffers, Zed, source_snapshot, sync_owners};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Refresh {
    AgentOwned,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Outcome {
    /// The source matched the mirror, or could not be observed as changed.
    NotNeeded,
    /// The native reload completed; later observations show its text.
    Reloaded,
    /// Another owner, a fence, or a native refusal left the mirror unchanged.
    Refused,
    /// The native reload outcome was not observed. Nothing is retried.
    Unknown,
}

const REGISTRY_WAIT: Duration = Duration::from_secs(2);
const RELOAD_WAIT: Duration = Duration::from_secs(5);

/// Owner IDs created by automatic acquisitions. Missing or busy registries
/// yield `None`, which refuses rather than guessing ownership.
async fn agent_owners(buffers: &Buffers) -> Option<HashSet<u64>> {
    let acquisitions = tokio::time::timeout(REGISTRY_WAIT, buffers.acquisitions.lock())
        .await
        .ok()?;
    let references = acquisitions.agent_leases();
    drop(acquisitions);
    let leases = tokio::time::timeout(REGISTRY_WAIT, buffers.leases.lock())
        .await
        .ok()?;
    Some(
        references
            .iter()
            .filter_map(|reference| leases.issued(reference))
            .collect(),
    )
}

/// Whether every owner of this buffer is an automatic agent acquisition.
pub(crate) fn agent_only(lease: &BufferLease, agents: &HashSet<u64>) -> bool {
    sync_owners::ensure_readable(lease).is_ok()
        && !lease.lease_ids.is_empty()
        && lease
            .lease_ids
            .iter()
            .all(|owner| matches!(owner, BufferOwner::Owned(id) if agents.contains(id)))
}

pub(crate) async fn refresh(
    root: &Path,
    source: &Path,
    key: &(PathBuf, PathBuf),
    buffers: &Buffers,
    zed: &Zed,
) -> Outcome {
    let Some(agents) = agent_owners(buffers).await else {
        return Outcome::Refused;
    };
    // Held through the reload: no owner can join or leave meanwhile.
    let all = buffers.active.read().await;
    let Some(lease) = all.get(key).filter(|lease| agent_only(lease, &agents)) else {
        return Outcome::Refused;
    };
    let Ok(observation) = zed.language(lease.remote_id).await else {
        return Outcome::Refused;
    };
    let state = source_snapshot::check(
        root,
        source,
        observation
            .snapshot
            .as_ref()
            .map(|snapshot| &snapshot.content),
    )
    .await;
    if state != source_snapshot::State::Changed {
        return Outcome::NotNeeded;
    }
    let reload = zed.request(proto::envelope::Payload::ReloadBuffers(
        proto::ReloadBuffers {
            project_id: proto::REMOTE_SERVER_PROJECT_ID,
            buffer_ids: vec![lease.remote_id],
        },
    ));
    match tokio::time::timeout(RELOAD_WAIT, reload).await {
        Ok(Ok(reply))
            if matches!(
                reply.payload,
                Some(proto::envelope::Payload::ReloadBuffersResponse(_))
            ) =>
        {
            Outcome::Reloaded
        }
        // Native refusal (size, binary, symlink) leaves text and version
        // unchanged; see the reload input bounds.
        Ok(Err(error)) if error.to_string().starts_with("Zed request failed:") => Outcome::Refused,
        _ => Outcome::Unknown,
    }
}

#[cfg(test)]
mod tests;
