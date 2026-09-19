// SPDX-License-Identifier: GPL-3.0-or-later
//! Closed local-server edit/undo ingress. No deferred queue or partial batch.
use super::*;
use anyhow::{Result, bail, ensure};
use rpc::proto;

pub const MAX_BATCH: usize = 128;

fn stamp(replica: u32, value: u32) -> Result<clock::Lamport> {
    ensure!(
        replica < MAX_VECTOR as u32 && value > 0 && value < u32::MAX - MAX_OPERATIONS as u32 - 2,
        "invalid private native operation clock"
    );
    Ok(clock::Lamport {
        replica_id: text::ReplicaId::new(replica as u16),
        value,
    })
}

/// Validate before the upstream decoder narrows replica IDs or allocates dense
/// clocks. This bounds the decoded batch, not the preceding protobuf transport.
fn decode(messages: Vec<proto::Operation>) -> Result<Vec<text::Operation>> {
    ensure!(
        messages.len() <= MAX_BATCH,
        "private native update batch exceeds budget"
    );
    let mut size = HistorySize::default();
    for message in &messages {
        let (id, value, version) = match message.variant.as_ref() {
            Some(proto::operation::Variant::Edit(edit)) => {
                ensure!(
                    edit.ranges.len() == edit.new_text.len(),
                    "invalid private native edit shape"
                );
                size.add(0, 1, edit.ranges.len())?;
                for text in &edit.new_text {
                    ensure!(!text.contains('\r'), "unnormalized private native edit");
                    size.add(text.len(), 0, 1)?;
                }
                ensure!(
                    size.text <= MAX_TEXT,
                    "private native update text exceeds budget"
                );
                let mut end = 0;
                for range in &edit.ranges {
                    ensure!(
                        range.start >= end
                            && range.start <= range.end
                            && range.end <= MAX_HISTORY_TEXT as u64,
                        "invalid private native edit range"
                    );
                    end = range.end;
                }
                (edit.replica_id, edit.lamport_timestamp, &edit.version)
            }
            Some(proto::operation::Variant::Undo(undo)) => {
                size.add(0, 1, undo.counts.len())?;
                let mut targets = std::collections::BTreeSet::new();
                for count in &undo.counts {
                    ensure!(
                        targets.insert(stamp(count.replica_id, count.lamport_timestamp)?)
                            && count.count <= MAX_OPERATIONS as u32,
                        "invalid private native undo count"
                    );
                }
                (undo.replica_id, undo.lamport_timestamp, &undo.version)
            }
            _ => bail!("private native updates accept only edit/undo operations"),
        };
        stamp(id, value)?;
        ensure!(
            version.len() <= MAX_VECTOR,
            "private native update vector exceeds budget"
        );
        let mut previous = None;
        for entry in version {
            ensure!(
                entry.replica_id < MAX_VECTOR as u32
                    && previous.is_none_or(|id| entry.replica_id > id)
                    && entry.timestamp < value,
                "invalid private native operation version"
            );
            previous = Some(entry.replica_id);
        }
    }
    messages
        .into_iter()
        .map(|message| match language_operation(message)? {
            Operation::Buffer(operation) => Ok(operation),
            _ => unreachable!("closed variants checked before decoding"),
        })
        .collect()
}

fn language_operation(message: proto::Operation) -> Result<Operation> {
    crate::proto::deserialize_operation(message)
}

impl Buffer {
    /// One native mutation turn: validate every operation against a bounded
    /// detached branch first, then publish only new, exact operations together.
    /// The scratch branch retains the original acquisition lineage. It has no
    /// subscriptions, language work, filesystem access or asynchronous effects.
    pub fn cowboy_apply_remote_updates(
        &mut self,
        messages: Vec<proto::Operation>,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let operations = decode(messages)?;
        ensure!(
            self.capability == Capability::ReadWrite,
            "private native buffer is not writable"
        );
        self.cowboy_check_replacement()?;
        ensure!(
            !self.has_deferred_ops(),
            "private native history is incomplete"
        );
        let mut size = self.cowboy_history_size()?;
        let mut preview = self.text.branch();
        let mut accepted: Vec<text::Operation> = Vec::new();
        for operation in operations {
            let timestamp = operation.timestamp();
            let lookup = |id: &clock::Lamport| {
                accepted
                    .iter()
                    .find(|op| op.timestamp() == *id)
                    .or_else(|| self.text.operations().get(id))
            };
            if let Some(previous) = lookup(&timestamp) {
                ensure!(
                    *previous == operation,
                    "private native operation identity changed"
                );
                continue;
            }
            let version = match &operation {
                text::Operation::Edit(edit) => &edit.version,
                text::Operation::Undo(undo) => &undo.version,
            };
            let current = preview.version();
            ensure!(
                current.observed_all(version)
                    && !current.observed(timestamp)
                    && version.get(timestamp.replica_id) == current.get(timestamp.replica_id),
                "private native operation has unavailable or conflicting history"
            );
            let base = clock::Lamport {
                replica_id: text::ReplicaId::LOCAL,
                value: 1,
            };
            ensure!(
                self.base_text().is_empty() || version.observed(base),
                "private native base is unobserved"
            );
            for entry in version.iter().filter(|entry| entry.value != 0) {
                ensure!(
                    (entry == base && !self.base_text().is_empty()) || lookup(&entry).is_some(),
                    "private native version names an unknown operation"
                );
            }
            for previous in self
                .text
                .operations()
                .iter()
                .map(|(_, op)| op)
                .chain(accepted.iter())
            {
                if version.observed(previous.timestamp()) {
                    let dependencies = match previous {
                        text::Operation::Edit(edit) => &edit.version,
                        text::Operation::Undo(undo) => &undo.version,
                    };
                    ensure!(
                        version.observed_all(dependencies),
                        "private native version omits causal history"
                    );
                }
            }
            match &operation {
                text::Operation::Edit(edit) => {
                    size.add(0, 1, edit.ranges.len())?;
                    for text in &edit.new_text {
                        size.add(text.len(), 0, 1)?;
                    }
                    ensure!(
                        preview.cowboy_check_full_ranges(version, &edit.ranges),
                        "invalid private native full-offset boundary"
                    );
                }
                text::Operation::Undo(undo) => {
                    size.add(0, 1, undo.counts.len())?;
                    for id in undo.counts.keys() {
                        ensure!(
                            version.observed(*id)
                                && lookup(id).is_some_and(text::Operation::is_edit),
                            "invalid private native undo target"
                        );
                    }
                }
            }
            preview.apply_ops([operation.clone()]);
            ensure!(
                preview.len() <= MAX_TEXT && !preview.has_deferred_ops(),
                "private native update result exceeds budget"
            );
            accepted.push(operation);
        }
        // Dropping the scratch state cannot publish text or consume undo groups.
        drop(preview);
        if !accepted.is_empty() {
            self.apply_ops(accepted.into_iter().map(Operation::Buffer), cx);
        }
        Ok(())
    }
}
