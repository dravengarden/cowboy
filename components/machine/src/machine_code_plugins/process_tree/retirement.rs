//! Reserve tracking before spawn, then retain the original group after its
//! last runtime owner leaves. Sending SIGKILL never removes resource evidence.
use crate::cgroup::containment::{Containment, ResourceSample};
use anyhow::{Context as _, Result, ensure};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

const LIMIT: usize = 4096;

#[derive(Default)]
struct State {
    next: usize,
    /// Maps each live or retiring reservation id to its optional
    /// `(plugin_id, generation_digest)` so retiring descendants keep their
    /// generation pinned until containment is confirmed empty.
    owned: BTreeMap<usize, Option<(String, String)>>,
    /// Conservative pins kept when the platform lacks cgroup containment and
    /// therefore cannot prove detached descendants exited.
    conservative: BTreeSet<(String, String)>,
    retired: BTreeMap<usize, Arc<Containment>>,
}

#[derive(Default)]
pub(crate) struct Retirement(parking_lot::Mutex<State>);

pub(crate) struct Reservation {
    pool: Arc<Retirement>,
    id: Option<usize>,
}

impl Retirement {
    #[cfg(test)]
    pub(crate) fn reserve(self: &Arc<Self>) -> Result<Reservation> {
        self.reserve_inner(None)
    }

    pub(crate) fn reserve_for(
        self: &Arc<Self>,
        plugin_id: String,
        generation_digest: String,
    ) -> Result<Reservation> {
        self.reserve_inner(Some((plugin_id, generation_digest)))
    }

    fn reserve_inner(
        self: &Arc<Self>,
        generation: Option<(String, String)>,
    ) -> Result<Reservation> {
        let mut state = self.0.lock();
        ensure!(
            state.owned.len() < LIMIT,
            "Code process retirement capacity exhausted"
        );
        let id = state
            .next
            .checked_add(1)
            .context("Code process retirement identity exhausted")?;
        state.next = id;
        state.owned.insert(id, generation);
        Ok(Reservation {
            pool: Arc::clone(self),
            id: Some(id),
        })
    }

    /// Generation digests of `plugin_id` still pinned by live runtimes,
    /// retiring process trees whose containment has not yet been confirmed
    /// empty, or conservative fallback pins on platforms without containment.
    pub(crate) fn pinned_generations(&self, plugin_id: &str) -> BTreeSet<String> {
        let state = self.0.lock();
        let mut pinned: BTreeSet<String> = state
            .owned
            .values()
            .filter_map(|entry| match entry {
                Some((pid, digest)) if pid == plugin_id => Some(digest.clone()),
                _ => None,
            })
            .collect();
        for (pid, digest) in &state.conservative {
            if pid == plugin_id {
                pinned.insert(digest.clone());
            }
        }
        pinned
    }

    /// Caller uses a blocking worker. The registry lock never covers kernel
    /// inspection, so interactive launches cannot queue behind a slow scan.
    pub(crate) fn observe(&self) -> Result<Vec<(usize, ResourceSample)>> {
        let groups: Vec<_> = self
            .0
            .lock()
            .retired
            .iter()
            .map(|(id, group)| (*id, Arc::clone(group)))
            .collect();
        let mut samples = Vec::new();
        let mut failure = None;
        for (id, group) in groups {
            match group.try_finish() {
                Ok(true) => {
                    let mut state = self.0.lock();
                    state.retired.remove(&id);
                    state.owned.remove(&id);
                }
                Ok(false) => match group.resources() {
                    Ok(sample) => samples.push((id, sample)),
                    Err(error) => {
                        failure = Some(error);
                    }
                },
                Err(error) => {
                    failure = Some(error);
                }
            }
        }
        if let Some(error) = failure {
            return Err(error).context("retiring Code process resources unavailable");
        }
        Ok(samples)
    }
}

impl Reservation {
    pub(crate) fn retire(mut self, group: Containment) {
        let id = self
            .id
            .take()
            .expect("retirement reservation consumed once");
        self.pool.0.lock().retired.insert(id, Arc::new(group));
    }

    /// Keep this generation conservatively pinned when the platform cannot
    /// prove detached descendants exited (no cgroup containment).
    pub(crate) fn retain_conservatively(mut self) {
        if let Some(id) = self.id.take() {
            let mut state = self.pool.0.lock();
            if let Some(Some(generation)) = state.owned.remove(&id) {
                state.conservative.insert(generation);
            }
        }
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if let Some(id) = self.id {
            self.pool.0.lock().owned.remove(&id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracking_is_reserved_before_effects_and_never_reuses_an_identity() {
        let pool = Arc::new(Retirement::default());
        let mut slots: Vec<_> = (0..LIMIT).map(|_| pool.reserve().unwrap()).collect();
        assert!(pool.reserve().is_err());
        let last = slots.pop().unwrap();
        let id = last.id.unwrap();
        drop(last);
        let next = pool.reserve().unwrap();
        assert!(next.id.unwrap() > id);
        assert!(pool.reserve().is_err());
        drop(next);
        drop(slots);
        assert!(pool.0.lock().owned.is_empty());
    }

    #[tokio::test]
    async fn failed_spawn_does_not_consume_retirement_capacity() {
        let pool = Arc::new(Retirement::default());
        let mut command = tokio::process::Command::new("/nonexistent/cowboy-code-binary");
        assert!(
            super::super::ProcessTree::spawn_in(&mut command, Some((&pool, "fixture", "gen")))
                .is_err()
        );
        assert!(pool.0.lock().owned.is_empty());
    }

    #[tokio::test]
    async fn exhausted_retirement_capacity_refuses_spawn() {
        let pool = Arc::new(Retirement::default());
        let _held: Vec<_> = (0..LIMIT).map(|_| pool.reserve().unwrap()).collect();
        let mut command = tokio::process::Command::new("/bin/sh");
        command.arg("-c").arg("exit 0");
        assert!(
            super::super::ProcessTree::spawn_in(&mut command, Some((&pool, "fixture", "gen")))
                .is_err()
        );
    }

    #[tokio::test]
    async fn fallback_without_containment_retains_conservative_generation_pin() {
        let pool = Arc::new(Retirement::default());
        let reservation = pool
            .reserve_for("fixture-code".into(), "fallback-gen".into())
            .unwrap();
        assert!(
            pool.pinned_generations("fixture-code")
                .contains("fallback-gen")
        );
        reservation.retain_conservatively();
        assert!(pool.0.lock().owned.is_empty());
        assert!(
            pool.pinned_generations("fixture-code")
                .contains("fallback-gen")
        );
    }

    #[tokio::test]
    #[ignore = "requires delegated writable cgroup v2"]
    async fn retiring_descendants_keep_reservation_until_empty() {
        let pool = Arc::new(Retirement::default());
        let mut command = tokio::process::Command::new("/bin/sh");
        command.arg("-c").arg("sleep 30 & exit 0");
        let (mut child, mut tree) =
            super::super::ProcessTree::spawn_in(&mut command, Some((&pool, "fixture", "gen")))
                .unwrap();
        let _ = child.wait().await;
        let _ = tree.request_stop();
        drop(tree);
        assert!(!pool.0.lock().owned.is_empty());
        for _ in 0..50 {
            let _ = pool.observe();
            if pool.0.lock().owned.is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(pool.0.lock().owned.is_empty());
    }
}
