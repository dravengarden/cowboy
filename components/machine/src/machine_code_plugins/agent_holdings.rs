//! Machine-local automatic holdings outlive a Controller connection. Worktree
//! confirmation permits original prepared release after a lost Open reply.
//! An uncertain Open/Close retains its reservation; absence is never success.
use super::*;
use std::time::Instant;

const IDLE: Duration = Duration::from_secs(300);
const PER_OWNER: usize = 4;
const PER_MACHINE: usize = 8;
const WORKTREES: usize = 2;

/// Cheap negative admission evidence, never a replacement for the native
/// resolver. Reject directories, missing inputs and symlinks at every level;
/// do not open special files or follow a projected checkout outside the root.
pub(super) fn regular_file(root: &Path, relative: &str) -> bool {
    use std::path::Component;
    let mut path = root.to_owned();
    let mut components = Path::new(relative).components().peekable();
    if components.peek().is_none() {
        return false;
    }
    while let Some(component) = components.next() {
        let Component::Normal(name) = component else {
            return false;
        };
        path.push(name);
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            return false;
        };
        if components.peek().is_some() {
            if !metadata.is_dir() {
                return false;
            }
        } else {
            return metadata.is_file();
        }
    }
    false
}

// Both protocol generations count against the same session share on rollback.
pub(super) fn owner(lease: &str, version_two: bool) -> Result<String> {
    use sha2::{Digest as _, Sha256};
    let Some(raw) = lease.strip_prefix("cowboy-agent-") else {
        ensure!(!version_two, "invalid automatic owner identity");
        return Ok(lease.to_owned());
    };
    if !version_two {
        return Ok(format!("{:x}", Sha256::digest(raw.as_bytes())));
    }
    let (session, nonce) = raw
        .rsplit_once('-')
        .context("invalid automatic owner identity")?;
    ensure!(
        session.len() == 64
            && nonce.len() == 32
            && session
                .bytes()
                .chain(nonce.bytes())
                .all(|c| c.is_ascii_hexdigit()),
        "invalid automatic owner identity"
    );
    Ok(session.to_ascii_lowercase())
}

pub(super) struct Holding {
    worktree: String,
    idle_reclaimable: bool,
    last_used: Instant,
    worktree_confirmed: bool,
    confirmed: bool,
    pub(super) releasing: bool,
    pub(super) release: Option<super::agent_release::Attempt>,
}

impl Holding {
    pub(super) fn dispatching(&mut self, request: &Value) -> Result<()> {
        self.check_worktree(request)?;
        if request["type"] == "closeBuffer" && self.idle_reclaimable {
            ensure!(
                self.worktree_confirmed,
                "original automatic worktree remains unconfirmed"
            );
        }
        ensure!(
            !self.releasing,
            "original automatic buffer release remains unconfirmed"
        );
        self.releasing = request["type"] == "closeBuffer";
        Ok(())
    }

    pub(super) fn check_worktree(&self, request: &Value) -> Result<()> {
        ensure!(
            request["worktree"].as_str() == Some(self.worktree.as_str()),
            "original automatic worktree spelling changed"
        );
        Ok(())
    }
    pub(super) fn opening(worktree: &str, reclaimable: bool) -> Self {
        Self {
            worktree: worktree.to_owned(),
            // Old Controllers reuse deterministic owners after restart. Keep
            // their original cleanup behavior even during a rollback.
            idle_reclaimable: reclaimable,
            last_used: Instant::now(),
            worktree_confirmed: false,
            confirmed: false,
            releasing: false,
            release: None,
        }
    }

    pub(super) fn refresh(&mut self, worktree: &str) -> Result<()> {
        ensure!(
            self.worktree == worktree,
            "original automatic worktree spelling changed"
        );
        ensure!(
            !self.releasing,
            "original automatic buffer release remains unconfirmed"
        );
        self.last_used = Instant::now();
        self.worktree_confirmed = false;
        self.confirmed = false;
        Ok(())
    }

