//! Content-matched diagnostic baselines for `cowboy.code.diagnostics`.
//!
//! Each `document_current` read of a file captures an immutable observation
//! bound to its exact text identity. When the text changes, the previous
//! capture becomes the baseline and the reply compares against it. Without a
//! compatible baseline the comparison is `unknown`; it is never clean.
//!
//! Identity is the native server, source kind, registration, source and code.
//! Severity and message are mutable attributes: escalation is reported, a new
//! wording is not a new diagnostic. Matching is multiplicity-aware and has no
//! text-version anchor mapping, so a moved instance persists and a new one of
//! an already present identity is reported as an ambiguous candidate.
//! Diagnostics without a code or provenance stay unmatched rather than being
//! paired by message text. A comparison never claims that an edit caused a
//! diagnostic.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::LazyLock;
use std::time::Instant;

use parking_lot::Mutex;
use serde_json::{Value, json};

use super::super::super::{BufferSnapshot, CodeDiagnostic, ServerAnalysis};
use crate::core::SessionCodeScope;

/// A larger observation is not captured; comparisons against it are unknown.
const MAX_CAPTURED: usize = 200;
const MAX_FILES_PER_SESSION: usize = 64;
const MAX_FILES: usize = 512;
/// Listed new candidates per file; counts stay complete.
const MAX_LISTED: usize = 10;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Identity {
    server: String,
    source_kind: i32,
    registration: Option<String>,
    source: Option<String>,
    code: String,
}

#[derive(Clone, Debug)]
struct Entry {
    identity: Option<Identity>,
    severity: i32,
}

#[derive(Clone, Debug)]
struct Capture {
    stream: (String, String, String),
    sha256: String,
    utf8_bytes: u32,
    servers: BTreeSet<u64>,
    /// `None` when the observation exceeded the capture bound.
    entries: Option<Vec<Entry>>,
}

struct Record {
    baseline: Option<Capture>,
    latest: Capture,
    touched: Instant,
}

static RECORDS: LazyLock<Mutex<HashMap<(SessionCodeScope, String), Record>>> =
    LazyLock::new(Mutex::default);

/// A pending capture, recorded only after the read passed its trust fence.
pub(super) struct Pending {
    capture: Capture,
}

/// Prepare a capture of one `document_current` observation. A missing or
/// unbounded snapshot cannot bind a capture to content.
pub(super) fn capture(
    diagnostics: &[CodeDiagnostic],
    snapshot: Option<&BufferSnapshot>,
    analysis: &[ServerAnalysis],
) -> Option<Pending> {
    let snapshot = snapshot.filter(|value| value.bounded())?;
    let (instance, buffer, epoch) = snapshot.stream();
    let (sha256, utf8_bytes) = snapshot.content();
    let entries = (diagnostics.len() <= MAX_CAPTURED).then(|| {
        diagnostics
            .iter()
            .map(|value| Entry {
                identity: identity(value, snapshot),
                severity: value.severity,
            })
            .collect()
    });
    Some(Pending {
        capture: Capture {
            stream: (instance.to_owned(), buffer.to_owned(), epoch.to_owned()),
            sha256: sha256.to_owned(),
            utf8_bytes,
            servers: analysis.iter().map(|server| server.server_id).collect(),
            entries,
        },
    })
}

fn identity(value: &CodeDiagnostic, snapshot: &BufferSnapshot) -> Option<Identity> {
    let provenance = value
        .provenance
        .as_ref()
        .filter(|provenance| provenance.belongs_to(snapshot))?;
    let code = value.code.as_ref().filter(|code| code.len() <= 256)?;
    let (server, source_kind, registration) = provenance.matching_identity();
    Some(Identity {
        server: server.to_owned(),
        source_kind,
        registration: registration.map(str::to_owned),
        source: value.source.clone(),
        code: code.clone(),
    })
}

/// Record a fenced capture for this exact Session scope and return the
/// comparison against the baseline, if a compatible one exists.
pub(super) fn record(
    scope: &SessionCodeScope,
    path: &str,
    pending: Pending,
    diagnostics: &[CodeDiagnostic],
) -> Value {
    let capture = pending.capture;
    let mut records = RECORDS.lock();
    let key = (scope.clone(), path.to_owned());
    let baseline = match records.get_mut(&key) {
        Some(record) => {
            if record.latest.sha256 != capture.sha256
                || record.latest.utf8_bytes != capture.utf8_bytes
            {
                record.baseline = Some(std::mem::replace(&mut record.latest, capture.clone()));
            } else {
                // Same text: later evidence (for example a server that
                // finished) replaces the capture; the baseline stays.
                record.latest = capture.clone();
            }
            record.touched = Instant::now();
            record.baseline.clone()
        }
        None => {
            evict(&mut records, scope);
            records.insert(
                key,
                Record {
                    baseline: None,
                    latest: capture.clone(),
                    touched: Instant::now(),
                },
            );
            None
        }
    };
    drop(records);
    match baseline {
        Some(baseline) => compare(&baseline, &capture, diagnostics),
        None => unknown("no_baseline"),
    }
}

