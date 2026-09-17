//! Closed observation protocol shared by Controller and Machine, not a grant.
//! The version is the original open's lower bound, NOT a content snapshot or
//! authority to interpret cursor positions. Content-bound reads independently
//! compare the complete displayed text with the retained native buffer.

use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_REPLY_BYTES: usize = 2 * 1024 * 1024;

mod content;
pub(crate) use content::{Content, ContentOutput, Query};
mod text;
pub(crate) use text::{TextOutput, TextPage};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum Request {
    Language {},
    Symbols {},
    Content { content: Content, query: Query },
    Text { content: Content, page: TextPage },
}

impl Request {
    pub(crate) fn validate(&self) -> Result<()> {
        if let Self::Content { content, .. } = self {
            content.validate()?;
        }
        if let Self::Text { content, page } = self {
            page.validate(content)?;
        }
        Ok(())
    }

    #[cfg(feature = "full")]
    pub(crate) fn support_kind(&self) -> &'static str {
        match self {
            Self::Content { .. } => "bufferLeaseContentSupport",
            Self::Text { .. } => "bufferLeaseTextSupport",
            Self::Language {} | Self::Symbols {} => "bufferLeaseReadSupport",
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reply<L> {
    #[serde(rename = "type")]
    kind: ReplyKind,
    api_version: u8,
    pub lease: L,
    pub opened_version: Vec<VersionEntry>,
    pub result: Output,
}

#[derive(Debug, Deserialize, Serialize)]
enum ReplyKind {
    #[serde(rename = "bufferLeaseRead")]
    BufferLeaseRead,
}

impl<L: serde::de::DeserializeOwned + PartialEq> Reply<L> {
    pub(crate) fn parse(value: &Value, lease: &L, request: Request) -> Result<Self> {
        request.validate()?;
        // Re-decode with serde_json's depth bound as well as our byte limit.
        // A Value supplied by an in-process caller has not necessarily crossed
        // a bounded socket decoder.
        let bytes = serde_json::to_vec(value)?;
        ensure!(
            bytes.len() <= MAX_REPLY_BYTES,
            "buffer read reply too large"
        );
        let reply: Self = serde_json::from_slice(&bytes)?;
        ensure!(reply.api_version == 1, "unsupported buffer read reply");
        ensure!(&reply.lease == lease, "buffer read changed its owner");
        ensure!(
            reply.opened_version.len() <= 256
                && reply
                    .opened_version
                    .windows(2)
                    .all(|pair| pair[0].replica_id < pair[1].replica_id),
            "invalid buffer version vector"
        );
        reply.result.validate(request)?;
        Ok(reply)
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct VersionEntry {
    replica_id: u32,
    timestamp: u32,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum Output {
    Language(LanguageOutput),
    Symbols(SymbolOutput),
    Content {
        content: Content,
        result: ContentOutput,
    },
    Text {
        content: Content,
        result: TextOutput,
    },
}

impl Output {
    fn validate(&self, request: Request) -> Result<()> {
        match (request, self) {
            (Request::Language {}, Self::Language(value)) => value.validate()?,
            (Request::Symbols {}, Self::Symbols(value)) => value.validate()?,
            (
                Request::Content { content, query },
                Self::Content {
                    content: observed,
                    result,
                },
            ) => {
                ensure!(
                    &content == observed,
                    "buffer read changed its content identity"
                );
                result.validate(query)?;
            }
            (
                Request::Text { content, page },
                Self::Text {
                    content: observed,
                    result,
                },
            ) => {
                ensure!(
                    &content == observed,
                    "buffer text changed its content identity"
                );
                result.validate(&content, &page)?;
            }
            _ => anyhow::bail!("buffer read reply changed operation"),
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct LanguageOutput {
    diagnostics_state: DiagnosticStatus,
    diagnostics: Vec<Diagnostic>,
    inlay_hints: Vec<InlayHint>,
    semantic_tokens: Vec<u32>,
}

impl LanguageOutput {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.diagnostics_state == DiagnosticStatus::Observed || self.diagnostics.is_empty(),
            "unobserved diagnostics must be empty"
        );
        ensure!(
            self.diagnostics.len() <= 1_000
                && self.inlay_hints.len() <= 2_000
                && self.semantic_tokens.len() <= 50_000
                && self.semantic_tokens.len().is_multiple_of(5),
            "language result exceeds limits"
        );
        for diagnostic in &self.diagnostics {
            ensure!(
                diagnostic.start <= diagnostic.end,
                "invalid diagnostic range"
            );
            text(&diagnostic.message)?;
            optional_text(diagnostic.source.as_deref())?;
        }
        for hint in &self.inlay_hints {
            ensure!(hint.offset <= u64::from(u32::MAX), "invalid inlay offset");
            text(&hint.label)?;
            optional_text(hint.kind.as_deref())?;
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SymbolOutput {
    symbols: Vec<Symbol>,
}

impl SymbolOutput {
    fn validate(&self) -> Result<()> {
        let mut pending: Vec<_> = self.symbols.iter().map(|symbol| (symbol, 1)).collect();
        ensure!(pending.len() <= 2_000, "too many symbols");
        let mut count = 0;
        while let Some((symbol, depth)) = pending.pop() {
            count += 1;
            ensure!(count <= 2_000 && depth <= 16, "symbol tree exceeds limits");
            text(&symbol.name)?;
            ensure!(
                symbol.start <= symbol.selection_start
                    && symbol.selection_start <= symbol.selection_end
                    && symbol.selection_end <= symbol.end,
                "invalid symbol range"
            );
            ensure!(
                pending.len() + symbol.children.len() <= 2_000,
                "too many symbols"
            );
            pending.extend(symbol.children.iter().map(|child| (child, depth + 1)));
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DiagnosticStatus {
    Unobserved,
    Observed,
}

fn text(value: &str) -> Result<()> {
    ensure!(value.len() <= 64 * 1024, "language text exceeds limits");
    Ok(())
}

fn optional_text(value: Option<&str>) -> Result<()> {
    value.map_or(Ok(()), text)
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Point {
    row: u32,
    column: u32,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Diagnostic {
    start: Point,
    end: Point,
    severity: i32,
    source: Option<String>,
    message: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct InlayHint {
    offset: u64,
    label: String,
    kind: Option<String>,
    padding_left: bool,
    padding_right: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Symbol {
    name: String,
    kind: i32,
    start: Point,
    end: Point,
    selection_start: Point,
    selection_end: Point,
    children: Vec<Self>,
}

#[cfg(test)]
mod tests;
