//! Passive, bounded native text pages. No native request, path lookup, reload,
//! registration or new lifetime. The original buffer owner remains required.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::fmt::Write as _;

use crate::{buffer_leases::LeaseRef, content_reads::Content, diagnostics::Cache};

pub(crate) const MAX_PAGE_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum Page {
    Start {},
    Continue { offset: u32, snapshot: String },
}

impl Page {
    pub(crate) fn validate(&self, content: &Content) -> Result<()> {
        content.validate()?;
        if let Self::Continue { offset, snapshot } = self {
            ensure!(
                *offset > 0 && *offset < content.utf8_bytes,
                "invalid text offset"
            );
            ensure!(
                snapshot.len() == 64
                    && snapshot
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "invalid text snapshot"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(crate) enum Output {
    Mismatch {},
    Stale {},
    Page {
        snapshot: String,
        offset: u32,
        text: String,
        next_offset: Option<u32>,
    },
}

impl Cache {
    /// Caller retains both the original owner and this mirror's mutex. Content
    /// comparison, epoch binding and page copy have no asynchronous gap.
    pub(crate) fn text_read(
        &self,
        id: u64,
        lease: &LeaseRef,
        content: &Content,
        page: &Page,
    ) -> Result<Output> {
        page.validate(content)?;
        let Some(revision) = self.match_content(id, content)? else {
            return Ok(Output::Mismatch {});
        };
        // Never disclose native IDs/vectors. Bind the observation to one exact
        // adapter instance/ordinary owner as well as its monotonic mirror epoch.
        let digest = Sha256::digest(serde_json::to_vec(&(
            "cowboy.buffer-text/v1",
            lease,
            id,
            revision,
            content,
        ))?);
        let mut snapshot = String::with_capacity(64);
        for byte in digest {
            write!(snapshot, "{byte:02x}").expect("write to string");
        }
        let offset = match page {
            Page::Start {} => 0,
            Page::Continue {
                offset,
                snapshot: expected,
            } => {
                if expected != &snapshot {
                    return Ok(Output::Stale {});
                }
                *offset
            }
        };
        let text = self.text_page(id, offset)?;
        let end = offset + u32::try_from(text.len())?;
        Ok(Output::Page {
            snapshot,
            offset,
            text,
            next_offset: (end < content.utf8_bytes).then_some(end),
        })
    }
}

#[cfg(test)]
mod tests;
