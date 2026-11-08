//! The native text actually observed, independently of the original Open reply.
use crate::{ADAPTER_VERSION, Buffers, Response, Zed, buffer_key, source_snapshot, sync_owners};
use anyhow::{Context as _, Result};
use serde::Serialize;
use std::path::PathBuf;

use crate::{BufferVersionEntry, LanguageDiagnostic, LanguageInlayHint, analysis, diagnostics};

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LanguageObservation {
    pub(crate) diagnostics_state: diagnostics::Status,
    #[serde(serialize_with = "diagnostics::serialize_legacy")]
    pub(crate) diagnostics: Vec<LanguageDiagnostic>,
    pub(crate) inlay_hints: Vec<LanguageInlayHint>,
    pub(crate) semantic_tokens: Vec<u32>,
    // Owned protocol one is closed. Only the legacy language response exposes
    // these additive fields; no owned consumer silently changes its contract.
    #[serde(skip)]
    pub(crate) analysis: Option<Vec<analysis::ServerAnalysis>>,
    #[serde(skip)]
    pub(crate) version: Option<Vec<BufferVersionEntry>>,
    #[serde(skip)]
    pub(crate) snapshot: Option<Box<BufferSnapshot>>,
    #[serde(skip)]
    pub(crate) revision: Option<u64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BufferSnapshot {
    pub(crate) instance: String,
    pub(crate) buffer: String,
    pub(crate) buffer_epoch: String,
    pub(crate) version: Vec<BufferVersionEntry>,
    pub(crate) content: crate::content_reads::Content,
}

pub(crate) async fn respond(
    request: crate::Request,
    worktrees: &crate::Worktrees,
    buffers: &Buffers,
    zed: Option<&Zed>,
) -> Result<Response> {
    let crate::Request::BufferLanguage {
        worktree,
        path,
        refresh,
    } = request
    else {
        anyhow::bail!("not a language observation request");
    };
    crate::trust::ensure_language(&worktree, worktrees).await?;
    buffer_language(worktree, path, refresh, buffers, zed).await
}

pub(crate) async fn buffer_language(
    worktree: PathBuf,
    path: PathBuf,
    refresh: Option<crate::agent_refresh::Refresh>,
    buffers: &Buffers,
    zed: Option<&Zed>,
) -> Result<Response> {
    let source_path = path.clone();
    let (worktree, path) = buffer_key(worktree, path).await?;
    let refresh = match (refresh, zed) {
        (Some(crate::agent_refresh::Refresh::AgentOwned), Some(zed)) => Some(
            crate::agent_refresh::refresh(
                &worktree,
                &source_path,
                &(worktree.clone(), path.clone()),
                buffers,
                zed,
            )
            .await,
        ),
        (Some(_), None) => Some(crate::agent_refresh::Outcome::Refused),
        (None, _) => None,
    };
    let all = buffers.active.read().await;
    let lease = all
        .get(&(worktree.clone(), path.clone()))
        .context("buffer is not open")?;
    sync_owners::ensure_readable(lease)?;
    let observation = if let Some(zed) = zed {
        zed.language(lease.remote_id).await?
    } else {
        LanguageObservation::default()
    };
    let buffer_id = lease.remote_id;
    let version = observation.version.unwrap_or_else(|| lease.version.clone());
    drop(all);
    let source_state = source_snapshot::check(
        &worktree,
        &source_path,
        observation
            .snapshot
            .as_ref()
            .map(|snapshot| &snapshot.content),
    )
    .await;
    if let (Some(zed), Some(revision)) = (zed, observation.revision) {
        zed.diagnostics
            .lock()
            .expect("diagnostic cache poisoned")
            .check(buffer_id, revision)?;
    }
    Ok(Response::BufferLanguage {
        api_version: ADAPTER_VERSION,
        worktree,
        path,
        version,
        buffer_snapshot: observation.snapshot,
        source_state,
        diagnostics: observation.diagnostics,
        inlay_hints: observation.inlay_hints,
        semantic_tokens: observation.semantic_tokens,
        analysis: observation.analysis,
        refresh,
    })
}
