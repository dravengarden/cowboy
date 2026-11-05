//! Per-server analysis evidence for a buffer's diagnostics. The private server
//! reports which document version each server's retained diagnostics set was
//! computed against; this compares that with the mirror the diagnostics are
//! read from. `document_current` covers one document's text only, never the
//! project's other inputs, so nothing here ever claims project freshness.
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Result, bail};
use serde::Serialize;

use crate::BufferVersionEntry;
use crate::sync_native::wire::{self, cowboy_buffer_sync_envelope::Payload};

const MAX_SERVERS: usize = 32;
/// A server without this extension never answers; do not hold the read.
const FRONTIER_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum State {
    DocumentCurrent,
    Stale,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Reason {
    /// The server has no retained diagnostics set for this buffer.
    Unpublished,
    /// The set read here and the one the server reported differ.
    InFlight,
    /// The server did not name the version it analyzed.
    Unversioned,
    /// Save-triggered analysis of saved content.
    SaveTriggered,
    /// The set mixes entries from an earlier analysis.
    Merged,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ServerAnalysis {
    pub(crate) server_id: u64,
    pub(crate) name: String,
    pub(crate) state: State,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<Reason>,
}

type Clock = BTreeMap<u32, u32>;

fn clock(entries: impl IntoIterator<Item = (u32, u32)>) -> Clock {
    let mut clock = Clock::new();
    for (replica, timestamp) in entries {
        if timestamp > 0 {
            let value = clock.entry(replica).or_default();
            *value = (*value).max(timestamp);
        }
    }
    clock
}

fn observed_by(earlier: &Clock, later: &Clock) -> bool {
    earlier
        .iter()
        .all(|(replica, timestamp)| later.get(replica).is_some_and(|seen| seen >= timestamp))
}

fn wire_clock(entries: &[wire::CowboyBufferSyncVersion]) -> Clock {
    clock(
        entries
            .iter()
            .map(|entry| (entry.replica_id, entry.timestamp)),
    )
}

/// One server's evidence against the mirror version and the stamp of the set
/// the mirror holds for it (`(lamport, replica)`).
pub(crate) fn classify(
    server: &wire::CowboyAnalysisServer,
    held: Option<(u32, u32)>,
    mirror: &[BufferVersionEntry],
) -> (State, Option<Reason>) {
    if !server.published {
        return (State::Unknown, Some(Reason::Unpublished));
    }
    if held != Some((server.lamport_timestamp, server.replica_id)) {
        return (State::Unknown, Some(Reason::InFlight));
    }
    let mirror = clock(
        mirror
            .iter()
            .map(|entry| (entry.replica_id, entry.timestamp)),
    );
    let analyzed = wire_clock(&server.analyzed);
    if !observed_by(&analyzed, &mirror) {
        return (State::Unknown, Some(Reason::InFlight));
    }
    if analyzed != mirror {
        // Analysis is bounded by the publication version, which the text has
        // moved past: whatever the server analyzed is older than the content.
        return (State::Stale, None);
    }
    if server.disk_based {
        return if wire_clock(&server.saved) == mirror {
            (State::Unknown, Some(Reason::SaveTriggered))
        } else {
            (State::Stale, Some(Reason::SaveTriggered))
        };
    }
    if !server.versioned {
        return (State::Unknown, Some(Reason::Unversioned));
    }
    if server.reused {
        return (State::Unknown, Some(Reason::Merged));
    }
    (State::DocumentCurrent, None)
}

pub(crate) struct Frontier {
    supported: AtomicBool,
}

impl Default for Frontier {
    fn default() -> Self {
        Self {
            supported: AtomicBool::new(true),
        }
    }
}

impl Frontier {
    /// The server's evidence, or `None` when it has no such extension.
    pub(crate) async fn request(
        &self,
        zed: &crate::ZedRuntime,
        buffer_id: u64,
    ) -> Result<Option<Vec<wire::CowboyAnalysisServer>>> {
        if !self.supported.load(Ordering::Relaxed) {
            return Ok(None);
        }
        let request = Payload::AnalysisRequest(wire::CowboyAnalysisFrontier {
            project_id: proto::REMOTE_SERVER_PROJECT_ID,
            protocol: 1,
            buffer_id,
        });
        let Ok(reply) =
            tokio::time::timeout(FRONTIER_TIMEOUT, zed.sync.exchange(zed, request)).await
        else {
            self.supported.store(false, Ordering::Relaxed);
            return Ok(None);
        };
        let Payload::AnalysisResponse(response) = reply? else {
            bail!("unexpected analysis frontier response kind");
        };
        validate(&response)?;
        Ok(Some(response.servers))
    }
}

fn validate(response: &wire::CowboyAnalysisFrontierResponse) -> Result<()> {
    anyhow::ensure!(response.protocol == 1, "invalid analysis frontier response");
    anyhow::ensure!(
        response.servers.len() <= MAX_SERVERS,
        "analysis frontier exceeds limits"
    );
    let mut seen = std::collections::BTreeSet::new();
    for server in &response.servers {
        anyhow::ensure!(
            seen.insert(server.server_id)
                && server.name.len() <= 128
                && server.analyzed.len() <= 64
                && server.saved.len() <= 64,
            "invalid analysis frontier server"
        );
    }
    Ok(())
}

pub(crate) fn project(
    servers: &[wire::CowboyAnalysisServer],
    held: impl Fn(u64) -> Option<(u32, u32)>,
    mirror: &[BufferVersionEntry],
) -> Vec<ServerAnalysis> {
    servers
        .iter()
        .map(|server| {
            let (state, reason) = classify(server, held(server.server_id), mirror);
            ServerAnalysis {
                server_id: server.server_id,
                name: server.name.clone(),
                state,
                reason,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(replica_id: u32, timestamp: u32) -> BufferVersionEntry {
        BufferVersionEntry {
            replica_id,
            timestamp,
        }
    }

    fn version(entries: &[(u32, u32)]) -> Vec<wire::CowboyBufferSyncVersion> {
        entries
            .iter()
            .map(|(replica_id, timestamp)| wire::CowboyBufferSyncVersion {
                replica_id: *replica_id,
                timestamp: *timestamp,
            })
            .collect()
    }

    fn published(analyzed: &[(u32, u32)]) -> wire::CowboyAnalysisServer {
        wire::CowboyAnalysisServer {
            server_id: 1,
            name: "rust-analyzer".into(),
            published: true,
            replica_id: 0,
            lamport_timestamp: 9,
            versioned: true,
            analyzed: version(analyzed),
            saved: version(analyzed),
            ..Default::default()
        }
    }

    const HELD: Option<(u32, u32)> = Some((9, 0));

    #[test]
    fn exact_versioned_analysis_of_the_mirror_is_document_current() {
        let mirror = [entry(0, 4), entry(1, 2)];
        // Zero entries and order do not change a version.
        let server = published(&[(1, 2), (0, 4), (2, 0)]);
        assert_eq!(
            classify(&server, HELD, &mirror),
            (State::DocumentCurrent, None)
        );
    }

    #[test]
    fn analysis_of_an_older_version_is_stale_whatever_its_kind() {
        let mirror = [entry(0, 5)];
        for (versioned, disk_based) in [(true, false), (false, false), (false, true)] {
            let mut server = published(&[(0, 4)]);
            server.versioned = versioned;
            server.disk_based = disk_based;
            assert_eq!(classify(&server, HELD, &mirror).0, State::Stale);
        }
    }

    #[test]
    fn weaker_evidence_is_never_current() {
        let mirror = [entry(0, 4)];
        let mut server = published(&[(0, 4)]);
        server.versioned = false;
        assert_eq!(
            classify(&server, HELD, &mirror),
            (State::Unknown, Some(Reason::Unversioned))
        );
        let mut server = published(&[(0, 4)]);
        server.reused = true;
        assert_eq!(
            classify(&server, HELD, &mirror),
            (State::Unknown, Some(Reason::Merged))
        );
        let mut server = published(&[(0, 4)]);
        server.disk_based = true;
        assert_eq!(
            classify(&server, HELD, &mirror),
            (State::Unknown, Some(Reason::SaveTriggered))
        );
        server.saved = version(&[(0, 3)]);
        assert_eq!(
            classify(&server, HELD, &mirror),
            (State::Stale, Some(Reason::SaveTriggered))
        );
    }

    #[test]
    fn mismatched_sets_and_unseen_versions_are_in_flight() {
        let mirror = [entry(0, 4)];
        let server = published(&[(0, 4)]);
        for held in [None, Some((8, 0)), Some((9, 1))] {
            assert_eq!(
                classify(&server, held, &mirror),
                (State::Unknown, Some(Reason::InFlight))
            );
        }
        // The server analyzed edits this mirror has not applied yet.
        let ahead = published(&[(0, 5)]);
        assert_eq!(
            classify(&ahead, HELD, &mirror),
            (State::Unknown, Some(Reason::InFlight))
        );
        let concurrent = published(&[(0, 4), (1, 1)]);
        assert_eq!(classify(&concurrent, HELD, &mirror).0, State::Unknown);
        let mut unpublished = published(&[(0, 4)]);
        unpublished.published = false;
        assert_eq!(
            classify(&unpublished, HELD, &mirror),
            (State::Unknown, Some(Reason::Unpublished))
        );
    }

    #[test]
    fn owned_language_observations_never_carry_analysis() {
        // Installed Machines parse owned reads with deny_unknown_fields.
        let observation = crate::LanguageObservation {
            analysis: Some(Vec::new()),
            ..Default::default()
        };
        let value = serde_json::to_value(&observation).unwrap();
        assert!(value.get("analysis").is_none(), "{value}");
    }

    #[test]
    fn responses_are_bounded_and_unique() {
        let ok = wire::CowboyAnalysisFrontierResponse {
            protocol: 1,
            servers: vec![published(&[(0, 1)])],
        };
        assert!(validate(&ok).is_ok());
        let mut duplicate = ok.clone();
        duplicate.servers.push(published(&[(0, 1)]));
        assert!(validate(&duplicate).is_err());
        let mut wrong = ok.clone();
        wrong.protocol = 2;
        assert!(validate(&wrong).is_err());
        let mut many = ok;
        many.servers = (0..33)
            .map(|id| wire::CowboyAnalysisServer {
                server_id: id,
                ..Default::default()
            })
            .collect();
        assert!(validate(&many).is_err());
    }
}
