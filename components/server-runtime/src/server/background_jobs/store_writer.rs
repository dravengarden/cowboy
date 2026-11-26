//! Write-behind store writer batching, transcript anchor rebasing, and
//! acknowledged lineage-freeze / clear serialization.

use std::collections::VecDeque;

use super::super::*;
use super::apply_non_event_write;

/// Cap retained failed fire-and-forget non-event writes (such as InsertSession
/// and PutSetting) so a sustained DB outage cannot grow memory unboundedly.
const MAX_PENDING_NON_EVENTS: usize = 2048;

struct PendingNonEvents {
    queue: VecDeque<StoreWrite>,
}

impl PendingNonEvents {
    fn new() -> Self {
        Self {
            queue: VecDeque::new(),
        }
    }

    fn is_clean(&self) -> bool {
        self.queue.is_empty()
    }

    fn at_capacity(&self) -> bool {
        self.queue.len() >= MAX_PENDING_NON_EVENTS
    }

    fn is_retriable(write: &StoreWrite) -> bool {
        !matches!(
            write,
            StoreWrite::UpdatePending {
                ack_id: Some(_),
                ..
            } | StoreWrite::Barrier { .. }
                | StoreWrite::ClearEvents { .. }
                | StoreWrite::FreezeLineagePrefix { .. }
        )
    }

    async fn drain(&mut self, store: &Store, acks: Option<&crate::core::StoreAckSink>) -> bool {
        while let Some(front) = self.queue.front() {
            if !apply_non_event_write(store, front, acks).await {
                return false;
            }
            self.queue.pop_front();
        }
        true
    }

    async fn apply_or_retain(
        &mut self,
        store: &Store,
        write: StoreWrite,
        acks: Option<&crate::core::StoreAckSink>,
    ) -> bool {
        let drained = self.drain(store, acks).await;
        if !Self::is_retriable(&write) {
            if !drained {
                if let StoreWrite::UpdatePending {
                    ack_id: Some(id), ..
                } = write
                    && let Some(acks) = acks
                {
                    acks.complete(
                        id,
                        Err("Blocked by an earlier failed durable write; retry shortly".to_owned()),
                    );
                }
                return false;
            }
            return apply_non_event_write(store, &write, acks).await;
        }
        if !self.queue.is_empty() {
            self.queue.push_back(write);
            return false;
        }
        if apply_non_event_write(store, &write, acks).await {
            true
        } else {
            self.queue.push_back(write);
            false
        }
    }
}

struct PendingEventBatch {
    events: BTreeMap<(String, u64), Envelope>,
    highwaters: HashMap<String, u64>,
    anchors: Vec<crate::transcript_anchors::AnchorWrite>,
    occurrences: Vec<crate::task_notifications::Occurrence>,
}

