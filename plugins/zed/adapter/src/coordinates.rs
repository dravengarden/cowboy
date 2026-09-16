//! A passive mirror of the exact pinned server's text CRDT. No disk reads,
//! edits, reopen, clock renewal or cross-buffer coordinate fallback.
use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{Context as _, Result, ensure};
use proto::Message as _;
use sha2::{Digest as _, Sha256};

use crate::{BufferVersionEntry, LanguagePoint};

pub(crate) const MAX_HISTORY: usize = 4 * 1024 * 1024;
const MAX_OPERATIONS: usize = 4_096;
const MAX_PARTS: usize = 1_024;
const MAX_REPLICA: u32 = 1_023;
type Stamp = (u32, u32);

pub(crate) struct Mirror {
    buffer: text::Buffer,
    has_base: bool,
    // Same upstream engine, insertion-only projection. Its historical rope
    // exposes native FullOffsets (including tombstones) for UTF-8 validation;
    // no independently implemented fragment ordering or CRDT is involved.
    full: text::Buffer,
    insertions: BTreeMap<Stamp, String>,
    operations: BTreeMap<Stamp, [u8; 32]>,
    required: clock::Global,
    bytes: usize,
}

impl Mirror {
    pub(crate) fn new(id: u64, base: &str) -> Result<Self> {
        ensure!(
            base.len() <= MAX_HISTORY && !base.contains('\r'),
            "invalid native base text"
        );
        let buffer = text::Buffer::new(text::ReplicaId::LOCAL, text::BufferId::new(id)?, base);
        let insertions = if base.is_empty() {
            BTreeMap::new()
        } else {
            BTreeMap::from([((0, 1), base.to_owned())])
        };
        let full = text::Buffer::new(text::ReplicaId::LOCAL, text::BufferId::new(id)?, base);
        Ok(Self {
            buffer,
            has_base: !base.is_empty(),
            full,
            insertions,
            operations: BTreeMap::new(),
            required: clock::Global::new(),
            bytes: base.len(),
        })
    }

    pub(crate) fn bytes(&self) -> usize {
        self.bytes
    }

    /// Native history is a causally ordered stream. Missing predecessors are
    /// refused instead of retaining an unbounded deferred-operation queue.
    pub(crate) fn apply(&mut self, operation: &proto::Operation, available: usize) -> Result<bool> {
        let (replica, value, entries) = match operation.variant.as_ref() {
            Some(proto::operation::Variant::Edit(edit)) => {
                (edit.replica_id, edit.lamport_timestamp, &edit.version)
            }
            Some(proto::operation::Variant::Undo(undo)) => {
                (undo.replica_id, undo.lamport_timestamp, &undo.version)
            }
            _ => anyhow::bail!("not a text operation"),
        };
        let timestamp = timestamp(replica, value)?;
        let key = (replica, value);
        let digest: [u8; 32] = Sha256::digest(operation.encode_to_vec()).into();
        if let Some(previous) = self.operations.get(&key) {
            ensure!(*previous == digest, "native operation identity changed");
            return Ok(false);
        }
        // Upstream clocks use dense replica storage, even for sparse wire
        // versions. Charge that allocation as well as encoded history.
        let clock_cost = entries
            .iter()
            .map(|entry| entry.replica_id as usize + 1)
            .max()
            .unwrap_or(0)
            * 4;
        let cost = operation.encoded_len().saturating_add(clock_cost);
        ensure!(
            self.operations.len() < MAX_OPERATIONS
                && cost <= available
                && cost <= MAX_HISTORY.saturating_sub(self.bytes),
            "native text history limit"
        );
        let version = decode_version(entries)?;
        self.validate_history(timestamp, &version)?;
        let native = self.decode_operation(operation, timestamp, version)?;
        let mut full = native.clone();
        match &mut full {
            text::Operation::Edit(edit) => {
                let inserted = edit.new_text.concat();
                if !inserted.is_empty() {
                    self.insertions.insert(key, inserted);
                }
                for range in &mut edit.ranges {
                    range.end = range.start;
                }
            }
            text::Operation::Undo(undo) => undo.counts.clear(),
        }
        self.buffer.apply_ops([native]);
        self.full.apply_ops([full]);
        ensure!(
            !self.buffer.has_deferred_ops() && !self.full.has_deferred_ops(),
            "native text history is incomplete"
        );
        self.operations.insert(key, digest);
        self.bytes += cost;
        Ok(true)
    }

