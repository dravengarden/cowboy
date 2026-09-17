//! A closed read conditional on complete LF-normalized UTF-8 text equality.
//! No reload, save, path lookup, replacement owner or native writer authority.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

use crate::{LanguageDocumentSymbol, LanguageHoverBlock, LanguageObservation, ZedRuntime};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Content {
    pub(crate) sha256: String,
    pub(crate) utf8_bytes: u32,
}

impl Content {
    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            self.utf8_bytes <= 4 * 1024 * 1024
                && self.sha256.len() == 64
                && self
                    .sha256
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "invalid buffer content identity"
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Point {
    pub(crate) row: u32,
    pub(crate) column: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum Query {
    Language {},
    Symbols {},
    Hover { position: Point },
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum Output {
    Mismatch {},
    Observed { observation: Observation },
    Hover { contents: Vec<LanguageHoverBlock> },
}

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum Observation {
    Language(LanguageObservation),
    Symbols {
        symbols: Vec<LanguageDocumentSymbol>,
    },
}

impl ZedRuntime {
    pub(crate) async fn content_read(
        &self,
        id: u64,
        content: &Content,
        query: Query,
    ) -> Result<Output> {
        content.validate()?;
        let revision = self
            .diagnostics
            .lock()
            .expect("diagnostic cache poisoned")
            .match_content(id, content)?;
        let Some(revision) = revision else {
            return Ok(Output::Mismatch {});
        };
        let output = match query {
            Query::Language {} => Output::Observed {
                observation: Observation::Language(self.language(id).await?),
            },
            Query::Symbols {} => Output::Observed {
                observation: Observation::Symbols {
                    symbols: self.document_symbols(id).await?,
                },
            },
            Query::Hover { position } => Output::Hover {
                contents: self.hover(id, position.row, position.column).await?,
            },
        };
        // In addition to each query's own epoch check, cover the gap between
        // content equality and query admission, including edit/undo ABA.
        self.diagnostics
            .lock()
            .expect("diagnostic cache poisoned")
            .check(id, revision)?;
        Ok(output)
    }
}
