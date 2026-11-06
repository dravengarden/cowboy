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
    owned: BTreeSet<usize>,
    retired: BTreeMap<usize, Arc<Containment>>,
}

#[derive(Default)]
pub(crate) struct Retirement(parking_lot::Mutex<State>);

pub(crate) struct Reservation {
    pool: Arc<Retirement>,
    id: Option<usize>,
}

impl Retirement {
    pub(crate) fn reserve(self: &Arc<Self>) -> Result<Reservation> {
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
        state.owned.insert(id);
        Ok(Reservation {
            pool: Arc::clone(self),
            id: Some(id),
        })
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
        let mut command = tokio::process::Command::new("/cowboy-nonexistent-retirement-fixture");
        assert!(super::super::ProcessTree::spawn_in(&mut command, Some(&pool)).is_err());
        assert!(pool.0.lock().owned.is_empty());
    }

    #[tokio::test]
    #[ignore = "requires delegated writable cgroup v2"]
    async fn dropped_process_tree_stays_owned_until_original_group_is_removed() {
        let pool = Arc::new(Retirement::default());
        let mut command = tokio::process::Command::new("sh");
        command
            .args(["-c", "setsid sh -c 'echo ready; exec sleep 60' & wait"])
            .stdout(std::process::Stdio::piped())
            .kill_on_drop(true);
        let (mut child, tree) =
            super::super::ProcessTree::spawn_in(&mut command, Some(&pool)).unwrap();
        assert!(
            tree._containment.is_some(),
            "native acceptance requires containment"
        );
        let mut ready = [0; 6];
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            tokio::io::AsyncReadExt::read_exact(&mut child.stdout.take().unwrap(), &mut ready),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(&ready, b"ready\n");
        drop(tree);
        assert_eq!(
            pool.0.lock().owned.len(),
            1,
            "a stop request cannot free tracking"
        );
        assert_eq!(pool.0.lock().retired.len(), 1);
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let _ = pool.observe();
                if pool.0.lock().owned.is_empty() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("original group removal must confirm retirement");
        assert!(pool.0.lock().retired.is_empty());
        assert!(!child.wait().await.unwrap().success());
    }
}
