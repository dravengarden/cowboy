// SPDX-License-Identifier: GPL-3.0-or-later
//! Finite sync/reload admission, not a budget for every upstream writer or RSS.
use super::*;
use gpui::{EntityId, Global};
use std::sync::atomic::{AtomicUsize, Ordering};

pub const MAX_JOBS: usize = 4;
pub const MAX_TEXT: usize = 4 * 1024 * 1024;
pub const MAX_HISTORY_TEXT: usize = 8 * 1024 * 1024;
pub const MAX_OPERATIONS: usize = 4096;
pub const MAX_PARTS: usize = 16 * 1024;
pub const MAX_DIFF_EDITS: usize = 1024;
const MAX_VECTOR: usize = 256;

#[derive(Default)]
struct Budget(Arc<AtomicUsize>);
impl Global for Budget {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Capacity,
    History,
    Input,
    Changed,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Capacity => "private native replacement capacity exceeded",
            Self::History => "private native replacement history exceeds budget",
            Self::Input => "private native replacement input exceeds budget",
            Self::Changed => "private native replacement source changed",
        })
    }
}
impl std::error::Error for Refusal {}

/// One shared charge across the actual job and its completed results.
#[derive(Clone)]
pub struct Job {
    _charge: Arc<Charge>,
}
struct Charge(Arc<AtomicUsize>);

pub fn acquire(cx: &mut App) -> Result<Job, Refusal> {
    let used = &cx.default_global::<Budget>().0;
    used.fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
        (count < MAX_JOBS).then_some(count + 1)
    })
    .map_err(|_| Refusal::Capacity)?;
    Ok(Job {
        _charge: Arc::new(Charge(used.clone())),
    })
}

impl Drop for Charge {
    fn drop(&mut self) {
        let previous = self.0.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(previous > 0);
    }
}

pub struct Held<T> {
    pub value: T,
    pub job: Job,
}

impl Job {
    pub fn hold<T>(self, value: T) -> Held<T> {
        Held { value, job: self }
    }
}

/// Not Clone or serializable. Only the exact native entity/version may consume it.
pub struct Replacement {
    source: EntityId,
    held: Held<Diff>,
}

impl Replacement {
    pub fn diff(&self) -> &Diff {
        &self.held.value
    }
}

#[derive(Default)]
struct HistorySize {
    text: usize,
    operations: usize,
    parts: usize,
}

impl HistorySize {
    fn add(&mut self, text: usize, operations: usize, parts: usize) -> Result<(), Refusal> {
        self.text = self.text.checked_add(text).ok_or(Refusal::History)?;
        self.operations = self
            .operations
            .checked_add(operations)
            .ok_or(Refusal::History)?;
        self.parts = self.parts.checked_add(parts).ok_or(Refusal::History)?;
        if self.text > MAX_HISTORY_TEXT
            || self.operations > MAX_OPERATIONS
            || self.parts > MAX_PARTS
        {
            return Err(Refusal::History);
        }
        Ok(())
    }
}

impl Buffer {
    fn cowboy_history_size(&self) -> Result<HistorySize, Refusal> {
        let mut size = HistorySize::default();
        size.add(self.base_text().len(), 0, 0)?;
        for (_, operation) in self.text.operations().iter() {
            let version = match operation {
                text::Operation::Edit(edit) => {
                    size.add(0, 1, edit.ranges.len())?;
                    for text in &edit.new_text {
                        size.add(text.len(), 0, 1)?;
                    }
                    &edit.version
                }
                text::Operation::Undo(undo) => {
                    size.add(0, 1, undo.counts.len())?;
                    &undo.version
                }
            };
            if version.iter().take(MAX_VECTOR + 1).count() > MAX_VECTOR {
                return Err(Refusal::History);
            }
        }
        if self.text.replica_id().as_u16() as usize >= MAX_VECTOR
            || self.version().iter().take(MAX_VECTOR + 1).count() > MAX_VECTOR
            || self.text.deferred_ops_len() != 0
        {
            return Err(Refusal::History);
        }
        Ok(size)
    }

    /// Check retained history before starting another load/diff. No serialization
    /// or history clone is needed, and traversal stops at the first exceeded cap.
    pub fn cowboy_check_replacement(&self) -> Result<(), Refusal> {
        if self.len() > MAX_TEXT {
            return Err(Refusal::Input);
        }
        self.cowboy_history_size().map(|_| ())
    }

    pub fn cowboy_diff(
        &self,
        mut new_text: String,
        job: Job,
        cx: &Context<Self>,
    ) -> Result<Task<Result<Replacement, Refusal>>, Refusal> {
        self.cowboy_check_replacement()?;
        if new_text.len() > MAX_TEXT {
            return Err(Refusal::Input);
        }
        let old_text = self.as_rope().clone();
        let base_version = self.version();
        let source = cx.entity_id();
        Ok(cx.background_spawn(async move {
            // Charge lives in this worker, not only its cancellable observer.
            let old_text = old_text.to_string();
            let line_ending = LineEnding::detect(&new_text);
            LineEnding::normalize(&mut new_text);
            let edits = crate::text_diff::cowboy_text_diff(&old_text, &new_text, MAX_DIFF_EDITS)
                .ok_or(Refusal::Input)?;
            Ok(Replacement {
                source,
                held: job.hold(Diff {
                    base_version,
                    line_ending,
                    edits,
                }),
            })
        }))
    }

    pub fn cowboy_apply_replacement(
        &mut self,
        replacement: Replacement,
        cx: &mut Context<Self>,
    ) -> Result<Option<Transaction>, Refusal> {
        if replacement.source != cx.entity_id() || replacement.diff().base_version != self.version()
        {
            return Err(Refusal::Changed);
        }
        self.cowboy_check_replacement()?;
        let mut size = self.cowboy_history_size()?;
        let diff = replacement.held.value;
        if !diff.edits.is_empty() {
            size.add(0, 1, 0)?;
            for (_, text) in &diff.edits {
                size.add(text.len(), 0, 2)?;
            }
        }
        // No await or mutation precedes the complete capacity check. Rejection
        // cannot finalize an undo group, change encoding, mark saved or prune.
        self.finalize_last_transaction();
        self.apply_diff(diff, cx);
        let transaction = self.finalize_last_transaction().cloned();
        drop(replacement.held.job);
        Ok(transaction)
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn in_use(cx: &App) -> usize {
    cx.try_global::<Budget>()
        .map_or(0, |value| value.0.load(Ordering::Acquire))
}
