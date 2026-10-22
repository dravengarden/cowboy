//! Bounded, resumable session-stop pass. It does not declare account revocation
//! complete: login cancellation, grant revocation and native logout are separate.
use super::{Result, RuntimeRouter, Step, Store};
use anyhow::Context as _;
use futures::{StreamExt as _, stream::FuturesUnordered};
use std::future::Future;

pub(super) struct SessionStop {
    pub session_id: String,
    /// Errors stay attached to their target; never abandon other Machines.
    pub outcome: Result<Step>,
}

pub(super) async fn stop_sessions(
    store: &Store,
    router: &RuntimeRouter,
    owner: &str,
    operation: &str,
) -> Result<Vec<SessionStop>> {
    let receipt = store
        .provider_account_receipt(owner, operation)
        .await?
        .context("revocation_operation_missing")?;
    let plan = receipt
        .revocation
        .as_ref()
        .context("revocation_plan_missing")?;
    let outcomes = bounded_pass(&plan.sessions, |target| async {
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            super::stop_planned_session(
                store,
                router,
                owner,
                operation,
                &receipt.account_id,
                target,
            ),
        )
        .await
        .unwrap_or(Ok(Step::Pending))
    })
    .await;
    Ok(plan
        .sessions
        .iter()
        .zip(outcomes)
        .map(|(target, outcome)| SessionStop {
            session_id: target.session_id.clone(),
            outcome,
        })
        .collect())
}

pub(super) async fn bounded_pass<'a, T, R, F, Fut>(targets: &'a [T], run: F) -> Vec<R>
where
    F: Fn(&'a T) -> Fut,
    Fut: Future<Output = R>,
{
    async fn indexed<R>(index: usize, future: impl Future<Output = R>) -> (usize, R) {
        (index, future.await)
    }
    let mut pending = FuturesUnordered::new();
    let mut remaining = targets.iter().enumerate();
    let mut results = Vec::with_capacity(targets.len());
    loop {
        while pending.len() < 4 {
            let Some((index, target)) = remaining.next() else {
                break;
            };
            pending.push(indexed(index, run(target)));
        }
        let Some(result) = pending.next().await else {
            break;
        };
        results.push(result);
    }
    results.sort_by_key(|(index, _)| *index);
    results.into_iter().map(|(_, result)| result).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn pass_bounds_concurrency_keeps_errors_and_restores_plan_order() {
        let active = AtomicUsize::new(0);
        let peak = AtomicUsize::new(0);
        let finished = AtomicUsize::new(0);
        let targets: Vec<_> = (0..13).collect();
        let results = bounded_pass(&targets, |&target| {
            let active = &active;
            let peak = &peak;
            let finished = &finished;
            async move {
                let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(count, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(13 - target)).await;
                active.fetch_sub(1, Ordering::SeqCst);
                finished.fetch_add(1, Ordering::SeqCst);
                if target == 2 {
                    Err("offline")
                } else {
                    Ok(target)
                }
            }
        })
        .await;
        assert_eq!(peak.load(Ordering::SeqCst), 4);
        assert_eq!(active.load(Ordering::SeqCst), 0);
        assert_eq!(finished.load(Ordering::SeqCst), targets.len());
        assert_eq!(results[2], Err("offline"));
        for (index, result) in results.into_iter().enumerate() {
            if index != 2 {
                assert_eq!(result, Ok(u64::try_from(index).unwrap()));
            }
        }
    }
}
