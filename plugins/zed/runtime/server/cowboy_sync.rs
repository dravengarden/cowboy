// SPDX-License-Identifier: GPL-3.0-or-later
//! Private conditional native effect. No public project API or path reload.
//! Prepared tickets may expire. Admitted effects never expire, replay, or
//! disappear when their HTTP/native observer disconnects. Restart loses this
//! process-local instance; it is not durable restoration.
use super::*;
use language::cowboy_replacement;
use proto::cowboy_buffer_sync::Action;
use proto::cowboy_buffer_sync_response::{Phase, Refusal};
use sha2::{Digest as _, Sha256};
use std::time::Duration;

const PROTOCOL: u32 = 1;
const MAX_RECORDS: usize = 256;
const MAX_BYTES: u32 = 4 * 1024 * 1024;
const PREPARE_TTL: Duration = Duration::from_secs(30);

#[cfg(test)]
mod tests;

pub(super) struct State {
    instance: [u8; 16],
    last_id: u64,
    records: HashMap<u64, Record>,
    #[cfg(test)]
    barriers: [Option<tests::Barrier>; 2],
}

impl Default for State {
    fn default() -> Self {
        Self {
            instance: rand::random(),
            last_id: 0,
            records: HashMap::default(),
            #[cfg(test)]
            barriers: Default::default(),
        }
    }
}

struct Record {
    sender: PeerId,
    buffer: Entity<Buffer>,
    file: Arc<dyn language::File>,
    expected: clock::Global,
    content_sha256: Vec<u8>,
    content_bytes: u32,
    created: Instant,
    reply: proto::CowboyBufferSyncResponse,
    task: Option<Task<()>>,
}

impl State {
    fn expire_prepared(&mut self) {
        self.records.retain(|_, record| {
            record.reply.phase != Phase::Prepared as i32 || record.created.elapsed() < PREPARE_TTL
        });
    }

    fn reply(&self, id: u64, phase: Phase) -> proto::CowboyBufferSyncResponse {
        proto::CowboyBufferSyncResponse {
            protocol: PROTOCOL,
            instance: self.instance.to_vec(),
            operation_id: id,
            phase: phase as i32,
            ..Default::default()
        }
    }
}

fn no_content(request: &proto::CowboyBufferSync) -> bool {
    request.buffer_id == 0
        && request.version.is_empty()
        && request.content_sha256.is_empty()
        && request.content_bytes == 0
}

fn valid_version(version: &[proto::CowboyBufferSyncVersion]) -> bool {
    version.len() <= 256
        && version
            .iter()
            .all(|entry| entry.replica_id <= u16::MAX as u32 && entry.timestamp > 0)
        && version
            .windows(2)
            .all(|pair| pair[0].replica_id < pair[1].replica_id)
}

impl BufferStore {
    pub(super) async fn handle_cowboy_buffer_sync(
        this: Entity<Self>,
        envelope: TypedEnvelope<proto::CowboyBufferSync>,
        mut cx: AsyncApp,
    ) -> Result<proto::CowboyBufferSyncResponse> {
        let sender = envelope.original_sender_id().unwrap_or_default();
        let request = envelope.payload;
        this.update(&mut cx, |this, cx| this.cowboy_sync(request, sender, cx))
    }