/// Keep room for one more record within the Session and global bounds,
/// dropping the least recently observed. Retargeted scopes age out too.
fn evict(records: &mut HashMap<(SessionCodeScope, String), Record>, scope: &SessionCodeScope) {
    let session = records.keys().filter(|(owner, _)| owner == scope).count();
    let over = if session >= MAX_FILES_PER_SESSION {
        Some(true)
    } else if records.len() >= MAX_FILES {
        Some(false)
    } else {
        None
    };
    let Some(within_session) = over else {
        return;
    };
    let oldest = records
        .iter()
        .filter(|((owner, _), _)| !within_session || owner == scope)
        .min_by_key(|(_, record)| record.touched)
        .map(|(key, _)| key.clone());
    if let Some(oldest) = oldest {
        records.remove(&oldest);
    }
}

pub(super) fn unknown(reason: &str) -> Value {
    json!({"state":"unknown","reason":reason})
}

fn compare(baseline: &Capture, current: &Capture, diagnostics: &[CodeDiagnostic]) -> Value {
    if baseline.stream != current.stream {
        return unknown("buffer_replaced");
    }
    if baseline.servers != current.servers {
        return unknown("servers_changed");
    }
    let (Some(before), Some(after)) = (&baseline.entries, &current.entries) else {
        return unknown("over_capture_bound");
    };
    let group = |entries: &[Entry]| {
        let mut grouped: BTreeMap<Identity, Vec<i32>> = BTreeMap::new();
        let mut unmatched = 0usize;
        for entry in entries {
            match &entry.identity {
                Some(identity) => grouped
                    .entry(identity.clone())
                    .or_default()
                    .push(entry.severity),
                None => unmatched += 1,
            }
        }
        for severities in grouped.values_mut() {
            // LSP severity 1 is the most severe; pair most severe first.
            severities.sort_unstable_by_key(|value| rank(*value));
        }
        (grouped, unmatched)
    };
    let (before_groups, before_unmatched) = group(before);
    let (after_groups, after_unmatched) = group(after);
    let (mut introduced, mut resolved, mut persisting, mut escalated) = (0, 0, 0, 0);
    let mut grown = BTreeSet::new();
    let identities: BTreeSet<&Identity> = before_groups.keys().chain(after_groups.keys()).collect();
    for identity in identities {
        let old = before_groups.get(identity).map_or(&[][..], Vec::as_slice);
        let new = after_groups.get(identity).map_or(&[][..], Vec::as_slice);
        let paired = old.len().min(new.len());
        persisting += paired;
        escalated += old
            .iter()
            .zip(new)
            .filter(|(old, new)| rank(**new) < rank(**old))
            .count();
        if new.len() > old.len() {
            introduced += new.len() - old.len();
            grown.insert(identity.clone());
        } else {
            resolved += old.len() - new.len();
        }
    }
    // Candidates of a grown identity: without an anchor mapping, any of
    // them may be the new instance when the baseline already had one.
    let mut candidates: Vec<Value> = Vec::new();
    for (index, entry) in after.iter().enumerate() {
        let Some(identity) = entry
            .identity
            .as_ref()
            .filter(|value| grown.contains(value))
        else {
            continue;
        };
        if candidates.len() == MAX_LISTED {
            break;
        }
        let Some(value) = diagnostics.get(index) else {
            continue;
        };
        candidates.push(json!({
            "severity": super::diagnostics::severity(value.severity),
            "line": value.start.row + 1,
            "column": value.start.column + 1,
            "source": identity.source,
            "code": identity.code,
            "ambiguous": before_groups.contains_key(identity),
        }));
    }
    json!({
        "state": if before_unmatched == 0 && after_unmatched == 0 { "complete" } else { "partial" },
        "against": {"sha256": baseline.sha256, "utf8Bytes": baseline.utf8_bytes},
        "introduced": introduced,
        "resolved": resolved,
        "persisting": persisting,
        "escalated": escalated,
        "unmatched": {"baseline": before_unmatched, "current": after_unmatched},
        "candidates": candidates,
    })
}

/// Order LSP severities with unknown values last, never as most severe.
fn rank(severity: i32) -> i32 {
    if (1..=4).contains(&severity) {
        severity
    } else {
        i32::MAX
    }
}

#[cfg(test)]
mod tests;
