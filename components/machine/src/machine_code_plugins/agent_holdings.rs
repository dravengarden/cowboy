//! Machine-local automatic holdings outlive a Controller connection. Only
//! confirmed idle Opens are closed automatically, on their original route.
//! An uncertain Open/Close retains its reservation; absence is never success.
use super::*;
use std::time::Instant;

const IDLE: Duration = Duration::from_secs(300);
const PER_OWNER: usize = 4;
const PER_MACHINE: usize = 8;
const WORKTREES: usize = 2;

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
    confirmed: bool,
    releasing: bool,
}

impl Holding {
    pub(super) fn dispatching(&mut self, request: &Value) -> Result<()> {
        ensure!(
            request["worktree"].as_str() == Some(self.worktree.as_str()),
            "original automatic worktree spelling changed"
        );
        ensure!(
            !self.releasing,
            "original automatic buffer release remains unconfirmed"
        );
        self.releasing = request["type"] == "closeBuffer";
        Ok(())
    }
    pub(super) fn opening(worktree: &str, reclaimable: bool) -> Self {
        Self {
            worktree: worktree.to_owned(),
            // Old Controllers reuse deterministic owners after restart. Keep
            // their original cleanup behavior even during a rollback.
            idle_reclaimable: reclaimable,
            last_used: Instant::now(),
            confirmed: false,
            releasing: false,
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
        self.confirmed = false;
        Ok(())
    }

    pub(super) fn confirm(&mut self) {
        self.confirmed = true;
        self.last_used = Instant::now();
    }

    fn begin_release(&mut self) -> bool {
        if !self.idle_reclaimable
            || !self.confirmed
            || self.releasing
            || self.last_used.elapsed() < IDLE
        {
            return false;
        }
        self.releasing = true;
        true
    }
}

impl CodeRuntimeHost {
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
                host.release_idle_agents().await;
            }
        })
    }

    async fn release_idle_agents(&self) {
        if let Err(error) = self.reap_retired_processes().await {
            tracing::warn!(%error, "Code process retirement remains unconfirmed");
        }
        // Do not hold the registry while waiting on a native exchange. The
        // route lock serializes maintenance with both new Opens and UI work.
        let routes: Vec<_> = self.routes.lock().await.values().map(Arc::clone).collect();
        for route in routes {
            let Ok(mut route) = route.try_lock() else {
                continue;
            };
            if route.agent_buffers.is_empty() {
                continue;
            }
            let Some(CodeTarget::Installed(runtime)) = route.target.as_ref() else {
                continue;
            };
            // Leader loss is not evidence that descendants released resources.
            if !matches!(runtime.is_running(), Ok(true)) {
                continue;
            }
            let socket = runtime.socket.clone();
            let keys: Vec<_> = route.agent_buffers.keys().cloned().collect();
            for (path, lease) in keys {
                let Some(holding) = route.agent_buffers.get_mut(&(path.clone(), lease.clone()))
                else {
                    continue;
                };
                if !holding.begin_release() {
                    continue;
                }
                let worktree = holding.worktree.clone();
                let close = serde_json::json!({"type":"closeBuffer", "worktree":worktree, "path":path, "leaseId":lease});
                match exchange(&socket, &close).await {
                    Ok(response) => match route.observe(&close, &response) {
                        Ok(true) => {
                            *route = WorktreeRoute::default();
                            break;
                        }
                        Ok(false) => {}
                        Err(_) => tracing::warn!("automatic Code idle release remains unconfirmed"),
                    },
                    Err(_) => tracing::warn!("automatic Code idle release remains unconfirmed"),
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "agent_holdings_tests.rs"]
mod tests;
