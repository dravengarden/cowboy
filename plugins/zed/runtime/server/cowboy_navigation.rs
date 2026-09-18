// SPDX-License-Identifier: GPL-3.0-or-later
//! Private, two-phase navigation: collect/validate ALL LSP responses, then
//! acquire targets only through the source's original worktree. No invisible
//! worktree discovery, archive extraction, partial success or error omission.
use super::*;
use crate::Location;
use proto::LspRequestMessage as _;
use proto::cowboy_navigation_response::{Outcome, Refusal};
use proto::{Message as _, PeerId};
use std::path::Component;
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_SERVERS: usize = 4;
const MAX_LOCATIONS: usize = 256;
const MAX_TARGETS: usize = 32;
const MAX_BYTES: usize = 4 * 1024 * 1024;

#[cfg(test)]
mod tests;

#[derive(Default)]
pub(super) struct State(Arc<AtomicBool>);

struct Admission(Arc<AtomicBool>);
impl Drop for Admission {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
impl State {
    fn admit(&self) -> Result<Admission, Refusal> {
        self.0
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Refusal::Budget)?;
        Ok(Admission(self.0.clone()))
    }
}

struct Unresolved {
    origin: Option<lsp::Range>,
    uri: lsp::Uri,
    range: lsp::Range,
}

trait Locations {
    fn locations(self) -> Result<Vec<Unresolved>, Refusal>;
}
impl Locations for Option<lsp::GotoDefinitionResponse> {
    fn locations(self) -> Result<Vec<Unresolved>, Refusal> {
        use lsp::GotoDefinitionResponse::*;
        let count = match &self {
            None => 0,
            Some(Scalar(_)) => 1,
            Some(Array(v)) => v.len(),
            Some(Link(v)) => v.len(),
        };
        if count > MAX_LOCATIONS {
            return Err(Refusal::Budget);
        }
        Ok(match self {
            None => Vec::new(),
            Some(Scalar(v)) => vec![Unresolved {
                origin: None,
                uri: v.uri,
                range: v.range,
            }],
            Some(Array(v)) => v
                .into_iter()
                .map(|v| Unresolved {
                    origin: None,
                    uri: v.uri,
                    range: v.range,
                })
                .collect(),
            Some(Link(v)) => v
                .into_iter()
                .map(|v| Unresolved {
                    origin: v.origin_selection_range,
                    uri: v.target_uri,
                    range: v.target_selection_range,
                })
                .collect(),
        })
    }
}
impl Locations for Option<Vec<lsp::Location>> {
    fn locations(self) -> Result<Vec<Unresolved>, Refusal> {
        Self::map_or_else(
            self,
            || Ok(Vec::new()),
            |v| {
                if v.len() > MAX_LOCATIONS {
                    return Err(Refusal::Budget);
                }
                Ok(v.into_iter()
                    .map(|v| Unresolved {
                        origin: None,
                        uri: v.uri,
                        range: v.range,
                    })
                    .collect())
            },
        )
    }
}

trait Navigation: LspCommand {
    fn response(links: Vec<LocationLink>) -> Self::Response;
    fn position(request: &Self::ProtoRequest) -> Option<proto::Anchor>;
}
macro_rules! links {
    ($($command:ty),*) => { $(impl Navigation for $command {
        fn response(links: Vec<LocationLink>) -> Self::Response { links }
        fn position(request: &Self::ProtoRequest) -> Option<proto::Anchor> { request.position.clone() }
    })* };
}
links!(
    GetDefinitions,
    GetDeclarations,
    GetTypeDefinitions,
    GetImplementations
);
impl Navigation for GetReferences {
    fn position(request: &Self::ProtoRequest) -> Option<proto::Anchor> {
        request.position.clone()
    }
    fn response(links: Vec<LocationLink>) -> Self::Response {
        links.into_iter().map(|v| v.target).collect()
    }
}

struct Planned {
    origin: Option<lsp::Range>,
    path: Arc<RelPath>,
    range: lsp::Range,
}

