// SPDX-License-Identifier: GPL-3.0-or-later
//! Private analysis-frontier evidence: for each diagnostics set a language
//! server published into a buffer, which document version it was computed
//! against. Zed maps a versioned LSP publication onto that exact snapshot; an
//! unversioned one onto the text current at publication, which bounds the
//! analyzed content from above. Disk-based (save-triggered) entries analyzed
//! saved content. This is evidence for one document only, never for the
//! project's other inputs.
use super::*;

/// Entries retained across all buffers; the oldest publication is forgotten
/// first and then answers as unpublished, never as current.
const MAX_ENTRIES: usize = 8_192;
const MAX_SERVERS: usize = 32;
const MAX_NAME: usize = 128;

#[cfg(test)]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Evidence {
    /// The diagnostics operation that carried this set.
    pub(super) stamp: clock::Lamport,
    /// The server named the document version it analyzed.
    pub(super) versioned: bool,
    /// Analyzed version when versioned; otherwise the version at publication.
    pub(super) published: clock::Global,
    pub(super) saved: clock::Global,
    /// New entries include disk-based (save-triggered) diagnostics.
    pub(super) disk_based: bool,
    /// The set keeps entries carried over from an earlier analysis.
    pub(super) reused: bool,
}

#[derive(Default)]
pub(super) struct State {
    entries: HashMap<(BufferId, LanguageServerId), (u64, Evidence)>,
    sequence: u64,
}

impl State {
    pub(super) fn record(
        &mut self,
        buffer: BufferId,
        server: LanguageServerId,
        evidence: Evidence,
    ) {
        self.sequence += 1;
        let key = (buffer, server);
        if self.entries.len() >= MAX_ENTRIES && !self.entries.contains_key(&key) {
            if let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, (sequence, _))| *sequence)
                .map(|(key, _)| *key)
            {
                self.entries.remove(&oldest);
            }
        }
        self.entries.insert(key, (self.sequence, evidence));
    }

    pub(super) fn get(&self, buffer: BufferId, server: LanguageServerId) -> Option<&Evidence> {
        self.entries
            .get(&(buffer, server))
            .map(|(_, evidence)| evidence)
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

fn version(value: &clock::Global) -> Vec<proto::CowboyBufferSyncVersion> {
    language::proto::serialize_version(value)
        .into_iter()
        .map(|entry| proto::CowboyBufferSyncVersion {
            replica_id: entry.replica_id,
            timestamp: entry.timestamp,
        })
        .collect()
}

/// A named version is evidence only while its exact snapshot is retained.
/// Upstream accepts version zero without snapshots as an unversioned update;
/// that compatibility fallback must not become a freshness proof.
pub(super) fn tracked_version(
    local: &LocalLspStore,
    buffer: &Entity<Buffer>,
    server: LanguageServerId,
    version: Option<i32>,
    cx: &App,
) -> bool {
    version.is_some_and(|version| {
        local
            .buffer_snapshots
            .get(&buffer.read(cx).remote_id())
            .and_then(|servers| servers.get(&server))
            .is_some_and(|snapshots| snapshots.iter().any(|snapshot| snapshot.version == version))
    })
}

pub(super) fn server(
    id: LanguageServerId,
    name: &str,
    evidence: Option<&Evidence>,
) -> proto::CowboyAnalysisServer {
    let mut name = name.to_owned();
    if name.len() > MAX_NAME {
        let mut end = MAX_NAME;
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        name.truncate(end);
    }
    let mut value = proto::CowboyAnalysisServer {
        server_id: id.to_proto(),
        name,
        ..Default::default()
    };
    if let Some(evidence) = evidence {
        value.published = true;
        value.replica_id = evidence.stamp.replica_id.as_u16().into();
        value.lamport_timestamp = evidence.stamp.value;
        value.versioned = evidence.versioned;
        value.analyzed = version(&evidence.published);
        value.saved = version(&evidence.saved);
        value.disk_based = evidence.disk_based;
        value.reused = evidence.reused;
    }
    value
}

impl LspStore {
    /// The document version each server last received for this buffer. A
    /// pull request is answered for the state the server holds when it
    /// handles it, which follows every earlier change notification.
    pub(super) fn cowboy_pull_versions(
        &self,
        buffer: &Entity<Buffer>,
        cx: &App,
    ) -> HashMap<LanguageServerId, i32> {
        let id = buffer.read(cx).remote_id();
        self.as_local()
            .and_then(|local| local.buffer_snapshots.get(&id))
            .map(|servers| {
                servers
                    .iter()
                    .filter_map(|(server, snapshots)| Some((*server, snapshots.last()?.version)))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(super) async fn handle_cowboy_analysis_frontier(
        this: Entity<Self>,
        envelope: TypedEnvelope<proto::CowboyAnalysisFrontier>,
        mut cx: AsyncApp,
    ) -> Result<proto::CowboyAnalysisFrontierResponse> {
        let sender = envelope.original_sender_id().unwrap_or_default();
        let request = envelope.payload;
        anyhow::ensure!(
            request.protocol == 1 && request.project_id == proto::REMOTE_SERVER_PROJECT_ID,
            "invalid private analysis frontier request"
        );
        let buffer_id = BufferId::new(request.buffer_id)?;
        this.update(&mut cx, |this, cx| {
            let buffer = this
                .buffer_store
                .read(cx)
                .get(buffer_id)
                .context("unknown buffer")?;
            anyhow::ensure!(
                this.buffer_store
                    .read(cx)
                    .cowboy_navigation_owner(sender, &buffer, cx),
                "buffer is not shared with this peer"
            );
            let servers = buffer.update(cx, |buffer, cx| {
                this.language_servers_for_local_buffer(buffer, cx)
            });
            anyhow::ensure!(servers.len() <= MAX_SERVERS, "too many language servers");
            let local = this
                .as_local()
                .context("analysis frontier requires local store")?;
            Ok(proto::CowboyAnalysisFrontierResponse {
                protocol: 1,
                servers: servers
                    .into_iter()
                    .map(|id| {
                        let name = this
                            .language_server_adapter_for_id(id)
                            .map(|adapter| adapter.name.0.to_string())
                            .unwrap_or_default();
                        server(id, &name, local.cowboy_frontier.get(buffer_id, id))
                    })
                    .collect(),
            })
        })
    }
}
