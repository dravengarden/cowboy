//! Zed 1.13 diagnostics arrive as buffer operations, not `LspQueryResponse`.
//! Until a full CRDT mirror exists, anchor conversion is supported only while
//! the original base text is unchanged. Never resolve anchors using disk text.

use std::collections::{BTreeMap, HashMap};

use anyhow::{Context as _, Result, ensure};
use serde::Serialize;

use crate::{LanguageDiagnostic, LanguagePoint, MAX_DIAGNOSTICS};

const MAX_TEXT: usize = 4 * 1024 * 1024;
const MAX_TOTAL_TEXT: usize = 32 * 1024 * 1024;
const MAX_DIAGNOSTIC_TEXT: usize = 1024 * 1024;
const MAX_TOTAL_DIAGNOSTIC_TEXT: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Status {
    #[default]
    Unobserved,
    Observed,
}

struct Buffer {
    text: Option<String>,
    revision: u64,
    servers: BTreeMap<u64, Server>,
    diagnostic_bytes: usize,
    overflow: bool,
}

struct Server {
    stamp: (u32, u32),
    diagnostics: Vec<LanguageDiagnostic>,
}

#[derive(Default)]
pub(crate) struct Cache {
    buffers: HashMap<u64, Buffer>,
    text_bytes: usize,
    diagnostic_bytes: usize,
    sequence: u64,
}

impl Cache {
    pub(crate) fn remove(&mut self, id: u64) {
        if let Some(buffer) = self.buffers.remove(&id) {
            self.text_bytes -= buffer.text.as_ref().map_or(0, String::len);
            self.diagnostic_bytes -= buffer.diagnostic_bytes;
        }
    }

    pub(crate) fn observe(&mut self, payload: &proto::envelope::Payload) {
        match payload {
            proto::envelope::Payload::CreateBufferForPeer(message) => match &message.variant {
                Some(proto::create_buffer_for_peer::Variant::State(state)) => {
                    self.remove(state.id);
                    if self.buffers.len() >= 1_024 {
                        return;
                    }
                    let Some(sequence) = self.sequence.checked_add(1) else {
                        return;
                    };
                    self.sequence = sequence;
                    let text = (state.base_text.len() <= MAX_TEXT
                        && self.text_bytes + state.base_text.len() <= MAX_TOTAL_TEXT)
                        .then(|| state.base_text.clone());
                    self.text_bytes += text.as_ref().map_or(0, String::len);
                    self.buffers.insert(
                        state.id,
                        Buffer {
                            text,
                            revision: sequence,
                            servers: BTreeMap::new(),
                            diagnostic_bytes: 0,
                            overflow: false,
                        },
                    );
                }
                Some(proto::create_buffer_for_peer::Variant::Chunk(chunk)) => {
                    self.operations(chunk.buffer_id, &chunk.operations);
                }
                None => {}
            },
            proto::envelope::Payload::UpdateBuffer(update) => {
                self.operations(update.buffer_id, &update.operations);
            }
            _ => {}
        }
    }

    fn operations(&mut self, id: u64, operations: &[proto::Operation]) {
        let Some(buffer) = self.buffers.get_mut(&id) else {
            return;
        };
        for operation in operations {
            match &operation.variant {
                Some(proto::operation::Variant::Edit(_) | proto::operation::Variant::Undo(_)) => {
                    self.text_bytes -= buffer.text.take().as_ref().map_or(0, String::len);
                    self.diagnostic_bytes -= buffer.diagnostic_bytes;
                    buffer.diagnostic_bytes = 0;
                    buffer.servers.clear();
                }
                Some(proto::operation::Variant::UpdateDiagnostics(update)) => {
                    if buffer.text.is_none() || buffer.overflow {
                        continue;
                    }
                    let stamp = (update.lamport_timestamp, update.replica_id);
                    if buffer
                        .servers
                        .get(&update.server_id)
                        .is_some_and(|old| old.stamp >= stamp)
                    {
                        continue;
                    }
                    let others = buffer
                        .servers
                        .iter()
                        .filter(|(server, _)| **server != update.server_id);
                    let (mut count, mut bytes) = (0, 0);
                    for diagnostic in others.flat_map(|(_, server)| &server.diagnostics) {
                        count += 1;
                        bytes += diagnostic.message.len()
                            + diagnostic.source.as_ref().map_or(0, String::len);
                    }
                    for diagnostic in &update.diagnostics {
                        count += 1;
                        bytes += diagnostic.message.len()
                            + diagnostic.source.as_ref().map_or(0, String::len);
                    }
                    self.diagnostic_bytes -= buffer.diagnostic_bytes;
                    buffer.diagnostic_bytes = 0;
                    if count > MAX_DIAGNOSTICS
                        || bytes > MAX_DIAGNOSTIC_TEXT
                        || self.diagnostic_bytes + bytes > MAX_TOTAL_DIAGNOSTIC_TEXT
                        || (!buffer.servers.contains_key(&update.server_id)
                            && buffer.servers.len() >= 32)
                    {
                        buffer.overflow = true;
                        buffer.servers.clear();
                    } else {
                        let text = buffer.text.as_deref().expect("snapshot was checked");
                        let diagnostics = convert(text, id, &update.diagnostics);
                        if let Ok(diagnostics) = diagnostics {
                            buffer
                                .servers
                                .insert(update.server_id, Server { stamp, diagnostics });
                            buffer.diagnostic_bytes = bytes;
                            self.diagnostic_bytes += bytes;
                        } else {
                            buffer.overflow = true;
                            buffer.servers.clear();
                        }
                    }
                }
                _ => {}
            }
        }
    }

