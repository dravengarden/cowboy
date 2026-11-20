//! The scheduled provider reset timer.

use super::*;

/// Fire scheduled provider resets when they come due, paced by usage limits,
/// until shutdown. Reset credits belong to provider accounts, not sessions.
pub(super) fn spawn_provider_reset_timer(
    usage: UsageService,
    store: Option<Store>,
    mut shutdown: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            for provider in &usage.reset_provider_ids() {
                let action = match store.as_ref() {
                    Some(store) => store.load_provider_reset(provider).await.ok().flatten(),
                    None => None,
                };
                if let Some(action) = action.filter(|item| item.next_attempt_at_ms <= now_ms()) {
                    let key = action.idempotency_key;
                    // Plugins that cannot accept an idempotency key claim
                    // the one-shot timer before any provider call so a
                    // crash or ambiguous response cannot consume a later
                    // reset on an automatic retry.
                    // Pacing may delay a due reset; reserve it before an
                    // irreversible claim so pacing never discards it.
                    let reservation = match usage.reserve_reset(provider).await {
                        Ok(reservation) => reservation,
                        Err(error) => {
                            tracing::debug!(%error, %provider, "scheduled provider reset waits for usage pacing");
                            continue;
                        }
                    };
                    let claim_before = usage.reset_claims_before_attempt(provider);
                    if claim_before {
                        let Some(store) = store.as_ref() else {
                            continue;
                        };
                        match store.claim_provider_reset(provider, &key).await {
                            Ok(true) => {}
                            Ok(false) => continue,
                            Err(error) => {
                                tracing::error!(%error, %provider, "claiming scheduled provider reset");
                                continue;
                            }
                        }
                        usage.set_reset_schedule(provider, None).await;
                    }
                    if let Some(store) = store.as_ref() {
                        let _ = store
                            .append_provider_action_log(
                                provider,
                                "scheduled",
                                "started",
                                "preflight",
                                "Scheduled reset attempt started",
                                None,
                                Some(&key),
                                now_ms(),
                            )
                            .await;
                    }
                    match usage
                        .consume_reserved_reset(reservation, provider, &key)
                        .await
                    {
                        Ok(result) => {
                            tracing::info!(%provider, outcome = %result.outcome, "scheduled provider reset finished");
                            if let Some(store) = store.as_ref() {
                                let _ = store
                                    .append_provider_action_log(
                                        provider,
                                        "scheduled",
                                        "succeeded",
                                        "provider_response",
                                        &result.outcome,
                                        result.credit_id.as_deref(),
                                        Some(&key),
                                        now_ms(),
                                    )
                                    .await;
                            }
                            if !claim_before && let Some(store) = store.as_ref() {
                                match store.claim_provider_reset(provider, &key).await {
                                    Ok(true) => usage.set_reset_schedule(provider, None).await,
                                    Ok(false) => {}
                                    Err(error) => {
                                        tracing::warn!(%error, %provider, "clearing scheduled provider reset");
                                    }
                                }
                            }
                        }
                        Err(error) => {
                            if claim_before {
                                let (status, phase) = if error.call_may_have_reached_provider {
                                    ("unknown", "consume")
                                } else {
                                    ("failed", "preflight")
                                };
                                tracing::error!(%error, %provider, "scheduled one-shot provider reset stopped without retry");
                                if let Some(store) = store.as_ref() {
                                    let _ = store
                                        .append_provider_action_log(
                                            provider,
                                            "scheduled",
                                            status,
                                            phase,
                                            &error.to_string(),
                                            error.credit_id.as_deref(),
                                            Some(&key),
                                            now_ms(),
                                        )
                                        .await;
                                }
                                continue;
                            }
                            if scheduled_reset_failure_policy(
                                error.call_may_have_reached_provider,
                                action.attempt_count,
                            ) == ScheduledResetFailurePolicy::StopUnknown
                            {
                                tracing::error!(%error, %provider, "scheduled provider reset outcome unknown; automatic retry disabled");
                                if let Some(store) = store.as_ref() {
                                    let _ = store
                                        .append_provider_action_log(
                                            provider,
                                            "scheduled",
                                            "unknown",
                                            "consume",
                                            &error.to_string(),
                                            error.credit_id.as_deref(),
                                            Some(&key),
                                            now_ms(),
                                        )
                                        .await;
                                    match store.claim_provider_reset(provider, &key).await {
                                        Ok(true) => {
                                            usage.set_reset_schedule(provider, None).await;
                                        }
                                        Ok(false) => {}
                                        Err(clear_error) => {
                                            tracing::warn!(%clear_error, %provider, "clearing scheduled provider reset");
                                        }
                                    }
                                }
                            } else if scheduled_reset_failure_policy(false, action.attempt_count)
                                == ScheduledResetFailurePolicy::RetryPreflight
                            {
                                tracing::warn!(%error, %provider, "scheduled provider reset preflight failed; retrying safely");
                                if let Some(store) = store.as_ref() {
                                    let _ = store
                                        .append_provider_action_log(
                                            provider,
                                            "scheduled",
                                            "retrying",
                                            "preflight",
                                            &error.to_string(),
                                            error.credit_id.as_deref(),
                                            Some(&key),
                                            now_ms(),
                                        )
                                        .await;
                                    let _ = store
                                        .defer_provider_reset(
                                            provider,
                                            &key,
                                            now_ms().saturating_add(60_000),
                                        )
                                        .await;
                                }
                            } else {
                                tracing::error!(%error, %provider, "scheduled provider reset preflight retry limit reached");
                                if let Some(store) = store.as_ref() {
                                    let _ = store
                                        .append_provider_action_log(
                                            provider,
                                            "scheduled",
                                            "failed",
                                            "preflight",
                                            &error.to_string(),
                                            error.credit_id.as_deref(),
                                            Some(&key),
                                            now_ms(),
                                        )
                                        .await;
                                    match store.claim_provider_reset(provider, &key).await {
                                        Ok(true) => {
                                            usage.set_reset_schedule(provider, None).await;
                                        }
                                        Ok(false) => {}
                                        Err(clear_error) => {
                                            tracing::warn!(%clear_error, %provider, "clearing scheduled provider reset");
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(15)) => {}
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() { break; }
                }
            }
        }
    })
}