    fn validate_history(&self, timestamp: clock::Lamport, version: &clock::Global) -> Result<()> {
        ensure!(
            self.buffer.version().observed_all(version)
                && !self.buffer.version().observed(timestamp)
                && !version.observed(timestamp),
            "native operation has unavailable or conflicting history"
        );
        if let Some(previous) = self
            .buffer
            .version()
            .iter()
            .find(|entry| entry.replica_id == timestamp.replica_id)
        {
            ensure!(
                version.observed(previous),
                "native author omitted its own history"
            );
        }
        for entry in version.iter() {
            let base =
                entry.replica_id == text::ReplicaId::LOCAL && entry.value == 1 && self.has_base;
            ensure!(
                base || self
                    .operations
                    .contains_key(&(u32::from(entry.replica_id.as_u16()), entry.value)),
                "native version names an unknown operation"
            );
            ensure!(
                timestamp.value > entry.value,
                "invalid native Lamport order"
            );
        }
        if self.has_base {
            ensure!(
                version.observed(clock::Lamport {
                    replica_id: text::ReplicaId::LOCAL,
                    value: 1
                }),
                "native base is unobserved"
            );
        }
        for (_, previous) in self.buffer.operations().iter() {
            if version.observed(previous.timestamp()) {
                let predecessor = match previous {
                    text::Operation::Edit(edit) => &edit.version,
                    text::Operation::Undo(undo) => &undo.version,
                };
                ensure!(
                    version.observed_all(predecessor),
                    "native version omits causal history"
                );
            }
        }
        Ok(())
    }

    fn decode_operation(
        &self,
        operation: &proto::Operation,
        timestamp: clock::Lamport,
        version: clock::Global,
    ) -> Result<text::Operation> {
        Ok(match operation.variant.as_ref() {
            Some(proto::operation::Variant::Edit(edit)) => {
                ensure!(
                    edit.ranges.len() <= MAX_PARTS && edit.ranges.len() == edit.new_text.len(),
                    "invalid native edit shape"
                );
                let full = self.full.rope_for_version(&version);
                let mut previous_end = 0;
                for range in &edit.ranges {
                    ensure!(
                        range.start >= previous_end
                            && range.start <= range.end
                            && range.end <= u64::try_from(full.len())?,
                        "invalid native edit range"
                    );
                    for offset in [range.start, range.end] {
                        let offset = usize::try_from(offset)?;
                        ensure!(
                            full.clip_offset(offset, text::Bias::Left) == offset,
                            "native edit splits UTF-8"
                        );
                    }
                    previous_end = range.end;
                }
                ensure!(
                    edit.new_text.iter().all(|text| !text.contains('\r')),
                    "unnormalized native edit"
                );
                text::Operation::Edit(text::EditOperation {
                    timestamp,
                    version,
                    ranges: edit
                        .ranges
                        .iter()
                        .map(|range| {
                            Ok(text::FullOffset(usize::try_from(range.start)?)
                                ..text::FullOffset(usize::try_from(range.end)?))
                        })
                        .collect::<Result<_>>()?,
                    new_text: edit
                        .new_text
                        .iter()
                        .map(|value| Arc::<str>::from(value.as_str()))
                        .collect(),
                })
            }
            Some(proto::operation::Variant::Undo(undo)) => {
                ensure!(undo.counts.len() <= MAX_PARTS, "invalid native undo shape");
                let mut counts = BTreeMap::new();
                for count in &undo.counts {
                    let target = timestamp_for_count(count)?;
                    ensure!(
                        version.observed(target)
                            && self
                                .buffer
                                .operations()
                                .get(&target)
                                .is_some_and(text::Operation::is_edit)
                            && counts.insert(target, count.count).is_none(),
                        "invalid native undo target"
                    );
                }
                text::Operation::Undo(text::UndoOperation {
                    timestamp,
                    version,
                    counts: counts.into_iter().collect(),
                })
            }
            _ => unreachable!("variant checked"),
        })
    }

    pub(crate) fn require(&mut self, entries: &[proto::VectorClockEntry]) -> Result<()> {
        for entry in decode_version(entries)?.iter() {
            self.required.observe(entry);
        }
        Ok(())
    }

    pub(crate) fn ready(&self) -> bool {
        self.buffer.version().observed_all(&self.required)
    }