    pub(super) fn confirm_worktree(&mut self) {
        self.worktree_confirmed = true;
    }

    pub(super) fn confirm(&mut self) {
        self.confirm_worktree();
        self.confirmed = true;
        self.last_used = Instant::now();
    }

    pub(super) fn recoverable_release(&self) -> Result<bool> {
        if self.idle_reclaimable {
            ensure!(
                self.worktree_confirmed,
                "original automatic worktree remains unconfirmed"
            );
        }
        Ok(self.idle_reclaimable)
    }

    fn begin_release(&mut self) -> bool {
        // Old Plugins have no recoverable prepared reference. Keep their
        // one-use fallback limited to positively confirmed Opens.
        if !self.confirmed || !self.idle_due() {
            return false;
        }
        self.releasing = true;
        true
    }

    fn idle_due(&self) -> bool {
        self.idle_reclaimable
            && self.worktree_confirmed
            && !self.releasing
            && self.last_used.elapsed() >= IDLE
    }
}

impl CodeRuntimeHost {
    /// The signed native fixture intentionally hides writable host cgroups.
    /// It accepts no real resource admission; exercise only the post-admission
    /// lifecycle on the already installed/opened disposable route.
    #[cfg(all(test, target_os = "linux"))]
    pub(crate) async fn fixture_acquire_admitted(&self, plugin: &str, payload: &Value) -> Value {
        let worktree = request_worktree(payload).unwrap().unwrap();
        let route = self.routes.lock().await[&(plugin.to_owned(), worktree)].clone();
        let mut route = route.lock().await;
        let socket = route.target.as_ref().unwrap().socket().to_owned();
        self.acquire_admitted(&mut route, &socket, payload)
            .await
            .unwrap()
    }

    /// Renew only a confirmed owner on its original live route. In particular,
    /// an idle-reclaimed owner can never be recreated by a delayed refresh.
    pub(super) async fn refresh_agent(
        &self,
        plugin: &str,
        worktree: &Path,
        payload: &Value,
    ) -> Result<Value> {
        let key = WorktreeRoute::buffer_key(payload)?;
        owner(&key.1, true)?;
        let route = self
            .routes
            .lock()
            .await
            .get(&(plugin.to_owned(), worktree.to_owned()))
            .cloned();
        let Some(route) = route else {
            return Ok(resource_budget::unavailable());
        };
        let mut route = route.lock().await;
        if !matches!(route.target.as_ref(), Some(CodeTarget::Installed(runtime)) if matches!(runtime.is_running(), Ok(true)))
            || !route.buffers.contains(&key)
        {
            return Ok(resource_budget::unavailable());
        }
        let Some(held) = route.agent_buffers.get_mut(&key) else {
            return Ok(resource_budget::unavailable());
        };
        held.check_worktree(payload)?;
        if held.releasing {
            return Ok(resource_budget::unavailable());
        }
        // Keep the Controller's idle cleanup observation for an uncertain
        // Open. Unlike an absent owner, it may still need its original Close.
        ensure!(
            held.idle_reclaimable && held.confirmed,
            "original automatic Open remains unconfirmed"
        );
        held.last_used = Instant::now();
        Ok(serde_json::json!({"type":"agentBufferRefreshed", "api_version":1, "leaseId":key.1}))
    }