impl PendingEventBatch {
    fn new() -> Self {
        Self {
            events: BTreeMap::new(),
            highwaters: HashMap::new(),
            anchors: Vec::new(),
            occurrences: Vec::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.events.is_empty()
            && self.highwaters.is_empty()
            && self.anchors.is_empty()
            && self.occurrences.is_empty()
    }

    fn has_uncommitted_for(&self, session_id: &str) -> bool {
        self.events.keys().any(|(sid, _)| sid == session_id)
            || self.highwaters.contains_key(session_id)
            || self.anchors.iter().any(|a| a.session_id == session_id)
    }

    async fn flush(&mut self, store: &Store, reducer: &mut EventReducer) -> bool {
        flush_event_batch(
            store,
            reducer,
            &mut self.events,
            &mut self.highwaters,
            &mut self.anchors,
            &mut self.occurrences,
        )
        .await
    }
}

/// Drain write-behind intents in small batches. Streaming text/tool updates are
/// reduced to stable rows before persistence, and transient usage/session-info
/// frames only advance the durable sequence watermark.
#[cfg(test)]
pub async fn run_store_writer(
    store: Store,
    rx: StoreReceiver,
    health: Arc<PersistenceHealth>,
    shutdown: watch::Receiver<bool>,
) {
    run_store_writer_with_acks(store, rx, health, shutdown, None).await;
}

pub async fn run_store_writer_with_acks(
    store: Store,
    rx: StoreReceiver,
    health: Arc<PersistenceHealth>,
    shutdown: watch::Receiver<bool>,
    acks: Option<crate::core::StoreAckSink>,
) {
    let mut initial_floors = Vec::new();
    match store.load_all_lineage_edges().await {
        Ok(edges) => {
            for edge in edges {
                if edge.segment_id.is_none() && !edge.display_detached && edge.before_seq > 0 {
                    initial_floors.push((edge.parent_session_id, edge.before_seq));
                }
            }
        }
        Err(error) => {
            tracing::error!(%error, "store writer failed to load lineage freeze floors");
            health.mark_failed_batch();
            if let Some(acks) = acks {
                acks.fail_all("store writer failed to load lineage state");
            }
            return;
        }
    }
    if let Ok(reserved_floors) = store.load_reserved_duplicate_freeze_floors().await {
        initial_floors.extend(reserved_floors);
    }
    run_store_writer_with_floors(store, rx, health, shutdown, acks, initial_floors).await;
}

pub async fn run_store_writer_with_floors(
    store: Store,
    mut rx: StoreReceiver,
    health: Arc<PersistenceHealth>,
    mut shutdown: watch::Receiver<bool>,
    acks: Option<crate::core::StoreAckSink>,
    initial_freeze_floors: Vec<(String, u64)>,
) {
    let mut reducer = EventReducer::default();
    for (session_id, before_seq) in initial_freeze_floors {
        reducer.set_freeze_floor(&session_id, before_seq);
    }
    // Retained across loop iterations so failed event batches and failed
    // session/setting writes are retried in FIFO order and never silently
    // acknowledged by a later Barrier or FreezeLineagePrefix.
    let mut pending = PendingEventBatch::new();
    let mut pending_non_events = PendingNonEvents::new();
    let mut draining = *shutdown.borrow();
    if draining {
        rx.close();
    }
    loop {
        if pending_non_events.at_capacity()
            && !draining
            && !pending_non_events.drain(&store, acks.as_ref()).await
        {
            health.mark_failed_batch();
            tokio::select! {
                biased;
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        rx.close();
                        draining = true;
                    }
                }
                () = tokio::time::sleep(std::time::Duration::from_millis(250)) => {}
            }
            continue;
        }
        let first = tokio::select! {
            biased;
            changed = shutdown.changed(), if !draining => {
                if changed.is_err() || *shutdown.borrow() {
                    rx.close();
                    draining = true;
                }
                continue;
            }
            write = rx.recv() => write,
        };
        let Some(first) = first else { break };
        let mut bytes = crate::core::estimated_store_write_bytes(&first);
        let mut batch = vec![first];
        while batch.len() < 256 && bytes < crate::core::STORE_BATCH_MAX_BYTES {
            match tokio::time::timeout(std::time::Duration::from_millis(10), rx.recv()).await {
                Ok(Some(write)) => {
                    bytes = bytes.saturating_add(crate::core::estimated_store_write_bytes(&write));
                    batch.push(write);
                }
                Ok(None) | Err(_) => break,
            }
        }
        if !apply_store_batch_with_acks(
            &store,
            &mut reducer,
            &mut pending,
            &mut pending_non_events,
            batch,
            acks.as_ref(),
        )
        .await
        {
            health.mark_failed_batch();
        }
    }
    if !pending_non_events.drain(&store, acks.as_ref()).await {
        health.mark_failed_batch();
    }
    if !pending.is_empty() && !pending.flush(&store, &mut reducer).await {
        health.mark_failed_batch();
    }
    if let Some(acks) = acks {
        acks.fail_all("store writer shut down");
    }
    tracing::info!("store writer shutting down (channel closed)");
}

#[cfg(test)]
pub async fn apply_store_batch(
    store: &Store,
    reducer: &mut EventReducer,
    writes: Vec<StoreWrite>,
) -> bool {
    let mut pending = PendingEventBatch::new();
    let mut pending_non_events = PendingNonEvents::new();
    apply_store_batch_with_acks(
        store,
        reducer,
        &mut pending,
        &mut pending_non_events,
        writes,
        None,
    )
    .await
}

async fn apply_append_event(
    store: &Store,
    reducer: &mut EventReducer,
    pending: &mut PendingEventBatch,
    env: Envelope,
) {
    pending
        .occurrences
        .extend(notifications::occurrence_of(&env));
    pending
        .highwaters
        .entry(env.session_id.clone())
        .and_modify(|seq| *seq = (*seq).max(env.seq.saturating_add(1)))
        .or_insert_with(|| env.seq.saturating_add(1));
    let anchor = match &env.event {
        Event::Update { update } => crate::transcript_anchors::anchor_key(update),
        _ => None,
    };
    let pending_anchor = anchor.as_deref().and_then(|key| {
        pending
            .anchors
            .iter()
            .find(|write| write.session_id == env.session_id && write.key == key)
            .map(|write| write.seq)
    });
    let mut unresolved = None;
    let floor = reducer.freeze_floor(&env.session_id);
    if let (Some(key), Some(seq)) = (anchor.as_deref(), pending_anchor) {
        if seq >= floor {
            reducer.adopt_anchor(&env.session_id, key, seq);
        } else {
            pending
                .anchors
                .push(crate::transcript_anchors::AnchorWrite {
                    session_id: env.session_id.clone(),
                    key: key.to_owned(),
                    seq: env.seq,
                    rebase: true,
                });
        }
    } else if let Some(key) = anchor.as_deref() {
        if reducer.knows_anchor(&env.session_id, key) {
            if reducer
                .anchor_seq(&env.session_id, key)
                .is_some_and(|seq| seq < floor)
            {
                pending
                    .anchors
                    .push(crate::transcript_anchors::AnchorWrite {
                        session_id: env.session_id.clone(),
                        key: key.to_owned(),
                        seq: env.seq,
                        rebase: true,
                    });
            }
        } else {
            match resolve_durable_anchor(store, &env.session_id, key).await {
                Ok(Some(seq)) if seq >= floor => reducer.adopt_anchor(&env.session_id, key, seq),
                Ok(Some(_)) => pending
                    .anchors
                    .push(crate::transcript_anchors::AnchorWrite {
                        session_id: env.session_id.clone(),
                        key: key.to_owned(),
                        seq: env.seq,
                        rebase: true,
                    }),
                Ok(None) => pending
                    .anchors
                    .push(crate::transcript_anchors::AnchorWrite {
                        session_id: env.session_id.clone(),
                        key: key.to_owned(),
                        seq: env.seq,
                        rebase: false,
                    }),
                Err(error) => {
                    tracing::warn!(%error, session = %env.session_id, key, "transcript anchor lookup failed");
                    unresolved = Some((env.session_id.clone(), key.to_owned()));
                }
            }
        }
    }
    if let Some(reduced) = reducer.reduce(env) {
        pending
            .events
            .insert((reduced.session_id.clone(), reduced.seq), reduced);
    }
    if let Some((session_id, key)) = unresolved {
        reducer.forget_anchor(&session_id, &key);
    }
}

