//! Bounded original-native-buffer observations, including edited text anchors.
//! Diagnostics remain last-observed state, not an atomic multi-LSP snapshot.
use std::collections::{BTreeMap, HashMap};

use anyhow::{Context as _, Result, ensure};
use serde::Serialize;

use crate::{
    BufferVersionEntry, LanguageDiagnostic, LanguagePoint, MAX_DIAGNOSTICS, coordinates::Mirror,
};

const MAX_TEXT: usize = crate::coordinates::MAX_HISTORY;
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
    text: Option<Mirror>,
    shared: bool,
    revision: u64,
    servers: BTreeMap<u64, Server>,
    diagnostic_bytes: usize,
    overflow: bool,
}

struct Server {
    stamp: (u32, u32),
    diagnostics: Vec<Diagnostic>,
}

struct Diagnostic {
    start: proto::Anchor,
    end: proto::Anchor,
    severity: i32,
    source: Option<String>,
    message: String,
}

/// Private query continuation, never serialized or reconstructed from a path.
pub(crate) struct Position {
    pub(crate) revision: u64,
    pub(crate) anchor: proto::Anchor,
    pub(crate) version: Vec<BufferVersionEntry>,
}

#[derive(Default)]
pub(crate) struct Cache {
    buffers: HashMap<u64, Buffer>,
    // Retained base + encoded text history, not just currently visible bytes.
    text_bytes: usize,
    diagnostic_bytes: usize,
    sequence: u64,
}