    pub(crate) fn version(&self) -> Vec<BufferVersionEntry> {
        self.buffer
            .version()
            .iter()
            .map(|entry| BufferVersionEntry {
                replica_id: u32::from(entry.replica_id.as_u16()),
                timestamp: entry.value,
            })
            .collect()
    }

    pub(crate) fn position(&self, row: u32, column: u32) -> Result<proto::Anchor> {
        let point = text::PointUtf16::new(row, column);
        ensure!(
            self.buffer
                .clip_point_utf16(text::Unclipped(point), text::Bias::Left)
                == point,
            "position is outside native content or splits UTF-16"
        );
        let offset = self.buffer.point_utf16_to_offset(point);
        let anchor = self.buffer.anchor_before(offset);
        Ok(proto::Anchor {
            replica_id: u32::from(anchor.timestamp().replica_id.as_u16()),
            timestamp: anchor.timestamp().value,
            offset: u64::from(anchor.offset),
            bias: match anchor.bias {
                text::Bias::Left => proto::Bias::Left,
                text::Bias::Right => proto::Bias::Right,
            } as i32,
            buffer_id: Some(u64::from(anchor.buffer_id)),
        })
    }

    pub(crate) fn offset(&self, wire: &proto::Anchor) -> Result<u64> {
        let id = self.buffer.remote_id();
        ensure!(
            wire.buffer_id.is_none_or(|value| value == u64::from(id)),
            "foreign native anchor"
        );
        let bias = match proto::Bias::from_i32(wire.bias).context("invalid native anchor bias")? {
            proto::Bias::Left => text::Bias::Left,
            proto::Bias::Right => text::Bias::Right,
        };
        let anchor = if (wire.replica_id, wire.timestamp, wire.offset) == (0, 0, 0) {
            text::Anchor::min_for_buffer(id)
        } else if (wire.replica_id, wire.timestamp, wire.offset)
            == (u32::from(u16::MAX), u32::MAX, u64::from(u32::MAX))
        {
            text::Anchor::max_for_buffer(id)
        } else {
            let stamp = timestamp(wire.replica_id, wire.timestamp)?;
            let text = self
                .insertions
                .get(&(wire.replica_id, wire.timestamp))
                .context("native anchor insertion is unavailable")?;
            let offset = usize::try_from(wire.offset)?;
            ensure!(
                offset <= text.len() && text.is_char_boundary(offset),
                "invalid native anchor offset"
            );
            text::Anchor::new(stamp, u32::try_from(offset)?, bias, id)
        };
        ensure!(
            self.buffer.can_resolve(&anchor),
            "native anchor is not observed"
        );
        let offset = self.buffer.offset_for_anchor(&anchor);
        ensure!(
            offset <= self.buffer.len()
                && self.buffer.clip_offset(offset, text::Bias::Left) == offset,
            "native anchor resolved outside content"
        );
        Ok(u64::try_from(offset)?)
    }

    pub(crate) fn point(&self, anchor: &proto::Anchor) -> Result<LanguagePoint> {
        let point = self
            .buffer
            .offset_to_point_utf16(usize::try_from(self.offset(anchor)?)?);
        Ok(LanguagePoint {
            row: point.row,
            column: point.column,
        })
    }
}

fn timestamp(replica: u32, value: u32) -> Result<clock::Lamport> {
    // Bound dense upstream vector storage and leave room for the passive
    // mirror's Lamport observation increments throughout its bounded history.
    ensure!(
        replica <= MAX_REPLICA && value > 0 && value < u32::MAX - 8_192,
        "invalid native timestamp"
    );
    Ok(clock::Lamport {
        replica_id: text::ReplicaId::new(u16::try_from(replica)?),
        value,
    })
}

fn timestamp_for_count(count: &proto::UndoCount) -> Result<clock::Lamport> {
    ensure!(count.count > 0, "invalid native undo count");
    timestamp(count.replica_id, count.lamport_timestamp)
}

fn decode_version(entries: &[proto::VectorClockEntry]) -> Result<clock::Global> {
    ensure!(entries.len() <= 256, "native version limit");
    let mut version = clock::Global::new();
    let mut replicas = std::collections::BTreeSet::new();
    for entry in entries {
        ensure!(
            replicas.insert(entry.replica_id),
            "duplicate native version entry"
        );
        version.observe(timestamp(entry.replica_id, entry.timestamp)?);
    }
    Ok(version)
}

#[cfg(test)]
pub(crate) mod tests;
