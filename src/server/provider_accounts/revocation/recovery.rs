//! Restart-safe discovery; the operation journal remains the only authority.
use super::*;
use crate::store::provider_accounts::revocation::discovery::{Cursor, Page};
use std::future::Future;

pub(in crate::server) async fn run(state: Arc<super::super::super::AppState>) {
    let mut cursor = None;
    loop {
        // Sleep after each page rather than interval ticks: slow work must not
        // cause catch-up bursts. Re-read the live configuration every time.
        let delay = state
            .service_config
            .get(&crate::config::schema::ACCOUNT_REVOCATION_RETRY_INTERVAL);
        tokio::time::sleep(delay).await;
        let Some(store) = &state.store else { continue };
        match store.provider_account_revocations(cursor.as_ref()).await {
            Ok(page) => {
                let (next, failures) = process_page(page, |target| {
                    let state = &state;
                    async move {
                        reconcile(state, &target.owner, &target.operation)
                            .await
                            .map(|_| ())
                    }
                })
                .await;
                cursor = next;
                if failures > 0 {
                    tracing::warn!(
                        failures,
                        "account revocation recovery page has unconfirmed operations"
                    );
                }
            }
            Err(_) => {
                tracing::warn!("account revocation discovery failed; retaining recovery cursor")
            }
        }
    }
}

async fn process_page<E, F, Fut>(page: Page, retry: F) -> (Option<Cursor>, usize)
where
    F: Fn(Cursor) -> Fut,
    Fut: Future<Output = std::result::Result<(), E>>,
{
    let results = batch::bounded_pass(&page.operations, |target| retry(target.clone())).await;
    (
        page.next,
        results.iter().filter(|result| result.is_err()).count(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[tokio::test]
    async fn failed_operations_do_not_starve_the_page_or_reset_its_cursor() {
        let operations: Vec<_> = (0..9)
            .map(|index| Cursor {
                owner: "owner".into(),
                operation: index.to_string(),
            })
            .collect();
        let next = operations.last().cloned();
        let visited = Mutex::new(Vec::new());
        let (actual, failures) = process_page(
            Page {
                operations: operations.clone(),
                next: next.clone(),
            },
            |target| {
                let visited = &visited;
                async move {
                    visited.lock().unwrap().push(target.clone());
                    if target.operation == "0" {
                        Err(())
                    } else {
                        Ok(())
                    }
                }
            },
        )
        .await;
        assert_eq!(actual, next);
        assert_eq!(failures, 1);
        assert_eq!(visited.lock().unwrap().len(), operations.len());
        let (end, failures) = process_page(
            Page {
                operations: vec![],
                next: None,
            },
            |_| async {
                panic!("empty page must not dispatch");
                #[allow(unreachable_code)]
                Ok::<(), ()>(())
            },
        )
        .await;
        assert!(end.is_none());
        assert_eq!(failures, 0);
    }
}
