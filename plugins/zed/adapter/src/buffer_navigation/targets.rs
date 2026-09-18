//! Capture and retain original native IDs. No filesystem canonicalization or
//! `OpenBuffer` fallback, and no partially published/truncated target list.
use super::*;
use crate::{BufferLease, BufferVersionEntry};
use std::collections::HashSet;
use std::path::{Component, Path};

pub(super) async fn retain(
    slot: &Slot,
    id: u64,
    responses: Vec<proto::LspResponse>,
    active: &mut HashMap<Key, BufferLease>,
    zed: &Zed,
) -> Result<(Vec<Target>, Vec<u64>)> {
    let locations = locations(responses, slot.kind)?;
    let native_ids: HashSet<_> = locations
        .iter()
        .map(|location| location.buffer_id)
        .collect();
    ensure!(
        native_ids.len() <= MAX_TARGETS && !native_ids.contains(&0),
        "native navigation targets exceed limits or have invalid IDs"
    );
    wait_for_initial_shares(slot, &native_ids, zed).await?;
    let files = zed.buffer_files.read().await;
    let worktrees = zed.worktree_paths.read().await;
    let cache = zed.diagnostics.lock().expect("diagnostic cache poisoned");
    cache.check(slot.remote_id, slot.position.revision)?;
    let source = files
        .get(&slot.remote_id)
        .context("native navigation source file unavailable")?;
    ensure!(
        worktrees.get(&source.worktree_id) == Some(&slot.key.0),
        "native navigation source worktree changed"
    );
    let mut targets = Vec::with_capacity(locations.len());
    let mut additions: HashMap<Key, (u64, Vec<BufferVersionEntry>)> = HashMap::new();
    for location in locations {
        let file = files
            .get(&location.buffer_id)
            .context("native navigation file unavailable")?;
        ensure!(
            file.worktree_id == source.worktree_id,
            "navigation outside original worktree is not admitted"
        );
        let path = Path::new(&file.path);
        ensure!(
            !file.path.is_empty()
                && file.path.len() <= 4096
                && !file.path.contains('\0')
                && !path.is_absolute()
                && path
                    .components()
                    .all(|part| matches!(part, Component::Normal(_))),
            "invalid native navigation path"
        );
        let key = (slot.key.0.clone(), path.to_path_buf());
        if let Some(buffer) = active.get(&key) {
            ensure!(
                buffer.remote_id == location.buffer_id,
                "navigation path belongs to a different retained buffer"
            );
            crate::sync_owners::ensure_readable(buffer)?;
        }
        if let Some((remote_id, _)) = additions.get(&key) {
            ensure!(
                *remote_id == location.buffer_id,
                "navigation path aliases distinct native buffers"
            );
        }
        let start = location.start.context("native navigation start missing")?;
        let end = location.end.unwrap_or_else(|| start.clone());
        let (start, end) = cache.range(location.buffer_id, &start, &end)?;
        let content = cache.content(location.buffer_id)?;
        content.validate()?;
        let revision = cache.revision(location.buffer_id)?;
        additions.insert(
            key.clone(),
            (location.buffer_id, cache.version(location.buffer_id)?),
        );
        targets.push(Target {
            key,
            remote_id: location.buffer_id,
            revision,
            location: Location {
                path: path.to_path_buf(),
                content,
                start,
                end,
            },
        });
    }
    // All validation precedes mutation. active is held across native query
    // and capture, cache through commit: no close/sync/epoch gap is introduced.
    let mut unregistered: Vec<_> = native_ids
        .into_iter()
        .filter(|id| !active.values().any(|buffer| buffer.remote_id == *id))
        .collect();
    unregistered.sort_unstable();
    for (key, (remote_id, version)) in additions {
        active
            .entry(key)
            .or_insert_with(|| BufferLease {
                lease_ids: HashSet::new(),
                remote_id,
                version,
                sync: None,
                closing: false,
            })
            .lease_ids
            .insert(BufferOwner::Navigation(id));
    }
    Ok((targets, unregistered))
}

async fn wait_for_initial_shares(slot: &Slot, ids: &HashSet<u64>, zed: &Zed) -> Result<()> {
    // Zed's create_buffer_for_peer shares asynchronously. An LSP result can
    // precede its target State/last Chunk. Subscribe before checking the cache;
    // wakeups are hints, never evidence or permission to open/retry a pathname.
    let mut events = zed.events.subscribe();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            {
                let cache = zed.diagnostics.lock().expect("diagnostic cache poisoned");
                cache.check(slot.remote_id, slot.position.revision)?;
                if ids.iter().all(|id| cache.initial_share_complete(*id)) {
                    return anyhow::Ok(());
                }
            }
            events
                .recv()
                .await
                .context("native navigation share stream unavailable")?;
        }
    })
    .await
    .context("native navigation initial share timed out")?
}

fn locations(
    responses: Vec<proto::LspResponse>,
    kind: NavigationKind,
) -> Result<Vec<proto::Location>> {
    use proto::lsp_response::Response as Reply;
    let mut locations = Vec::new();
    for response in responses {
        let links = match (kind, response.response) {
            (NavigationKind::Definition, Some(Reply::GetDefinitionResponse(value))) => value.links,
            (NavigationKind::Declaration, Some(Reply::GetDeclarationResponse(value))) => {
                value.links
            }
            (NavigationKind::TypeDefinition, Some(Reply::GetTypeDefinitionResponse(value))) => {
                value.links
            }
            (NavigationKind::Implementation, Some(Reply::GetImplementationResponse(value))) => {
                value.links
            }
            (NavigationKind::References, Some(Reply::GetReferencesResponse(value))) => {
                ensure!(
                    locations.len() + value.locations.len() <= MAX_DESTINATIONS,
                    "native navigation locations exceed limits"
                );
                locations.extend(value.locations);
                continue;
            }
            _ => anyhow::bail!("unexpected native navigation response"),
        };
        ensure!(
            locations.len() + links.len() <= MAX_DESTINATIONS,
            "native navigation locations exceed limits"
        );
        for link in links {
            locations.push(link.target.context("native navigation target missing")?);
        }
    }
    Ok(locations)
}
