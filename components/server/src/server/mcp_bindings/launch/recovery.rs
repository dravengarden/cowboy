use super::*;

impl Launches {
    pub(super) async fn recover_next(
        &self,
        state: &Context<'_>,
        cursor: &mut String,
    ) -> Result<bool> {
        let mut sessions = state.hub.session_list();
        sessions.sort_by(|a, b| a.id.cmp(&b.id));
        let start = sessions.partition_point(|meta| meta.id <= *cursor);
        for meta in sessions[start..].iter().chain(&sessions[..start]) {
            let Some(stored) = Stored::load(state.hub, &meta.id) else {
                continue;
            };
            if stored.bindings.is_empty() || self.resolve(state, meta).is_ok() {
                continue;
            }
            // Advance even on failure so an unavailable Machine cannot starve others.
            *cursor = meta.id.clone();
            return self
                .prepare_context(state, &meta.id, &stored.epoch)
                .await
                .map(|()| true);
        }
        Ok(false)
    }
}

pub(super) fn start(state: &Arc<AppState>) {
    let weak = Arc::downgrade(state);
    tokio::spawn(async move {
        let mut cursor = String::new();
        let mut failures = HashMap::new();
        let mut tick = tokio::time::interval(Duration::from_secs(5));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let Some(state) = weak.upgrade() else { break };
            if *state.shutdown.borrow() {
                break;
            }
            let Ok(_permit) = state
                .extensions
                .bindings
                .capacity
                .clone()
                .try_acquire_owned()
            else {
                continue;
            };
            // After a restart every surviving MCP worker waits on its owner:
            // continue while sessions recover; a failure waits for the next tick.
            for _ in 0..64 {
                let recovered = state
                    .extensions
                    .launches
                    .recover_next(&Context::from(state.as_ref()), &mut cursor)
                    .await;
                let recovered = match recovered {
                    Ok(recovered) => {
                        failures.remove(&cursor);
                        recovered
                    }
                    // The cursor names the session that just failed.
                    Err(error) => retry_advanced(&state, &cursor, error, &mut failures).await,
                };
                if !recovered || *state.shutdown.borrow() {
                    break;
                }
            }
        }
    });
}

/// A session whose Plugin was released again since its policy was pinned can
/// never prepare the superseded generation: advance it and prepare once more.
/// Other failures are logged once per distinct reason, then retried quietly.
async fn retry_advanced(
    state: &AppState,
    session: &str,
    error: anyhow::Error,
    failures: &mut HashMap<String, String>,
) -> bool {
    let error = match super::super::generations::advance(state, session).await {
        Ok(Some(epoch)) => match state
            .extensions
            .launches
            .prepare(state, session, &epoch)
            .await
        {
            Ok(()) => {
                failures.remove(session);
                tracing::info!(session = %session, "MCP launch owner recovered on the active Plugin generation");
                return true;
            }
            Err(error) => format!("{error:#}"),
        },
        Ok(None) => format!("{error:#}"),
        Err(reason) => format!("{error:#}; advancing the Plugin generation failed: {reason}"),
    };
    failures.retain(|id, _| state.hub.session_info(id).is_some());
    if failures.get(session) != Some(&error) {
        tracing::warn!(session = %session, error = %error, "MCP launch owner recovery failed");
        failures.insert(session.to_owned(), error);
    }
    false
}