    fn cowboy_sync(
        &mut self,
        request: proto::CowboyBufferSync,
        sender: PeerId,
        cx: &mut Context<Self>,
    ) -> Result<proto::CowboyBufferSyncResponse> {
        anyhow::ensure!(
            matches!(self.state, BufferStoreState::Local(_))
                && request.project_id == proto::REMOTE_SERVER_PROJECT_ID
                && request.protocol == PROTOCOL,
            "unsupported private buffer synchronization"
        );
        self.cowboy_sync.expire_prepared();
        let action = Action::from_i32(request.action).context("invalid sync action")?;
        if action == Action::Probe {
            anyhow::ensure!(
                request.instance.is_empty() && request.operation_id == 0 && no_content(&request),
                "invalid sync probe"
            );
            return Ok(self.cowboy_sync.reply(0, Phase::Supported));
        }
        anyhow::ensure!(
            request.instance == self.cowboy_sync.instance,
            "sync instance ended"
        );
        if action == Action::Prepare {
            return self.cowboy_prepare_sync(request, sender, cx);
        }
        anyhow::ensure!(
            matches!(action, Action::Apply | Action::Query | Action::Retire)
                && no_content(&request)
                && request.operation_id > 0
                && request.operation_id <= self.cowboy_sync.last_id,
            "invalid original sync ticket"
        );
        let id = request.operation_id;
        let Some(record) = self.cowboy_sync.records.get(&id) else {
            // High-water identity is never recycled. An expired/retired ticket
            // cannot become a fresh effect or prove any buffer was released.
            return Ok(self.cowboy_sync.reply(id, Phase::Retired));
        };
        anyhow::ensure!(record.sender == sender, "sync ticket has another owner");
        let phase = record.reply.phase;
        if action == Action::Query || phase == Phase::Pending as i32 {
            return Ok(record.reply.clone());
        }
        if action == Action::Retire {
            self.cowboy_sync.records.remove(&id);
            return Ok(self.cowboy_sync.reply(id, Phase::Retired));
        }
        if phase != Phase::Prepared as i32 {
            return Ok(record.reply.clone()); // Duplicate Apply never repeats.
        }
        // At most one bounded source load is admitted per Store. Refusing a
        // competing Apply does not consume its still-effect-free ticket.
        anyhow::ensure!(
            !self
                .cowboy_sync
                .records
                .values()
                .any(|other| other.reply.phase == Phase::Pending as i32),
            "another native sync is pending"
        );
        let record = self.cowboy_sync.records.get_mut(&id).unwrap();
        let job = match record
            .buffer
            .read(cx)
            .cowboy_check_replacement()
            .and_then(|()| cowboy_replacement::acquire(cx))
        {
            Ok(job) => job,
            Err(_) => {
                record.reply.phase = Phase::Refused as i32;
                record.reply.refusal = Refusal::Budget as i32;
                return Ok(record.reply.clone());
            }
        };
        record.reply.phase = Phase::Pending as i32;
        let reply = record.reply.clone();
        let buffer = record.buffer.clone();
        let file = record.file.clone();
        let expected = record.expected.clone();
        let hash = record.content_sha256.clone();
        let bytes = record.content_bytes;
        // Store owns this task before the request returns. Dropping the request
        // or a transport response cannot cancel or re-admit the operation.
        record.task = Some(cx.spawn(async move |this, cx| {
            let outcome = async {
                #[cfg(test)]
                tests::pause(&this, cx, 0).await?;
                let source = this
                    .update(cx, |this, cx| {
                        this.cowboy_check_shared(&buffer, sender, cx)?;
                        let value = buffer.read(cx);
                        check_buffer(value, &file, &expected)?;
                        let local = File::from_dyn(Some(&file)).ok_or(Refusal::Source)?;
                        let fs = local
                            .worktree
                            .read(cx)
                            .as_local()
                            .ok_or(Refusal::Source)?
                            .fs()
                            .clone();
                        let path = file.as_local().ok_or(Refusal::Source)?.abs_path(cx);
                        let job = job.clone();
                        Ok::<_, Refusal>(cx.background_spawn(async move {
                            let result = fs
                                .cowboy_load_bytes_bounded(&path, MAX_BYTES as usize)
                                .await;
                            result.map(|bytes| job.hold(bytes))
                        }))
                    })
                    .map_err(|_| Refusal::Changed)??;
                let loaded = source.await.map_err(|_| Refusal::Source)?;
                let loaded = loaded.value;
                if loaded.len() != bytes as usize
                    || Sha256::digest(&loaded)[..] != hash[..]
                    || loaded.contains(&b'\r')
                    || loaded.starts_with(&[0xef, 0xbb, 0xbf])
                {
                    return Err(Refusal::Source);
                }
                let text = String::from_utf8(loaded).map_err(|_| Refusal::Source)?;
                let diff = buffer.update(cx, |buffer, cx| {
                    check_buffer(buffer, &file, &expected)?;
                    buffer
                        .cowboy_diff(text, job, cx)
                        .map_err(replacement_refusal)
                })?;
                let replacement = diff.await.map_err(replacement_refusal)?;
                #[cfg(test)]
                tests::pause(&this, cx, 1).await?;
                this.update(cx, |this, cx| {
                    this.cowboy_check_shared(&buffer, sender, cx)?;
                    buffer.update(cx, |buffer, cx| {
                        // This comparison and apply are one native mutation
                        // turn: no await, filesystem read or observer callback
                        // can insert an edit between the condition and write.
                        check_buffer(buffer, &file, &expected)?;
                        if replacement.diff().base_version != expected {
                            return Err(Refusal::Changed);
                        }
                        buffer
                            .cowboy_apply_replacement(replacement, cx)
                            .map_err(replacement_refusal)?;
                        buffer.did_reload(
                            buffer.version(),
                            LineEnding::Unix,
                            file.disk_state().mtime(),
                            cx,
                        );
                        Ok(serialize_version(&buffer.version())
                            .into_iter()
                            .map(|entry| proto::CowboyBufferSyncVersion {
                                replica_id: entry.replica_id,
                                timestamp: entry.timestamp,
                            })
                            .collect())
                    })
                })
                .map_err(|_| Refusal::Changed)?
            }
            .await;
            let _ = this.update(cx, |this, _| {
                // Pending records cannot expire or retire. A missing Store is
                // instance loss, not completion evidence in another process.
                if let Some(record) = this.cowboy_sync.records.get_mut(&id) {
                    match outcome {
                        Ok(version) => {
                            record.reply.phase = Phase::Applied as i32;
                            record.reply.version = version;
                            record.reply.content_sha256 = hash;
                            record.reply.content_bytes = bytes;
                        }
                        Err(reason) => {
                            record.reply.phase = Phase::Refused as i32;
                            record.reply.refusal = reason as i32;
                        }
                    }
                }
            });
        }));
        Ok(reply)
    }