    pub(crate) fn revision(&self, id: u64) -> Result<u64> {
        let buffer = self
            .buffers
            .get(&id)
            .context("native buffer snapshot unavailable")?;
        ensure!(
            buffer.text.is_some(),
            "native buffer content changed or exceeds snapshot limits"
        );
        ensure!(
            !buffer.overflow,
            "native diagnostic snapshot is invalid or exceeds limits"
        );
        Ok(buffer.revision)
    }

    pub(crate) fn read(&self, id: u64, revision: u64) -> Result<(Status, Vec<LanguageDiagnostic>)> {
        ensure!(
            self.revision(id)? == revision,
            "native buffer snapshot changed during read"
        );
        let buffer = &self.buffers[&id];
        let diagnostics = buffer
            .servers
            .values()
            .flat_map(|server| server.diagnostics.clone())
            .collect();
        Ok((
            if buffer.servers.is_empty() {
                Status::Unobserved
            } else {
                Status::Observed
            },
            diagnostics,
        ))
    }

    pub(crate) fn anchor_offset(&self, id: u64, anchor: &proto::Anchor) -> Result<u64> {
        self.revision(id)?;
        let text = self.buffers[&id]
            .text
            .as_deref()
            .expect("snapshot was checked");
        let offset = anchor_offset(anchor, id, text.len())?;
        ensure!(
            text.is_char_boundary(usize::try_from(offset)?),
            "invalid inlay anchor boundary"
        );
        Ok(offset)
    }
}

fn convert(
    text: &str,
    id: u64,
    diagnostics: &[proto::Diagnostic],
) -> Result<Vec<LanguageDiagnostic>> {
    let mut ranges = Vec::with_capacity(diagnostics.len());
    let mut offsets = Vec::with_capacity(diagnostics.len() * 2);
    for diagnostic in diagnostics {
        let start = anchor_offset(
            diagnostic
                .start
                .as_ref()
                .context("diagnostic start missing")?,
            id,
            text.len(),
        )?;
        let end = anchor_offset(
            diagnostic.end.as_ref().context("diagnostic end missing")?,
            id,
            text.len(),
        )?;
        ensure!(
            start <= end
                && text.is_char_boundary(usize::try_from(start)?)
                && text.is_char_boundary(usize::try_from(end)?),
            "invalid diagnostic range"
        );
        ranges.push((start, end));
        offsets.extend([start, end]);
    }
    offsets.sort_unstable();
    offsets.dedup();
    let mut wanted = offsets.into_iter().peekable();
    let mut points = BTreeMap::new();
    let mut point = LanguagePoint { row: 0, column: 0 };
    // One scan, not one full-text scan per diagnostic in the protocol reader.
    for (offset, character) in text
        .char_indices()
        .chain(std::iter::once((text.len(), '\0')))
    {
        if wanted.peek() == Some(&u64::try_from(offset)?) {
            points.insert(wanted.next().expect("offset was present"), point.clone());
        }
        if wanted.peek().is_none() {
            break;
        }
        if character == '\n' {
            point.row += 1;
            point.column = 0;
        } else {
            point.column += u32::try_from(character.len_utf16())?;
        }
    }
    Ok(diagnostics
        .iter()
        .zip(ranges)
        .map(|(diagnostic, (start, end))| LanguageDiagnostic {
            start: points[&start].clone(),
            end: points[&end].clone(),
            severity: diagnostic.severity,
            source: diagnostic.source.clone(),
            message: diagnostic.message.clone(),
        })
        .collect())
}

fn anchor_offset(anchor: &proto::Anchor, id: u64, length: usize) -> Result<u64> {
    ensure!(
        anchor.buffer_id.is_none_or(|buffer| buffer == id),
        "foreign buffer anchor"
    );
    if anchor.replica_id == 0 && anchor.timestamp == 0 && anchor.offset == 0 {
        return Ok(0);
    }
    if anchor.replica_id == u32::from(u16::MAX)
        && anchor.timestamp == u32::MAX
        && anchor.offset == u64::from(u32::MAX)
    {
        return Ok(u64::try_from(length)?);
    }
    ensure!(
        anchor.replica_id == 0 && anchor.timestamp == 1 && anchor.offset <= u64::try_from(length)?,
        "anchor requires an unsupported content revision"
    );
    Ok(anchor.offset)
}

#[cfg(test)]
mod tests;
