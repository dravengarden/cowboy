//! Text equality evidence, not a lease, version, filesystem identity or grant.
use super::*;

pub(crate) use crate::machine_protocol::code_buffer_sync::Content;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum Query {
    Language {},
    Symbols {},
    Hover { position: Point },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum ContentOutput {
    /// No LSP query was dispatched. No current native digest/text is disclosed.
    Mismatch {},
    /// The existing closed observation is nested, never another content query.
    Observed {
        observation: Observation,
    },
    Hover {
        contents: Vec<HoverBlock>,
    },
}

impl ContentOutput {
    pub(super) fn validate(&self, query: Query) -> Result<()> {
        match (query, self) {
            (_, Self::Mismatch {}) => Ok(()),
            (
                Query::Language {},
                Self::Observed {
                    observation: Observation::Language(value),
                },
            ) => value.validate(),
            (
                Query::Symbols {},
                Self::Observed {
                    observation: Observation::Symbols(value),
                },
            ) => value.validate(),
            (Query::Hover { .. }, Self::Hover { contents }) => {
                ensure!(contents.len() <= 32, "too many hover blocks");
                for block in contents {
                    text(&block.text)?;
                    optional_text(block.language.as_deref())?;
                }
                Ok(())
            }
            _ => anyhow::bail!("content read changed operation"),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum Observation {
    Language(LanguageOutput),
    Symbols(SymbolOutput),
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HoverBlock {
    text: String,
    language: Option<String>,
    markdown: bool,
}