impl Cache {
    pub(crate) fn remove(&mut self, id: u64) {
        if let Some(buffer) = self.buffers.remove(&id) {
            self.text_bytes -= buffer.text.as_ref().map_or(0, Mirror::bytes);
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
                        .then(|| {
                            Mirror::new(state.id, &state.base_text)
                                .and_then(|mut text| {
                                    text.require(&state.saved_version)?;
                                    Ok(text)
                                })
                                .ok()
                        })
                        .flatten();
                    self.text_bytes += text.as_ref().map_or(0, Mirror::bytes);
                    self.buffers.insert(
                        state.id,
                        Buffer {
                            text,
                            shared: false,
                            revision: sequence,
                            servers: BTreeMap::new(),
                            diagnostic_bytes: 0,
                            overflow: false,
                        },
                    );
                }
                Some(proto::create_buffer_for_peer::Variant::Chunk(chunk)) => {
                    self.operations(chunk.buffer_id, &chunk.operations);
                    if chunk.is_last
                        && let Some(buffer) = self.buffers.get_mut(&chunk.buffer_id)
                    {
                        buffer.shared = true;
                    }
                }
                None => {}
            },
            proto::envelope::Payload::UpdateBuffer(update) => {
                self.operations(update.buffer_id, &update.operations);
            }
            proto::envelope::Payload::BufferReloaded(update) => {
                if let Some(buffer) = self.buffers.get_mut(&update.buffer_id) {
                    // The announcement may precede its edits. Keep the same
                    // history, but refuse reads until that exact floor arrives.
                    if let Some(sequence) = self.sequence.checked_add(1) {
                        self.sequence = sequence;
                        buffer.revision = sequence;
                        if let Some(text) = &mut buffer.text
                            && text.require(&update.version).is_err()
                        {
                            self.text_bytes -= buffer.text.take().map_or(0, |text| text.bytes());
                        }
                    } else {
                        self.text_bytes -= buffer.text.take().map_or(0, |text| text.bytes());
                    }
                }
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
                    let Some(mut text) = buffer.text.take() else {
                        continue;
                    };
                    let previous = text.bytes();
                    let applied = self
                        .sequence
                        .checked_add(1)
                        .zip(text.apply(operation, MAX_TOTAL_TEXT - self.text_bytes).ok());
                    self.text_bytes -= previous;
                    if let Some((sequence, changed)) = applied {
                        if changed {
                            self.sequence = sequence;
                            buffer.revision = sequence;
                        }
                        self.text_bytes += text.bytes();
                        buffer.text = Some(text);
                    } else {
                        self.diagnostic_bytes -= buffer.diagnostic_bytes;
                        buffer.diagnostic_bytes = 0;
                        buffer.servers.clear();
                    }
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
                    let captured = if count > MAX_DIAGNOSTICS
                        || bytes > MAX_DIAGNOSTIC_TEXT
                        || self.diagnostic_bytes + bytes > MAX_TOTAL_DIAGNOSTIC_TEXT
                        || (!buffer.servers.contains_key(&update.server_id)
                            && buffer.servers.len() >= 32)
                    {
                        None
                    } else {
                        capture(&update.diagnostics).ok()
                    };
                    if let Some(diagnostics) = captured {
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
                _ => {}
            }
        }
    }

    fn text(&self, id: u64) -> Result<&Mirror> {
        let buffer = self
            .buffers
            .get(&id)
            .context("native buffer snapshot unavailable")?;
        let text = buffer
            .text
            .as_ref()
            .context("native buffer history is invalid or exceeds limits")?;
        ensure!(
            buffer.shared && text.ready(),
            "native buffer history is incomplete"
        );
        ensure!(
            !buffer.overflow,
            "native diagnostic snapshot is invalid or exceeds limits"
        );
        Ok(text)
    }

    pub(crate) fn revision(&self, id: u64) -> Result<u64> {
        self.text(id)?;
        Ok(self.buffers[&id].revision)
    }

    pub(crate) fn version(&self, id: u64) -> Result<Vec<BufferVersionEntry>> {
        Ok(self.text(id)?.version())
    }

    pub(crate) fn check(&self, id: u64, revision: u64) -> Result<()> {
        ensure!(
            self.revision(id)? == revision,
            "native buffer snapshot changed during read"
        );
        Ok(())
    }

    pub(crate) fn position(&self, id: u64, row: u32, column: u32) -> Result<Position> {
        let text = self.text(id)?;
        Ok(Position {
            revision: self.revision(id)?,
            anchor: text.position(row, column)?,
            version: text.version(),
        })
    }

    pub(crate) fn read(&self, id: u64, revision: u64) -> Result<(Status, Vec<LanguageDiagnostic>)> {
        self.check(id, revision)?;
        let text = self.text(id)?;
        let buffer = &self.buffers[&id];
        let diagnostics = buffer
            .servers
            .values()
            .flat_map(|server| &server.diagnostics)
            .map(|diagnostic| convert(text, diagnostic))
            .collect::<Result<_>>()?;
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
        self.text(id)?.offset(anchor)
    }

    pub(crate) fn range(
        &self,
        id: u64,
        start: &proto::Anchor,
        end: &proto::Anchor,
    ) -> Result<(LanguagePoint, LanguagePoint)> {
        let text = self.text(id)?;
        ensure!(
            text.offset(start)? <= text.offset(end)?,
            "reversed native anchor range"
        );
        Ok((text.point(start)?, text.point(end)?))
    }
}

fn capture(values: &[proto::Diagnostic]) -> Result<Vec<Diagnostic>> {
    values
        .iter()
        .map(|value| {
            Ok(Diagnostic {
                start: value.start.clone().context("diagnostic start missing")?,
                end: value.end.clone().context("diagnostic end missing")?,
                severity: value.severity,
                source: value.source.clone(),
                message: value.message.clone(),
            })
        })
        .collect()
}

fn convert(text: &Mirror, diagnostic: &Diagnostic) -> Result<LanguageDiagnostic> {
    ensure!(
        text.offset(&diagnostic.start)? <= text.offset(&diagnostic.end)?,
        "reversed diagnostic range"
    );
    Ok(LanguageDiagnostic {
        start: text.point(&diagnostic.start)?,
        end: text.point(&diagnostic.end)?,
        severity: diagnostic.severity,
        source: diagnostic.source.clone(),
        message: diagnostic.message.clone(),
    })
}

#[cfg(test)]
mod tests;