// Count duplicate locations too; aliases only share the target allocation.
// Pure validation finishes for every server before the first target open.
fn plan(
    root: &Path,
    responses: Vec<(LanguageServerId, Vec<Unresolved>)>,
) -> Result<Vec<(LanguageServerId, Vec<Planned>)>, Refusal> {
    if responses.len() > MAX_SERVERS {
        return Err(Refusal::Budget);
    }
    let mut locations = 0usize;
    let mut targets = HashSet::default();
    let mut result = Vec::new();
    for (server, response) in responses {
        locations = locations
            .checked_add(response.len())
            .ok_or(Refusal::Budget)?;
        if locations > MAX_LOCATIONS {
            return Err(Refusal::Budget);
        }
        let mut planned = Vec::new();
        for value in response {
            if value.uri.as_str().len() > 8192
                || value.uri.scheme().as_str() != "file"
                || value.range.start > value.range.end
                || value
                    .origin
                    .as_ref()
                    .is_some_and(|range| range.start > range.end)
            {
                return Err(Refusal::Target);
            }
            let absolute = value
                .uri
                .to_file_path_ext(PathStyle::local())
                .map_err(|_| Refusal::Target)?;
            if !absolute.is_absolute()
                || absolute
                    .components()
                    .any(|c| !matches!(c, Component::RootDir | Component::Normal(_)))
            {
                return Err(Refusal::Target);
            }
            let relative = absolute.strip_prefix(root).map_err(|_| Refusal::Target)?;
            if relative.as_os_str().is_empty()
                || relative.as_os_str().len() > 4096
                || relative.as_os_str().as_encoded_bytes().contains(&0)
                || relative
                    .components()
                    .any(|c| !matches!(c, Component::Normal(_)))
            {
                return Err(Refusal::Target);
            }
            let path = RelPath::new(relative, PathStyle::local())
                .map_err(|_| Refusal::Target)?
                .into_arc();
            targets.insert(path.clone());
            if targets.len() > MAX_TARGETS {
                return Err(Refusal::Budget);
            }
            planned.push(Planned {
                origin: value.origin,
                path,
                range: value.range,
            });
        }
        result.push((server, planned));
    }
    Ok(result)
}

fn range(buffer: &Buffer, range: lsp::Range) -> Result<std::ops::Range<Anchor>, Refusal> {
    let start = PointUtf16::new(range.start.line, range.start.character);
    let end = PointUtf16::new(range.end.line, range.end.character);
    if start > end
        || buffer.len() > MAX_BYTES
        || buffer.clip_point_utf16(Unclipped(start), Bias::Left) != start
        || buffer.clip_point_utf16(Unclipped(end), Bias::Left) != end
    {
        return Err(Refusal::Target);
    }
    Ok(buffer.anchor_after(start)..buffer.anchor_before(end))
}

impl LspStore {
    pub(super) async fn handle_cowboy_navigation(
        this: Entity<Self>,
        envelope: TypedEnvelope<proto::CowboyNavigation>,
        mut cx: AsyncApp,
    ) -> Result<proto::CowboyNavigationResponse> {
        let sender = envelope.original_sender_id().unwrap_or_default();
        let request = envelope.payload;
        anyhow::ensure!(
            request.protocol == 1
                && request.project_id == proto::REMOTE_SERVER_PROJECT_ID
                && request.query.len() <= 8192,
            "invalid private navigation request"
        );
        let mut response = proto::CowboyNavigationResponse {
            protocol: 1,
            ..Default::default()
        };
        if request.query.is_empty() {
            anyhow::ensure!(
                this.read_with(&cx, |this, _| this.as_local().is_some()),
                "navigation requires local store"
            );
            response.outcome = Outcome::Supported as i32;
            return Ok(response);
        }
        let query = proto::LspQuery::decode(request.query.as_slice())?;
        anyhow::ensure!(
            query.project_id == proto::REMOTE_SERVER_PROJECT_ID
                && query.server_id.is_none()
                && query.lsp_request_id == 0,
            "invalid private navigation shape"
        );
        let admission = this.read_with(&cx, |this, _| this.cowboy_navigation.admit());
        let outcome = match admission {
            Err(reason) => Err(reason),
            Ok(_admission) => {
                let timer = cx.background_executor().timer(Duration::from_secs(20));
                let work = async {
                    use proto::lsp_query::Request;
                    match query.request.ok_or(Refusal::Source)? {
                        Request::GetDefinition(v) => {
                            Self::cowboy_navigate::<GetDefinitions>(
                                this.clone(),
                                v,
                                sender,
                                &mut cx,
                            )
                            .await
                        }
                        Request::GetDeclaration(v) => {
                            Self::cowboy_navigate::<GetDeclarations>(
                                this.clone(),
                                v,
                                sender,
                                &mut cx,
                            )
                            .await
                        }
                        Request::GetTypeDefinition(v) => {
                            Self::cowboy_navigate::<GetTypeDefinitions>(
                                this.clone(),
                                v,
                                sender,
                                &mut cx,
                            )
                            .await
                        }
                        Request::GetImplementation(v) => {
                            Self::cowboy_navigate::<GetImplementations>(
                                this.clone(),
                                v,
                                sender,
                                &mut cx,
                            )
                            .await
                        }
                        Request::GetReferences(v) => {
                            Self::cowboy_navigate::<GetReferences>(this.clone(), v, sender, &mut cx)
                                .await
                        }
                        _ => Err(Refusal::Source),
                    }
                };
                futures::pin_mut!(work, timer);
                match select(work, timer).await {
                    Either::Left((result, _)) => result,
                    Either::Right(_) => Err(Refusal::Deadline),
                }
            }
        };
        match outcome {
            Ok(result) => {
                response.outcome = Outcome::Complete as i32;
                response.result = result;
            }
            Err(reason) => {
                response.outcome = Outcome::Refused as i32;
                response.refusal = reason as i32;
            }
        }
        Ok(response)
    }

