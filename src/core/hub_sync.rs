//! The Hub's synchronized states: rename, reorder, folder and mobile-review mutations, sync apply and resync, and the queue projections they publish.

use super::*;

impl Hub {
    /// Apply a rename to the typed truth (NO `Sessions` re-broadcast — the sync
    /// channel carries the title now). Persists so fresh-connect + restart show
    /// it. Mirror of [`Self::rename_session`] minus the broadcast.
    pub(super) fn apply_rename(&self, session_id: &str, title: String) {
        {
            let mut sessions = self.inner.sessions.lock();
            if let Some(s) = sessions.get_mut(session_id) {
                s.meta.title.clone_from(&title);
            }
        }
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::UpdateTitle {
                session_id: session_id.to_owned(),
                title,
            });
        }
    }

    /// Apply a session reorder to the order list (NO broadcast — the sync channel
    /// carries it). Mirror of [`Self::reorder_sessions`] minus the broadcast.
    /// Submitted ids only permute names they include; every existing id survives.
    pub(super) fn apply_reorder(&self, order: &[String]) {
        {
            let mut list = self.inner.order.lock();
            *list = merge_session_order(&list, order);
        }
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let order = self.inner.order.lock().clone();
            let _ = tx.send(StoreWrite::UpdateSessionOrder { order });
        }
    }

    /// Apply a sidebar-folders mutation to the typed tree and persist what it
    /// changed: the owner's whole folder set and any explicit placements.
    /// Mirror of [`Self::apply_reorder`]; the sync channel carries the value.
    pub(super) fn apply_folders(
        &self,
        actor: &crate::session_folders::FolderActor,
        mutation: &str,
        args: &serde_json::Value,
    ) -> Result<(), String> {
        let (effects, folders) = {
            let mut state = self.inner.folders.lock();
            let effects = state.apply(actor, mutation, args)?;
            let folders = effects
                .replaced_owner
                .as_ref()
                .map(|owner| state.folders_of(owner.as_deref()));
            (effects, folders)
        };
        if let Some(tx) = self.inner.store_tx.as_ref() {
            if let (Some(owner_user_id), Some(folders)) = (effects.replaced_owner, folders) {
                let _ = tx.send(StoreWrite::ReplaceSessionFolders {
                    owner_user_id,
                    folders,
                });
            }
            if !effects.placements.is_empty() {
                let _ = tx.send(StoreWrite::UpdateSessionPlacement {
                    placements: effects.placements,
                });
            }
        }
        Ok(())
    }

    pub(super) fn apply_mobile_review(
        &self,
        session_id: &str,
        mutation: &str,
        args: &serde_json::Value,
    ) -> Result<(), String> {
        let persisted = {
            let mut sessions = self.inner.sessions.lock();
            let session = sessions
                .get_mut(session_id)
                .ok_or_else(|| "unknown mobile review session".to_owned())?;
            let state = &mut session.mobile_review;
            match mutation {
                "setDirectory" => {
                    state.directory = if args.get("inherit") == Some(&serde_json::Value::Bool(true))
                    {
                        if args.get("directory").is_some() {
                            return Err("inherit cannot also set a directory".into());
                        }
                        None
                    } else {
                        match args.get("directory") {
                            Some(serde_json::Value::Null) => Some(None),
                            Some(serde_json::Value::String(value))
                                if crate::review_directory::valid(value) =>
                            {
                                Some(Some(value.clone()))
                            }
                            _ => return Err("invalid Zed default directory".into()),
                        }
                    };
                }
                "setRemoteReview" => {
                    let binding = match args.get("binding") {
                        Some(serde_json::Value::Null) => None,
                        Some(value) => {
                            let binding: crate::workspace_extensions::ReviewBinding =
                                serde_json::from_value(value.clone())
                                    .map_err(|_| "invalid PR association")?;
                            if !binding.valid() {
                                return Err("invalid PR association".to_owned());
                            }
                            Some(binding)
                        }
                        None => return Err("missing PR association".to_owned()),
                    };
                    state.remote_review = binding;
                }
                "selectRemoteReview" => {
                    state.remote_selected = args
                        .get("selected")
                        .and_then(serde_json::Value::as_bool)
                        .ok_or("invalid PR selection")?;
                }
                "open" => {
                    let path = mobile_review_string_arg(args, "path", 4096)?;
                    if !valid_mobile_review_path(&path) {
                        return Err("invalid mobile review path".to_owned());
                    }
                    if !state.tabs.iter().any(|tab| tab.path == path) {
                        if state.tabs.len() >= MOBILE_REVIEW_TAB_CAP {
                            let evict = state.tabs.iter().position(|tab| !tab.pinned).unwrap_or(0);
                            let removed = state.tabs.remove(evict);
                            if state.active.as_deref() == Some(&removed.path) {
                                state.active = None;
                            }
                        }
                        state.tabs.push(MobileReviewTab {
                            path: path.clone(),
                            pinned: false,
                        });
                    }
                    state.active = Some(path);
                    state.mode = "files".to_owned();
                    state.open_revision = state.open_revision.saturating_add(1);
                    if args.get("reveal") == Some(&serde_json::Value::Bool(true)) {
                        state.reveal_revision = state.reveal_revision.saturating_add(1);
                        state.remote_selected = false;
                    }
                }
                "close" => {
                    let path = mobile_review_string_arg(args, "path", 4096)?;
                    state.tabs.retain(|tab| tab.path != path);
                    if state.active.as_deref() == Some(&path) {
                        state.active = state.tabs.last().map(|tab| tab.path.clone());
                    }
                }
                "reorder" => {
                    let order = args
                        .get("paths")
                        .and_then(serde_json::Value::as_array)
                        .ok_or("reorder: missing paths")?
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .filter(|path| valid_mobile_review_path(path))
                        .map(str::to_owned)
                        .collect::<Vec<_>>();
                    sort_by_id_order(&mut state.tabs, &order, |tab| &tab.path);
                }
                "setPinned" => {
                    let path = mobile_review_string_arg(args, "path", 4096)?;
                    let pinned = args
                        .get("pinned")
                        .and_then(serde_json::Value::as_bool)
                        .ok_or("setPinned: missing pinned")?;
                    if let Some(tab) = state.tabs.iter_mut().find(|tab| tab.path == path) {
                        tab.pinned = pinned;
                    }
                }
                "activate" => {
                    let path = args.get("path").and_then(serde_json::Value::as_str);
                    state.active = path
                        .filter(|path| state.tabs.iter().any(|tab| tab.path == *path))
                        .map(str::to_owned);
                }
                "setMode" => {
                    let mode = mobile_review_string_arg(args, "mode", 16)?;
                    if !matches!(mode.as_str(), "files" | "git") {
                        return Err("invalid mobile review mode".to_owned());
                    }
                    state.mode = mode;
                }
                "setComparison" => {
                    state.comparison = match args.get("comparison") {
                        None | Some(serde_json::Value::Null) => None,
                        Some(value) => Some(
                            value
                                .as_str()
                                .filter(|value| valid_mobile_review_comparison(value))
                                .map(str::to_owned)
                                .ok_or("setComparison: invalid comparison")?,
                        ),
                    };
                }
                "bind" => {
                    let context = mobile_review_string_arg(args, "context", 1024)?;
                    if !valid_mobile_review_binding(&context) {
                        return Err("bind: invalid context".to_owned());
                    }
                    if !state.bindings.contains(&context) {
                        if state.bindings.len() >= MOBILE_REVIEW_BINDING_CAP {
                            return Err("bind: too many bindings".to_owned());
                        }
                        state.bindings.push(context);
                    }
                }
                "unbind" => {
                    let context = mobile_review_string_arg(args, "context", 1024)?;
                    state.bindings.retain(|bound| *bound != context);
                }
                "markReviewed" => {
                    let key = mobile_review_string_arg(args, "key", 2048)?;
                    match args.get("revision").and_then(serde_json::Value::as_str) {
                        Some(revision) if !revision.is_empty() && revision.len() <= 512 => {
                            if state.progress.len() >= MOBILE_REVIEW_PROGRESS_CAP
                                && !state.progress.contains_key(&key)
                                && let Some(oldest) = state.progress.keys().next().cloned()
                            {
                                state.progress.remove(&oldest);
                            }
                            state.progress.insert(key, revision.to_owned());
                        }
                        None => {
                            state.progress.remove(&key);
                        }
                        _ => return Err("invalid review revision".to_owned()),
                    }
                }
                "setPosition" => {
                    let path = mobile_review_string_arg(args, "path", 4096)?;
                    if !valid_mobile_review_path(&path) {
                        return Err("invalid mobile review path".to_owned());
                    }
                    let line = args
                        .get("line")
                        .and_then(serde_json::Value::as_u64)
                        .and_then(|line| u32::try_from(line).ok())
                        .filter(|line| *line > 0)
                        .ok_or("setPosition: invalid line")?;
                    let revision = match args.get("revision") {
                        None | Some(serde_json::Value::Null) => None,
                        Some(value) => Some(
                            value
                                .as_str()
                                .filter(|revision| !revision.is_empty() && revision.len() <= 512)
                                .map(str::to_owned)
                                .ok_or("setPosition: invalid revision")?,
                        ),
                    };
                    if state.positions.len() >= MOBILE_REVIEW_POSITION_CAP
                        && !state.positions.contains_key(&path)
                        && let Some(oldest) = state.positions.keys().next().cloned()
                    {
                        state.positions.remove(&oldest);
                    }
                    state
                        .positions
                        .insert(path, MobileReviewPosition { line, revision });
                }
                _ => return Err(format!("unknown mobile review mutation {mutation}")),
            }
            serde_json::to_value(state).map_err(|error| error.to_string())?
        };
        if let Some(tx) = self.inner.store_tx.as_ref() {
            let _ = tx.send(StoreWrite::UpdateMobileReviewState {
                session_id: session_id.to_owned(),
                value: persisted,
            });
        }
        Ok(())
    }

    /// The derived JSON value of one synced state — what a `SyncPatch` carries and
    /// the client folds. Always read live from the typed truth (so it's durable by
    /// derivation, no shadow copy to drift).
    #[must_use]
    pub fn sync_value(&self, state: &str) -> serde_json::Value {
        match state {
            "title" => {
                let sessions = self.inner.sessions.lock();
                let map: serde_json::Map<String, serde_json::Value> = sessions
                    .values()
                    .map(|s| {
                        (
                            s.meta.id.clone(),
                            serde_json::Value::String(s.meta.title.clone()),
                        )
                    })
                    .collect();
                serde_json::Value::Object(map)
            }
            "order" => {
                let list = self.inner.order.lock();
                serde_json::Value::Array(
                    list.iter()
                        .map(|id| serde_json::Value::String(id.clone()))
                        .collect(),
                )
            }
            "folders" => self.inner.folders.lock().value(),
            "workspace-order" => serde_json::to_value(&*self.inner.workspace_orders.lock())
                .expect("workspace orders serialize"),
            "drafts" => serde_json::to_value(&*self.inner.draft_announcements.lock())
                .expect("draft announcements serialize"),
            "preferences" => serde_json::to_value(&*self.inner.preference_announcements.lock())
                .expect("preference announcements serialize"),
            _ if state.starts_with("mobile-review:") => {
                let session_id = &state["mobile-review:".len()..];
                let sessions = self.inner.sessions.lock();
                sessions
                    .get(session_id)
                    .and_then(|session| serde_json::to_value(&session.mobile_review).ok())
                    .unwrap_or(serde_json::Value::Null)
            }
            _ => serde_json::Value::Null,
        }
    }

    /// Whether `id` was already applied for `state` (a retried delivery), without
    /// consuming it.
    pub(super) fn sync_already_seen(&self, state: &str, id: &str) -> bool {
        self.inner
            .sync
            .lock()
            .get(state)
            .is_some_and(|entry| entry.seen.contains(id))
    }

    /// Record `id` as seen for `state`; returns true if it's NEW (first delivery).
    pub(super) fn sync_first_seen(&self, state: &str, id: &str) -> bool {
        let mut reg = self.inner.sync.lock();
        reg.entry(state.to_owned())
            .or_default()
            .seen
            .insert(id.to_owned())
    }

    /// Cmids carried inside a queue/drafts value — the confirm set for the queue
    /// sync state (the client drops an optimistic add the moment its cmid lands).
    pub(super) fn cmids_of(queue: &[QueuedMessage], drafts: &[QueuedMessage]) -> Vec<String> {
        queue
            .iter()
            .chain(drafts.iter())
            .filter_map(|m| m.cmid.clone())
            .collect()
    }

    /// Bump `state`'s version and broadcast a LIVE (version-gated) SyncPatch with
    /// the given absolute value + confirm set.
    pub(super) fn sync_emit(&self, state: &str, value: serde_json::Value, confirmed: Vec<String>) {
        let version = {
            let mut reg = self.inner.sync.lock();
            let e = reg.entry(state.to_owned()).or_default();
            e.version += 1;
            e.version
        };
        // Op-log: every AUTHORITATIVE state change, one line → journald → vector
        // → VictoriaLogs. Lets you (or an AI) replay "how state X reached version
        // N" via LogsQL. Low volume (user-paced changes, not per-agent-event).
        tracing::info!(target: "cowboy::oplog", op = "change", %state, version, confirmed = ?confirmed);
        self.fanout(Outbound::SyncPatch {
            state: state.to_owned(),
            version,
            value,
            confirmed,
            resync: false,
        });
    }

    /// Version-stamp `state` and broadcast its derived value, confirming the given
    /// mutation ids.
    pub(super) fn sync_broadcast(&self, state: &str, confirmed: Vec<String>) {
        let value = self.sync_value(state);
        self.sync_emit(state, value, confirmed);
    }

    /// Generic arbiter apply — see [`Inbound::Sync`]. Validate+parse (rejects a
    /// bad/unknown mutation before burning the id), dedupe (retry = no-op), apply
    /// the typed mutation, then version-stamp + broadcast. Returns an error string
    /// the server surfaces to the user.
    pub fn sync_apply(
        &self,
        state: &str,
        id: String,
        name: &str,
        args: &serde_json::Value,
    ) -> Result<(), String> {
        self.sync_apply_as(
            &crate::session_folders::FolderActor::local(),
            state,
            id,
            name,
            args,
        )
    }

    /// [`Self::sync_apply`] on behalf of a product principal: `actor` scopes
    /// the `"folders"` state to the principal's own tree.
    pub fn sync_apply_as(
        &self,
        actor: &crate::session_folders::FolderActor,
        state: &str,
        id: String,
        name: &str,
        args: &serde_json::Value,
    ) -> Result<(), String> {
        let _guard = self.inner.sync_apply_gate.lock();
        self.sync_apply_as_locked(actor, state, id, name, args)
    }

    pub(super) fn sync_apply_as_locked(
        &self,
        actor: &crate::session_folders::FolderActor,
        state: &str,
        id: String,
        name: &str,
        args: &serde_json::Value,
    ) -> Result<(), String> {
        enum Op {
            WorkspaceOrder {
                owner: String,
                order: Vec<String>,
            },
            Rename {
                session_id: String,
                title: String,
            },
            Reorder {
                order: Vec<String>,
            },
            Folders {
                mutation: String,
                args: serde_json::Value,
            },
            MobileReview {
                session_id: String,
                mutation: String,
                args: serde_json::Value,
            },
        }
        let op = match (state, name) {
            ("workspace-order", "reorder") => {
                let values = args
                    .get("order")
                    .and_then(serde_json::Value::as_array)
                    .ok_or("workspace order is missing")?;
                if values.len() > 12000 {
                    return Err("workspace order is too large".to_owned());
                }
                let mut order = Vec::new();
                for value in values {
                    let key = value.as_str().ok_or("invalid workspace item")?;
                    let valid = key
                        .strip_prefix("draft:")
                        .or_else(|| key.strip_prefix("session:"));
                    if valid.is_none_or(|id| {
                        id.is_empty()
                            || id.len() > 160
                            || !id
                                .bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                    }) {
                        return Err("invalid workspace item".to_owned());
                    }
                    if !order.iter().any(|existing| existing == key) {
                        order.push(key.to_owned());
                    }
                }
                Op::WorkspaceOrder {
                    owner: actor.user_id.clone().unwrap_or_default(),
                    order,
                }
            }
            ("title", "rename") => {
                let session_id = args
                    .get("session_id")
                    .and_then(serde_json::Value::as_str)
                    .ok_or("rename: missing session_id")?
                    .to_owned();
                let title = args
                    .get("title")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_owned();
                if title.is_empty() {
                    return Err("title cannot be empty".to_owned());
                }
                Op::Rename { session_id, title }
            }
            ("order", "reorder") => {
                let order = args
                    .get("order")
                    .and_then(serde_json::Value::as_array)
                    .ok_or("reorder: missing order")?
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect();
                Op::Reorder { order }
            }
            ("folders", name) => {
                // A retried delivery must not be re-validated against the state
                // it already changed ("folder id already exists"). Confirm it
                // again: the resending client may have missed the first patch.
                if self.sync_already_seen(state, &id) {
                    self.sync_broadcast(state, vec![id]);
                    return Ok(());
                }
                // The dedupe set does not survive a Controller restart, but a
                // create that already produced exactly this folder is the same
                // retried delivery. Confirm it instead of rejecting the replay.
                if name == "create" && self.inner.folders.lock().is_replayed_create(actor, args) {
                    self.sync_first_seen(state, &id);
                    self.sync_broadcast(state, vec![id]);
                    return Ok(());
                }
                // Dry-run against a copy so a rejected mutation never consumes
                // its id: the client keeps the error, not a silent no-op retry.
                self.inner.folders.lock().clone().apply(actor, name, args)?;
                Op::Folders {
                    mutation: name.to_owned(),
                    args: args.clone(),
                }
            }
            (state, name) if state.starts_with("mobile-review:") => {
                let session_id = state["mobile-review:".len()..].to_owned();
                if session_id.is_empty() || !self.inner.sessions.lock().contains_key(&session_id) {
                    return Err("unknown mobile review session".to_owned());
                }
                if !matches!(
                    name,
                    "open"
                        | "setDirectory"
                        | "setRemoteReview"
                        | "selectRemoteReview"
                        | "close"
                        | "reorder"
                        | "setPinned"
                        | "activate"
                        | "setMode"
                        | "setComparison"
                        | "bind"
                        | "unbind"
                        | "markReviewed"
                        | "setPosition"
                ) {
                    return Err(format!("unknown mobile review mutation {name}"));
                }
                Op::MobileReview {
                    session_id,
                    mutation: name.to_owned(),
                    args: args.clone(),
                }
            }
            _ => return Err(format!("unknown sync mutation {state}/{name}")),
        };
        if self.sync_already_seen(state, &id) {
            // Duplicate delivery/retry: already applied. Confirm it again so a
            // client that missed the original patch retires its outbox entry.
            self.sync_broadcast(state, vec![id]);
            return Ok(());
        }
        match op {
            Op::WorkspaceOrder { owner, order } => {
                let mut orders = self.inner.workspace_orders.lock();
                let current = orders.entry(owner.clone()).or_default();
                // Honor newly introduced rows at their submitted positions.
                // Hidden rows omitted by a folded client retain their relative order.
                let named: HashSet<&str> = order.iter().map(String::as_str).collect();
                let retained = current
                    .iter()
                    .filter(|key| !named.contains(key.as_str()))
                    .cloned();
                *current = order.iter().cloned().chain(retained).take(12_000).collect();
                if let Some(tx) = self.inner.store_tx.as_ref() {
                    let _ = tx.send(StoreWrite::UpdateWorkspaceOrder {
                        owner,
                        order: current.clone(),
                    });
                }
            }
            Op::Rename { session_id, title } => self.apply_rename(&session_id, title),
            Op::Reorder { order } => self.apply_reorder(&order),
            Op::Folders { mutation, args } => self.apply_folders(actor, &mutation, &args)?,
            Op::MobileReview {
                session_id,
                mutation,
                args,
            } => self.apply_mobile_review(&session_id, &mutation, &args)?,
        }
        // Op-log: the client INTENT behind a state change (who/what), paired with
        // the `op=change` line sync_emit writes for the authoritative version bump.
        tracing::info!(target: "cowboy::oplog", op = "mutation", %state, name, args = %args);
        self.sync_first_seen(state, &id);
        self.sync_broadcast(state, vec![id]);
        Ok(())
    }

    /// Resync `SyncPatch`es for the GLOBAL states on connect, `resync: true` so the
    /// client adopts them as ground truth. `title` and `order` are ALWAYS emitted
    /// (even if never mutated this lifetime), because a client may carry a locally
    /// persisted (`state/sync-idb`) cache of these states across reloads:
    /// without an unconditional authoritative seed, a stale cached override would
    /// overlay the fresh `Sessions` titles with nothing to correct it. Any other
    /// state mutated this lifetime is included too. Per-session queue states resync
    /// separately via [`Self::queue_resync`].
    #[must_use]
    pub fn sync_resync(&self) -> Vec<Outbound> {
        let snapshot: Vec<(String, u64, Vec<String>)> = {
            let reg = self.inner.sync.lock();
            let mut out: Vec<(String, u64, Vec<String>)> = reg
                .iter()
                .filter(|(s, _)| !s.starts_with("queue:") && !s.starts_with("mobile-review:"))
                .map(|(s, e)| (s.clone(), e.version, e.seen.iter().cloned().collect()))
                .collect();
            // Guarantee title + order + folders are present even when untouched
            // this lifetime.
            for state in ["title", "order", "folders", "workspace-order"] {
                if !out.iter().any(|(s, _, _)| s == state) {
                    let version = reg.get(state).map_or(0, |e| e.version);
                    out.push((state.to_owned(), version, Vec::new()));
                }
            }
            for session_id in self.inner.sessions.lock().keys() {
                let state = format!("mobile-review:{session_id}");
                if !out.iter().any(|(existing, _, _)| existing == &state) {
                    let version = reg.get(&state).map_or(0, |entry| entry.version);
                    out.push((state, version, Vec::new()));
                }
            }
            out
        };
        snapshot
            .into_iter()
            .map(|(state, version, confirmed)| Outbound::SyncPatch {
                value: self.sync_value(&state),
                state,
                version,
                confirmed,
                resync: true,
            })
            .collect()
    }

    /// The session's queued prompts and parked drafts, in order.
    /// Authorize and copy under the same lock so an ownership change cannot
    /// expose a different user's messages between admission and observation.
    #[must_use]
    pub fn queue_and_drafts(
        &self,
        session_id: &str,
        visible: impl FnOnce(&SessionMeta) -> bool,
    ) -> Option<(Vec<QueuedMessage>, Vec<QueuedMessage>)> {
        let sessions = self.inner.sessions.lock();
        let s = sessions.get(session_id)?;
        visible(&s.meta).then(|| (s.queue.clone(), s.drafts.clone()))
    }

    /// At most `max` hot-log events: the first ones after `after_seq` when it
    /// is given, otherwise the last ones before `before_seq` (or the tail).
    /// The flag reports whether the retained window's start was included.
    /// Visibility is checked atomically with the read.
    #[must_use]
    pub fn log_window(
        &self,
        session_id: &str,
        after_seq: Option<u64>,
        before_seq: Option<u64>,
        max: usize,
        visible: impl FnOnce(&SessionMeta) -> bool,
    ) -> Option<(Vec<Envelope>, bool)> {
        let sessions = self.inner.sessions.lock();
        let s = sessions.get(session_id)?;
        if !visible(&s.meta) {
            return None;
        }
        let end = before_seq.map_or(s.log.len(), |before| {
            s.log.partition_point(|e| e.seq < before)
        });
        let (start, end) = match after_seq {
            Some(after) => {
                let start = s.log.partition_point(|e| e.seq <= after).min(end);
                (start, end.min(start.saturating_add(max)))
            }
            None => (end.saturating_sub(max), end),
        };
        Some((s.log[start..end].to_vec(), s.reached_start && start == 0))
    }

    /// A resync `SyncPatch` for one session's queue+drafts state — `resync: true`
    /// so a (re)connecting client adopts it as ground truth. Confirmed = the cmids
    /// present in the value, so any optimistic add that landed while the client was
    /// away is dropped from its pending. `None` for an unknown session.
    #[must_use]
    pub fn queue_resync(&self, session_id: &str) -> Option<Outbound> {
        let (queue, drafts) = {
            let sessions = self.inner.sessions.lock();
            let s = sessions.get(session_id)?;
            (s.queue.clone(), s.drafts.clone())
        };
        let state = format!("queue:{session_id}");
        let version = self.inner.sync.lock().get(&state).map_or(0, |e| e.version);
        let confirmed = Self::cmids_of(&queue, &drafts);
        let value = serde_json::json!({ "queue": queue, "drafts": drafts });
        Some(Outbound::SyncPatch {
            state,
            version,
            value,
            confirmed,
            resync: true,
        })
    }
}
