//! Bounded pages of one original owner's native text observation. The opaque
//! snapshot detects native revision/owner ABA; it is not a resource or a grant.
use super::*;

pub(crate) const MAX_PAGE_BYTES: u32 = 64 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum TextPage {
    Start {},
    Continue { offset: u32, snapshot: String },
}

impl TextPage {
    pub(super) fn validate(&self, content: &Content) -> Result<()> {
        content.validate()?;
        if let Self::Continue { offset, snapshot } = self {
            ensure!(
                *offset > 0 && *offset < content.utf8_bytes,
                "invalid text offset"
            );
            snapshot_id(snapshot)?;
        }
        Ok(())
    }

    fn offset(&self) -> u32 {
        match self {
            Self::Start {} => 0,
            Self::Continue { offset, .. } => *offset,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub(crate) enum TextOutput {
    Mismatch {},
    Stale {},
    Page {
        snapshot: String,
        offset: u32,
        text: String,
        #[serde(deserialize_with = "Option::deserialize")]
        next_offset: Option<u32>,
    },
}

impl TextOutput {
    pub(super) fn validate(&self, content: &Content, page: &TextPage) -> Result<()> {
        match self {
            Self::Mismatch {} => Ok(()),
            Self::Stale {} => {
                ensure!(
                    matches!(page, TextPage::Continue { .. }),
                    "initial text cannot be stale"
                );
                Ok(())
            }
            Self::Page {
                snapshot,
                offset,
                text,
                next_offset,
            } => {
                snapshot_id(snapshot)?;
                if let TextPage::Continue {
                    snapshot: expected, ..
                } = page
                {
                    ensure!(snapshot == expected, "text snapshot changed");
                }
                ensure!(*offset == page.offset(), "text offset changed");
                ensure!(
                    text.len() <= MAX_PAGE_BYTES as usize && !text.contains('\r'),
                    "invalid text page"
                );
                let end = offset
                    .checked_add(u32::try_from(text.len())?)
                    .context("text offset overflow")?;
                ensure!(end <= content.utf8_bytes, "text exceeds complete content");
                if end < content.utf8_bytes {
                    // A fixed-size page may clip at most three trailing UTF-8
                    // bytes. Refuse tiny/empty pages and unbounded continuations.
                    ensure!(
                        *next_offset == Some(end) && text.len() >= (MAX_PAGE_BYTES - 3) as usize,
                        "invalid text continuation"
                    );
                } else {
                    ensure!(next_offset.is_none(), "complete text has continuation");
                }
                if *offset == 0 && next_offset.is_none() {
                    use sha2::{Digest as _, Sha256};
                    ensure!(
                        format!("{:x}", Sha256::digest(text.as_bytes())) == content.sha256,
                        "complete text digest changed"
                    );
                }
                Ok(())
            }
        }
    }
}

fn snapshot_id(value: &str) -> Result<()> {
    ensure!(
        value.len() == 64
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "invalid text snapshot"
    );
    Ok(())
}
