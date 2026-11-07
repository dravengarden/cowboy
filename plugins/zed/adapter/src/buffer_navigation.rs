//! Private, finite navigation acquisition. Preparing allocates only an ID;
//! execution can cause native LSPs to share buffers and must never be replayed.
//! No Service authority or generic Machine routing is provided by this module.
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::content_reads::{Content, Point, Query};
use crate::{BufferOwner, Buffers, NavigationKind, Response, Zed, buffer_leases, diagnostics};

#[cfg(test)]
pub(crate) mod connected;
#[cfg(test)]
pub(crate) mod connected_lsp;
mod targets;
#[cfg(test)]
mod tests;

type Key = (PathBuf, PathBuf);
const MAX_NAVIGATIONS: usize = 32;
const MAX_DESTINATIONS: usize = 256;
const MAX_TARGETS: usize = 32;
const PREPARE_TTL: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NavigationRef {
    instance: String,
    // A disjoint wire namespace, not just a Rust newtype around a lease ID.
    id: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Action {
    Execute,
    Query,
    Release,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum State {
    Prepared,
    Unknown,
    Retained { locations: Vec<Location> },
    ReleaseUnknown,
    Released,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Location {
    path: PathBuf,
    content: Content,
    start: crate::LanguagePoint,
    end: crate::LanguagePoint,
}

pub(super) struct Target {
    pub(super) key: Key,
    pub(super) remote_id: u64,
    pub(super) revision: u64,
    location: Location,
}

impl Target {
    pub(super) fn check_owner(
        &self,
        owner: u64,
        active: &HashMap<Key, crate::BufferLease>,
    ) -> Result<()> {
        let buffer = active
            .get(&self.key)
            .context("original destination unavailable")?;
        ensure!(
            buffer.remote_id == self.remote_id
                && buffer.lease_ids.contains(&BufferOwner::Navigation(owner)),
            "original destination owner changed"
        );
        crate::sync_owners::ensure_readable(buffer)
    }
}

enum Phase {
    Prepared,
    Unknown,
    Retained,
    ReleaseUnknown,
}

struct Slot {
    key: Key,
    owner: u64,
    remote_id: u64,
    position: diagnostics::Position,
    kind: NavigationKind,
    prepared_at: Instant,
    phase: Phase,
    // Retain the original evidence even if close enqueue fails partway through.
    // A phase transition must not discard handles for possibly live resources.
    targets: Vec<Target>,
    native: Option<crate::navigation_native::Ticket>,
}

#[derive(Default)]
pub(super) struct Registry {
    instance: Option<String>,
    last_id: u64,
    slots: HashMap<u64, Slot>,
}

pub(super) async fn respond(
    request: crate::Request,
    buffers: &Buffers,
    zed: Option<&Zed>,
) -> Result<Response> {
    use crate::Request;
    match request {
        Request::PrepareBufferNavigation {
            lease,
            content,
            position,
            kind,
        } => prepare(lease, content, position, kind, buffers, zed).await,
        Request::BufferNavigation { navigation, action } => {
            buffers
                .navigations
                .lock()
                .await
                .act(navigation, action, buffers, zed)
                .await
        }
        Request::ReadBufferNavigation {
            navigation,
            destination,
            content,
            query,
        } => {
            buffers
                .navigations
                .lock()
                .await
                .read(navigation, destination, content, query, buffers, zed)
                .await
        }
        _ => anyhow::bail!("not an owned navigation request"),
    }
}

pub(super) async fn prepare(
    lease: buffer_leases::LeaseRef,
    content: Content,
    position: Point,
    kind: NavigationKind,
    buffers: &Buffers,
    zed: Option<&Zed>,
) -> Result<Response> {
    content.validate()?;
    let zed = zed.context("native navigation unavailable")?;
    // Lock order: leases -> navigation registry -> active map. Execution and
    // release never acquire the original lease registry while holding active.
    let mut owners = buffers.leases.lock().await;
    let (owner, key) = owners.sync_target(&lease)?;
    let mut registry = buffers.navigations.lock().await;
    registry.expire();
    ensure!(
        registry.slots.len() < MAX_NAVIGATIONS,
        "navigation capacity reached"
    );
    let active = buffers.active.read().await;
    crate::sync_owners::ensure_admission(buffers, &active)?;
    let buffer = active
        .get(&key)
        .context("original native buffer unavailable")?;
    ensure!(
        buffer.lease_ids.contains(&BufferOwner::Owned(owner)),
        "original buffer owner changed"
    );
    let position = {
        let cache = zed.diagnostics.lock().expect("diagnostic cache poisoned");
        ensure!(
            cache.match_content(buffer.remote_id, &content)?.is_some(),
            "navigation source content changed"
        );
        cache.position(buffer.remote_id, position.row, position.column)?
    };
    if registry.instance.is_none() {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random).context("navigation identity unavailable")?;
        registry.instance = Some(format!("{:032x}", u128::from_be_bytes(random)));
    }
    let id = registry
        .last_id
        .checked_add(1)
        .context("navigation identities exhausted")?;
    registry.last_id = id;
    registry.slots.insert(
        id,
        Slot {
            key,
            owner,
            remote_id: buffer.remote_id,
            position,
            kind,
            prepared_at: Instant::now(),
            phase: Phase::Prepared,
            targets: Vec::new(),
            native: None,
        },
    );
    Ok(registry.response(
        NavigationRef {
            instance: registry.instance.clone().expect("instance initialized"),
            id: format!("nav:{id:016x}"),
        },
        id,
    ))
}

impl Registry {
    fn expire(&mut self) {
        // Never expire effects or uncertainty. An expired preparation owns no
        // native resource and its monotonic ID is never recycled.
        self.slots.retain(|_, slot| {
            !matches!(slot.phase, Phase::Prepared) || slot.prepared_at.elapsed() < PREPARE_TTL
        });
    }

    fn resolve(&mut self, reference: &NavigationRef) -> Result<u64> {
        self.expire();
        ensure!(
            self.instance.as_ref() == Some(&reference.instance),
            "navigation belongs to another adapter instance"
        );
        let hex = reference
            .id
            .strip_prefix("nav:")
            .context("invalid navigation reference")?;
        ensure!(
            hex.len() == 16
                && hex
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "invalid navigation reference"
        );
        let id = u64::from_str_radix(hex, 16)?;
        ensure!(
            id != 0 && id <= self.last_id,
            "navigation was never prepared"
        );
        Ok(id)
    }

    fn response(&self, navigation: NavigationRef, id: u64) -> Response {
        let state = self
            .slots
            .get(&id)
            .map_or(State::Released, |slot| match &slot.phase {
                Phase::Prepared => State::Prepared,
                Phase::Unknown => State::Unknown,
                Phase::ReleaseUnknown => State::ReleaseUnknown,
                Phase::Retained => State::Retained {
                    // Saved observations, not fresh positions or an authority to
                    // reopen these paths. read() checks the original target epoch.
                    locations: slot
                        .targets
                        .iter()
                        .map(|target| target.location.clone())
                        .collect(),
                },
            });
        Response::OwnedBufferNavigation {
            api_version: crate::ADAPTER_VERSION,
            navigation,
            state,
        }
    }

    pub(super) async fn act(
        &mut self,
        navigation: NavigationRef,
        action: Action,
        buffers: &Buffers,
        zed: Option<&Zed>,
    ) -> Result<Response> {
        let id = self.resolve(&navigation)?;
        if let Some(slot) = self.slots.get(&id)
            && matches!(slot.phase, Phase::ReleaseUnknown)
            && matches!(action, Action::Query | Action::Release)
            && let Some(zed) = zed
        {
            let mut keys = std::collections::HashSet::from([slot.key.clone()]);
            keys.extend(slot.targets.iter().map(|target| target.key.clone()));
            let mut active = buffers.active.write().await;
            if crate::native_close::recover(&keys, &BufferOwner::Navigation(id), &mut active, zed)
                .await
                .unwrap_or(false)
            {
                self.slots.remove(&id);
            }
        }
        if let Some(slot) = self.slots.get_mut(&id) {
            match action {
                Action::Query if matches!(slot.phase, Phase::Unknown) => {
                    recover(
                        slot,
                        id,
                        buffers,
                        zed.context("native navigation unavailable")?,
                    )
                    .await?;
                }
                Action::Execute if matches!(slot.phase, Phase::Prepared) => {
                    execute(
                        slot,
                        id,
                        buffers,
                        zed.context("native navigation unavailable")?,
                    )
                    .await?;
                }
                Action::Release if matches!(slot.phase, Phase::Prepared) => {
                    self.slots.remove(&id);
                }
                Action::Release if matches!(slot.phase, Phase::Retained) => {
                    release(
                        slot,
                        id,
                        buffers,
                        zed.context("native navigation unavailable")?,
                    )
                    .await?;
                    self.slots.remove(&id);
                }
                // Saved-ID queries, repeated Execute and ambiguous Release do
                // not reissue LSPs or invent a new native owner.
                _ => {}
            }
        }
        Ok(self.response(navigation, id))
    }

    pub(super) async fn read(
        &mut self,
        navigation: NavigationRef,
        destination: u32,
        content: Content,
        query: Query,
        buffers: &Buffers,
        zed: Option<&Zed>,
    ) -> Result<Response> {
        let (id, target) = self.destination(&navigation, destination, &content)?;
        let zed = zed.context("native navigation unavailable")?;
        let active = buffers.active.read().await;
        target.check_owner(id, &active)?;
        zed.diagnostics
            .lock()
            .expect("diagnostic cache poisoned")
            .check(target.remote_id, target.revision)?;
        let result = zed.content_read(target.remote_id, &content, query).await?;
        // The content guard alone cannot detect an edit/undo since navigation.
        zed.diagnostics
            .lock()
            .expect("diagnostic cache poisoned")
            .check(target.remote_id, target.revision)?;
        Ok(Response::BufferNavigationRead {
            api_version: crate::ADAPTER_VERSION,
            navigation,
            destination,
            result,
        })
    }

    pub(super) fn destination(
        &mut self,
        navigation: &NavigationRef,
        destination: u32,
        content: &Content,
    ) -> Result<(u64, &Target)> {
        content.validate()?;
        let id = self.resolve(navigation)?;
        let slot = self.slots.get(&id).context("navigation was released")?;
        let Phase::Retained = slot.phase else {
            anyhow::bail!("navigation does not retain destinations");
        };
        let target = slot
            .targets
            .get(usize::try_from(destination)?)
            .context("unknown navigation destination")?;
        ensure!(
            *content == target.location.content,
            "navigation destination content changed"
        );
        Ok((id, target))
    }
}

async fn execute(slot: &mut Slot, id: u64, buffers: &Buffers, zed: &Zed) -> Result<()> {
    let mut active = buffers.active.write().await;
    ensure!(
        slot.prepared_at.elapsed() < PREPARE_TTL,
        "navigation preparation expired while queued"
    );
    crate::sync_owners::ensure_admission(buffers, &active)?;
    let source = active
        .get_mut(&slot.key)
        .context("original navigation source unavailable")?;
    ensure!(
        source.remote_id == slot.remote_id
            && source.lease_ids.contains(&BufferOwner::Owned(slot.owner)),
        "original navigation source owner changed"
    );
    zed.diagnostics
        .lock()
        .expect("diagnostic cache poisoned")
        .check(slot.remote_id, slot.position.revision)?;
    slot.native = Some(
        crate::navigation_native::Ticket::prepare(
            zed,
            crate::navigation_request(
                slot.remote_id,
                &slot.position.version,
                slot.position.anchor.clone(),
                slot.kind,
            ),
        )
        .await?,
    );
    // Native navigation may open unknown target IDs. Retain a process-wide
    // admission fence BEFORE the first native await, including cancellation.
    source.lease_ids.insert(BufferOwner::NavigationPending(id));
    slot.phase = Phase::Unknown;
    if let Some(responses) = slot
        .native
        .as_ref()
        .expect("prepared native operation")
        .observe(zed, true)
        .await?
    {
        retain_result(slot, id, responses, &mut active, zed).await?;
    }
    Ok(())
}

async fn recover(slot: &mut Slot, id: u64, buffers: &Buffers, zed: &Zed) -> Result<()> {
    let mut active = buffers.active.write().await;
    let source = active
        .get(&slot.key)
        .context("original source unavailable")?;
    ensure!(
        source.remote_id == slot.remote_id
            && source
                .lease_ids
                .contains(&BufferOwner::NavigationPending(id)),
        "original navigation fence unavailable"
    );
    if let Some(ticket) = &slot.native
        && let Some(responses) = ticket.observe(zed, false).await?
    {
        retain_result(slot, id, responses, &mut active, zed).await?;
    }
    Ok(())
}

async fn retain_result(
    slot: &mut Slot,
    id: u64,
    responses: Vec<proto::LspResponse>,
    active: &mut HashMap<Key, crate::BufferLease>,
    zed: &Zed,
) -> Result<()> {
    let (targets, _) = targets::retain(slot, id, responses, active, zed).await?;
    // Native protocol two already registered targets before saving its receipt.
    slot.targets = targets;
    let cache = zed.diagnostics.lock().expect("diagnostic cache poisoned");
    cache.check(slot.remote_id, slot.position.revision)?;
    for target in &slot.targets {
        cache.check(target.remote_id, target.revision)?;
    }
    let source = active
        .get_mut(&slot.key)
        .expect("source held through navigation");
    source.lease_ids.remove(&BufferOwner::NavigationPending(id));
    source.lease_ids.insert(BufferOwner::Navigation(id));
    slot.phase = Phase::Retained;
    Ok(())
}

async fn release(slot: &mut Slot, id: u64, buffers: &Buffers, zed: &Zed) -> Result<()> {
    // Queue receipt-only retirement before losing the last local ticket. A full
    // queue leaves this retained operation retryable without another close.
    if let Some(ticket) = &slot.native {
        ticket.forget(zed)?;
        slot.native = None;
    }
    let mut active = buffers.active.write().await;
    let Phase::Retained = slot.phase else {
        unreachable!()
    };
    let mut keys = HashMap::from([(slot.key.clone(), slot.remote_id)]);
    for target in &slot.targets {
        keys.insert(target.key.clone(), target.remote_id);
    }
    for (key, remote_id) in &keys {
        let buffer = active
            .get(key)
            .context("original navigation resource unavailable")?;
        ensure!(
            buffer.remote_id == *remote_id
                && buffer.lease_ids.contains(&BufferOwner::Navigation(id)),
            "original navigation resource changed"
        );
        crate::sync_owners::ensure_readable(buffer)?;
    }
    // One bounded native set, not a series that may release only a prefix.
    // Confirmation is original peer removal, not LSP quiescence or recovery.
    crate::native_close::release(
        keys.into_keys().collect(),
        &BufferOwner::Navigation(id),
        &mut active,
        Some(zed),
        &buffers.native_open,
        || slot.phase = Phase::ReleaseUnknown,
    )
    .await
}