async fn apply_clear_events(
    store: &Store,
    reducer: &mut EventReducer,
    pending: &mut PendingEventBatch,
    write: &StoreWrite,
    acks: Option<&crate::core::StoreAckSink>,
) -> bool {
    let StoreWrite::ClearEvents {
        session_id, ack_id, ..
    } = write
    else {
        return true;
    };
    let flushed = pending.flush(store, reducer).await && !pending.has_uncommitted_for(session_id);
    if !flushed {
        if let (Some(id), Some(acks)) = (ack_id, acks) {
            acks.complete(
                *id,
                Err("Clear aborted: pending transcript events could not be flushed".to_owned()),
            );
        }
        return false;
    }
    let cleared = retry_store_write(store, write).await;
    if cleared {
        reducer.clear_session(session_id);
    }
    if let (Some(id), Some(acks)) = (ack_id, acks) {
        acks.complete(
            *id,
            if cleared {
                Ok(true)
            } else {
                Err("Clear was not saved".to_owned())
            },
        );
    }
    cleared
}

async fn apply_store_batch_with_acks(
    store: &Store,
    reducer: &mut EventReducer,
    pending: &mut PendingEventBatch,
    pending_non_events: &mut PendingNonEvents,
    writes: Vec<StoreWrite>,
    acks: Option<&crate::core::StoreAckSink>,
) -> bool {
    let mut usages: BTreeMap<String, StoreWrite> = BTreeMap::new();
    let mut ok = true;
    for write in writes {
        match write {
            StoreWrite::FreezeLineagePrefix {
                session_id,
                before_seq,
                ack_id,
            } => {
                reducer.set_freeze_floor(&session_id, before_seq);
                if let Some(id) = ack_id {
                    let flushed = pending.flush(store, reducer).await
                        && !pending.has_uncommitted_for(&session_id);
                    ok &= flushed;
                    if let Some(acks) = acks {
                        acks.complete(
                            id,
                            if flushed {
                                Ok(true)
                            } else {
                                Err("Pending events could not be flushed before lineage freeze"
                                    .to_owned())
                            },
                        );
                    }
                }
            }
            StoreWrite::Barrier { ack_id: Some(id) } => {
                let drained = pending_non_events.drain(store, acks).await;
                let flushed = pending.flush(store, reducer).await && pending.is_empty();
                let all_durable = drained && pending_non_events.is_clean() && flushed;
                ok &= all_durable;
                if let Some(acks) = acks {
                    acks.complete(
                        id,
                        if all_durable {
                            Ok(true)
                        } else {
                            Err("Pending store writes could not be persisted before barrier"
                                .to_owned())
                        },
                    );
                }
            }
            StoreWrite::RecordOccurrence(occurrence) => pending.occurrences.push(occurrence),
            StoreWrite::UpdateSessionUsage { ref session_id, .. } => {
                usages.insert(session_id.clone(), write);
            }
            StoreWrite::AppendEvent(env) => {
                apply_append_event(store, reducer, pending, env).await;
            }
            StoreWrite::ClearEvents { .. } => {
                ok &= pending_non_events.drain(store, acks).await;
                ok &= apply_clear_events(store, reducer, pending, &write, acks).await;
            }
            other => {
                ok &= pending.flush(store, reducer).await;
                ok &= pending_non_events.apply_or_retain(store, other, acks).await;
            }
        }
    }
    ok &= pending_non_events.drain(store, acks).await;
    ok &= pending.flush(store, reducer).await;
    for write in usages.into_values() {
        ok &= retry_store_write(store, &write).await;
    }
    ok
}
