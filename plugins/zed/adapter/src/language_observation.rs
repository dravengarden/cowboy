//! The native text actually observed, independently of the original Open reply.
use serde::Serialize;

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