    fn cowboy_check_shared(
        &self,
        buffer: &Entity<Buffer>,
        sender: PeerId,
        cx: &App,
    ) -> Result<(), Refusal> {
        let value = buffer.read(cx);
        let id = value.remote_id();
        let file = File::from_dyn(value.file()).ok_or(Refusal::Source)?;
        let worktree = &file.worktree;
        if self
            .worktree_store
            .read(cx)
            .worktree_for_id(worktree.read(cx).id(), cx)
            .as_ref()
            != Some(worktree)
        {
            return Err(Refusal::Changed);
        }
        let peers: Vec<_> = self
            .shared_buffers
            .iter()
            .filter(|(_, values)| values.contains_key(&id))
            .map(|(peer, _)| *peer)
            .collect();
        if peers == [sender] {
            Ok(())
        } else {
            Err(Refusal::Shared)
        }
    }

    fn cowboy_prepare_sync(
        &mut self,
        request: proto::CowboyBufferSync,
        sender: PeerId,
        cx: &mut Context<Self>,
    ) -> Result<proto::CowboyBufferSyncResponse> {
        anyhow::ensure!(
            request.operation_id == 0
                && request.content_bytes <= MAX_BYTES
                && request.content_sha256.len() == 32
                && valid_version(&request.version)
                && self.cowboy_sync.records.len() < MAX_RECORDS,
            "invalid sync preparation or capacity reached"
        );
        let buffer = self.get_existing(BufferId::new(request.buffer_id)?)?;
        self.cowboy_check_shared(&buffer, sender, cx)
            .map_err(|_| anyhow!("shared native buffer"))?;
        let value = buffer.read(cx);
        let file = value.file().context("native buffer has no file")?.clone();
        let expected = deserialize_version(
            &request
                .version
                .iter()
                .map(|entry| proto::VectorClockEntry {
                    replica_id: entry.replica_id,
                    timestamp: entry.timestamp,
                })
                .collect::<Vec<_>>(),
        );
        check_buffer(value, &file, &expected)
            .map_err(|_| anyhow!("native buffer preconditions changed"))?;
        let id = self
            .cowboy_sync
            .last_id
            .checked_add(1)
            .context("native sync tickets exhausted")?;
        let reply = self.cowboy_sync.reply(id, Phase::Prepared);
        self.cowboy_sync.records.insert(
            id,
            Record {
                sender,
                buffer,
                file,
                expected,
                content_sha256: request.content_sha256,
                content_bytes: request.content_bytes,
                created: Instant::now(),
                reply: reply.clone(),
                task: None,
            },
        );
        self.cowboy_sync.last_id = id;
        Ok(reply)
    }
}

fn replacement_refusal(reason: cowboy_replacement::Refusal) -> Refusal {
    match reason {
        cowboy_replacement::Refusal::Changed => Refusal::Changed,
        _ => Refusal::Budget,
    }
}

fn check_buffer(
    buffer: &Buffer,
    file: &Arc<dyn language::File>,
    expected: &clock::Global,
) -> Result<(), Refusal> {
    if !buffer.cowboy_can_sync()
        || buffer.len() > MAX_BYTES as usize
        || buffer.version() != *expected
        || !buffer
            .file()
            .is_some_and(|current| Arc::ptr_eq(current, file))
    {
        Err(Refusal::Changed)
    } else {
        Ok(())
    }
}