    /// Called under agent_admission, so two automatic Opens cannot both use
    /// the final slot. Busy interactive routes make admission unavailable.
    pub(super) async fn agent_holding_capacity(
        &self,
        plugin: &str,
        worktree: &Path,
        key: &(String, String),
        version_two: bool,
    ) -> Result<bool> {
        let session_owner = owner(&key.1, version_two)?;
        let routes = self.routes.lock().await;
        let mut total = 0;
        let mut owner_count = 0;
        let mut trees = 0;
        let mut existing_tree = false;
        let mut existing = false;
        for ((id, path), route) in routes.iter() {
            let route = route.try_lock().context("Code worktree is busy")?;
            total += route.agent_buffers.len();
            for ((_, lease), held) in &route.agent_buffers {
                owner_count += usize::from(owner(lease, held.idle_reclaimable)? == session_owner);
            }
            if !route.agent_buffers.is_empty() {
                trees += 1;
                existing_tree |= id == plugin && path == worktree;
            }
            if id == plugin
                && path == worktree
                && let Some(holding) = route.agent_buffers.get(key)
            {
                ensure!(
                    !holding.releasing,
                    "original automatic buffer release remains unconfirmed"
                );
                existing = true;
            }
        }
        Ok(existing
            || (total < PER_MACHINE
                && owner_count < PER_OWNER
                && (existing_tree || trees < WORKTREES)))
    }

    pub fn start_agent_maintenance(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let host = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let Some(host) = host.upgrade() else {
                    break;
                };
                host.buffer_sync.observe_pending().await;
                host.buffer_navigation.observe_pending().await;
                host.agent_retirements.observe().await;
                host.release_idle_agents().await;
            }
        })
    }

    pub(super) async fn release_idle_agents(&self) {
        if let Err(error) = self.reap_retired_processes().await {
            tracing::warn!(%error, "Code process retirement remains unconfirmed");
        }
        let routes: Vec<_> = self.routes.lock().await.values().map(Arc::clone).collect();
        let mut candidates = Vec::new();
        for route in routes {
            if let Ok(held) = route.try_lock() {
                for (key, holding) in &held.agent_buffers {
                    if holding.release.is_some() || holding.idle_due() {
                        candidates.push((Arc::clone(&route), key.clone()));
                    }
                }
            }
        }
        if candidates.is_empty() {
            return;
        }
        let start = self
            .agent_release_cursor
            .fetch_add(4, std::sync::atomic::Ordering::Relaxed)
            % candidates.len();
        let mut attempted = 0;
        for offset in 0..candidates.len() {
            if attempted == 4 {
                break;
            }
            let (route, (path, lease)) = &candidates[(start + offset) % candidates.len()];
            let Ok(mut route) = route.try_lock() else {
                continue;
            };
            let Some(holding) = route.agent_buffers.get(&(path.clone(), lease.clone())) else {
                continue;
            };
            if holding.release.is_none() && !holding.idle_due() {
                continue;
            }
            let worktree = holding.worktree.clone();
            let Some(CodeTarget::Installed(runtime)) = route.target.as_ref() else {
                continue;
            };
            // Leader loss never proves descendant cleanup or a released owner.
            if !matches!(runtime.is_running(), Ok(true)) {
                continue;
            }
            let runtime = Arc::clone(runtime);
            attempted += 1;
            let close = serde_json::json!({"type":"closeBuffer", "worktree":worktree, "path":path, "leaseId":lease});
            match self.release_agent(&mut route, &runtime, &close).await {
                Ok(Some(_)) => continue,
                Err(_) => {
                    tracing::warn!("automatic Code original release remains unconfirmed");
                    continue;
                }
                Ok(None) => {}
            }
            if !route
                .agent_buffers
                .get_mut(&(path.clone(), lease.clone()))
                .is_some_and(Holding::begin_release)
            {
                continue;
            }
            // Older Plugins retain their existing one-use Close deadline.
            match exchange(&runtime.socket, &close).await {
                Ok(response) => match route.observe(&close, &response) {
                    Ok(true) => *route = WorktreeRoute::default(),
                    Ok(false) => {}
                    Err(_) => tracing::warn!("automatic Code idle release remains unconfirmed"),
                },
                Err(_) => tracing::warn!("automatic Code idle release remains unconfirmed"),
            }
        }
    }
}

#[cfg(test)]
#[path = "agent_holdings_tests.rs"]
mod tests;