    async fn cowboy_navigate<T>(
        this: Entity<Self>,
        request: T::ProtoRequest,
        sender: PeerId,
        cx: &mut AsyncApp,
    ) -> Result<Vec<u8>, Refusal>
    where
        T: Navigation,
        T::ProtoRequest: proto::LspRequestMessage,
        <T::LspRequest as lsp::request::Request>::Result: Locations,
        <T::ProtoRequest as proto::RequestMessage>::Response:
            Into<<T::ProtoRequest as proto::LspRequestMessage>::Response>,
    {
        let entries = request.buffer_version();
        if entries.len() > 256
            || entries
                .iter()
                .any(|v| v.replica_id > u16::MAX as u32 || v.timestamp == 0)
            || entries
                .windows(2)
                .any(|v| v[0].replica_id >= v[1].replica_id)
        {
            return Err(Refusal::Source);
        }
        let expected = deserialize_version(entries);
        let position = T::position(&request).ok_or(Refusal::Source)?;
        if position.replica_id > u16::MAX as u32
            || position.offset > u32::MAX as u64
            || position.buffer_id != Some(request.buffer_id())
        {
            return Err(Refusal::Source);
        }
        let position = deserialize_anchor(position).ok_or(Refusal::Source)?;
        let id = BufferId::new(request.buffer_id()).map_err(|_| Refusal::Source)?;
        let buffer = this
            .read_with(cx, |this, cx| this.buffer_store.read(cx).get_existing(id))
            .map_err(|_| Refusal::Source)?;
        let (file, worktree, root) = buffer.read_with(cx, |value, cx| {
            if value.version() != expected
                || value.len() > MAX_BYTES
                || !value.can_resolve(&position)
            {
                return Err(Refusal::Source);
            }
            let file = value.file().ok_or(Refusal::Source)?.clone();
            let local = File::from_dyn(Some(&file)).ok_or(Refusal::Source)?;
            if !local.worktree.read(cx).is_local() {
                return Err(Refusal::Source);
            }
            Ok((
                file.clone(),
                local.worktree.clone(),
                local.worktree.read(cx).abs_path(),
            ))
        })?;
        let command = T::from_proto(request, this.clone(), buffer.clone(), cx.clone())
            .await
            .map_err(|_| Refusal::Source)?;
        let check = |this: &LspStore, cx: &App| -> Result<(), Refusal> {
            let source = buffer.read(cx);
            if source.version() != expected
                || !source
                    .file()
                    .is_some_and(|current| Arc::ptr_eq(current, &file))
                || worktree.read(cx).abs_path() != root
                || this
                    .worktree_store
                    .read(cx)
                    .worktree_for_id(worktree.read(cx).id(), cx)
                    .as_ref()
                    != Some(&worktree)
            {
                return Err(Refusal::Source);
            }
            Ok(())
        };
        let servers = this.read_with(cx, |this, cx| {
            check(this, cx)?;
            let local = this.as_local().ok_or(Refusal::Source)?;
            let scope = buffer.read(cx).snapshot().language_scope_at(position);
            // Snapshot only registered, capable servers. No startup/download or
            // unbounded fanout can occur while constructing the request batch.
            let servers = local
                .language_servers_for_buffer(buffer.read(cx), cx)
                .filter(|(adapter, _)| {
                    scope
                        .as_ref()
                        .is_none_or(|scope| scope.language_allowed(&adapter.name))
                })
                .filter(|(_, server)| {
                    command.check_capabilities(server.adapter_server_capabilities())
                })
                .filter(|(_, server)| {
                    local
                        .buffers_opened_in_servers
                        .get(&id)
                        .is_some_and(|ids| ids.contains(&server.server_id()))
                })
                .take(MAX_SERVERS + 1)
                .map(|(_, server)| server.clone())
                .collect::<Vec<_>>();
            if servers.len() > MAX_SERVERS {
                return Err(Refusal::Budget);
            }
            Ok(servers)
        })?;
        let mut pending = FuturesUnordered::new();
        for server in servers {
            let params = buffer.read_with(cx, |buffer, cx| {
                command
                    .to_lsp(
                        &file.as_local().ok_or(Refusal::Source)?.abs_path(cx),
                        buffer,
                        &server,
                        cx,
                    )
                    .map_err(|_| Refusal::Source)
            })?;
            pending.push(async move {
                let reply = server
                    .request::<T::LspRequest>(params, Duration::from_secs(5))
                    .await
                    .into_response()
                    .map_err(|_| Refusal::LanguageServer)?;
                Ok::<_, Refusal>((server.server_id(), reply.locations()?))
            });
        }
        let mut raw = Vec::new();
        while let Some(response) = pending.next().await {
            raw.push(response?);
        }
        let planned = plan(&root, raw)?;
        // No target can be opened before this complete batch has passed plan.
        let worktree_id = worktree.read_with(cx, |value, _| value.id());
        let mut targets = HashMap::default();
        for (_, values) in &planned {
            for value in values {
                if targets.contains_key(&value.path) {
                    continue;
                }
                let target = this
                    .update(cx, |this, cx| {
                        check(this, cx)?;
                        Ok::<_, Refusal>(this.buffer_store.update(cx, |store, cx| {
                            store.open_buffer(
                                ProjectPath {
                                    worktree_id,
                                    path: value.path.clone(),
                                },
                                cx,
                            )
                        }))
                    })?
                    .await
                    .map_err(|_| Refusal::Target)?;
                targets.insert(value.path.clone(), target);
            }
        }
        this.update(cx, |this, cx| {
            check(this, cx)?;
            // Validate every target and range before sharing or publishing any
            // result. Invalid UTF-16 coordinates refuse; never silently clip.
            let mut converted = Vec::new();
            for (server, values) in planned {
                let mut links = Vec::new();
                for value in values {
                    let target = &targets[&value.path];
                    let native = target.read(cx);
                    let local = File::from_dyn(native.file()).ok_or(Refusal::Target)?;
                    if local.worktree != worktree || local.path.as_ref() != value.path.as_ref() {
                        return Err(Refusal::Target);
                    }
                    links.push(LocationLink {
                        origin: value
                            .origin
                            .map(|r| {
                                range(buffer.read(cx), r).map(|range| Location {
                                    buffer: buffer.clone(),
                                    range,
                                })
                            })
                            .transpose()?,
                        target: Location {
                            buffer: target.clone(),
                            range: range(native, value.range)?,
                        },
                    });
                }
                converted.push((server, links));
            }
            let responses = converted
                .into_iter()
                .map(|(server, links)| proto::LspResponse {
                    server_id: server.to_proto(),
                    response: Some(T::ProtoRequest::response_to_proto_query(
                        T::response_to_proto(T::response(links), this, sender, &expected, cx)
                            .into(),
                    )),
                })
                .collect();
            Ok(proto::LspQueryResponse {
                project_id: proto::REMOTE_SERVER_PROJECT_ID,
                lsp_request_id: 0,
                responses,
            }
            .encode_to_vec())
        })
    }
}
