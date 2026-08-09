//! Cowboy-owned policy for process-local `DeepSeek` prompt-cache protection.

#![warn(clippy::pedantic)]

use sha2::{Digest as _, Sha256};

#[cfg(feature = "full")]
pub const CONFIG_ID: &str = "deepseek_cache_protection";
pub const SESSION_POLICY_ENV: &str = "COWBOY_DEEPSEEK_CACHE_PROTECTION";
#[cfg(feature = "full")]
pub const MINIMUM_HIT_TOKENS: u64 = 64_000;
#[cfg(feature = "machine-host")]
pub const SESSION_HEADER: &str = "X-Cowboy-Session-Id";
#[cfg(feature = "machine-host")]
const GATEWAY_CACHE_PATH: &str = "/internal/cache-protection";
#[cfg(feature = "machine-host")]
const GATEWAY_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);
#[cfg(feature = "machine-host")]
const GATEWAY_STATUS_MAX_BYTES: usize = 4 * 1024;

#[must_use]
pub fn supported_provider(provider: &str) -> bool {
    matches!(provider, "claude-deepseek" | "codex-deepseek")
}

#[must_use]
#[cfg(feature = "full")]
pub fn selected(preferences: &serde_json::Value, provider: &str) -> Option<bool> {
    supported_provider(provider).then(|| {
        preferences
            .get(CONFIG_ID)
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(true)
    })
}

#[must_use]
pub fn opaque_session_id(session_id: &str) -> String {
    format!("{:x}", Sha256::digest(session_id.as_bytes()))
}

#[must_use]
#[cfg(feature = "machine-host")]
pub fn gateway_origin(provider: &str) -> Option<&'static str> {
    match provider {
        "codex-deepseek" => Some("http://127.0.0.1:61137"),
        "claude-deepseek" => Some("http://127.0.0.1:61138"),
        _ => None,
    }
}

/// Best-effort removal of a process-local replay snapshot. This request is
/// deliberately independent from the worker lifecycle: callers can spawn it
/// without delaying or interrupting a real agent turn.
#[cfg(feature = "machine-host")]
pub async fn revoke_local_snapshot(provider: &str, session_id: &str) -> anyhow::Result<()> {
    let origin = gateway_origin(provider)
        .ok_or_else(|| anyhow::anyhow!("cache protection is unavailable for {provider}"))?;
    let client = reqwest::Client::builder()
        .timeout(GATEWAY_REQUEST_TIMEOUT)
        .build()?;
    client
        .delete(format!("{origin}{GATEWAY_CACHE_PATH}"))
        .header(SESSION_HEADER, opaque_session_id(session_id))
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

/// Read one content-free cache-protection status from the provider gateway on
/// the Machine that owns the session. The response is bounded and validated
/// before it crosses the Machine/controller adapter boundary.
#[cfg(feature = "machine-host")]
pub async fn local_snapshot_status(
    provider: &str,
    session_id: &str,
) -> anyhow::Result<serde_json::Value> {
    let origin = gateway_origin(provider)
        .ok_or_else(|| anyhow::anyhow!("cache protection is unavailable for {provider}"))?;
    let client = reqwest::Client::builder()
        .timeout(GATEWAY_REQUEST_TIMEOUT)
        .build()?;
    let response = client
        .get(format!("{origin}{GATEWAY_CACHE_PATH}"))
        .header(SESSION_HEADER, opaque_session_id(session_id))
        .send()
        .await?
        .error_for_status()?;
    let body = response.bytes().await?;
    if body.len() > GATEWAY_STATUS_MAX_BYTES {
        anyhow::bail!("cache-protection status exceeded the bounded response size");
    }
    let value: serde_json::Value = serde_json::from_slice(&body)?;
    let state = value.get("state").and_then(serde_json::Value::as_str);
    let algorithm = value.get("algorithm").and_then(serde_json::Value::as_str);
    if !matches!(state, Some("inactive" | "protected")) || algorithm != Some("adaptive-replay-v1") {
        anyhow::bail!("gateway returned an invalid cache-protection status");
    }
    Ok(value)
}

#[must_use]
#[cfg(feature = "full")]
pub fn config_option(provider: &str, enabled: bool) -> Option<serde_json::Value> {
    supported_provider(provider).then(|| {
        serde_json::json!({
            "id": CONFIG_ID,
            "name": "Cache protection",
            "description": "Automatically protects DeepSeek prompt caches after at least 64K verified hit tokens. Keepalives run only while idle, are preempted by real requests, and changing this setting restarts only this idle session.",
            "category": "model_config",
            "type": "select",
            "currentValue": enabled,
            "options": [
                { "value": true, "name": "Auto · recommended" },
                { "value": false, "name": "Off" }
            ]
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "full")]
    #[test]
    fn deepseek_defaults_to_auto_and_preserves_explicit_off() {
        assert_eq!(
            selected(&serde_json::json!({}), "codex-deepseek"),
            Some(true)
        );
        assert_eq!(
            selected(
                &serde_json::json!({"deepseek_cache_protection": false}),
                "claude-deepseek"
            ),
            Some(false)
        );
        assert_eq!(selected(&serde_json::json!({}), "codex"), None);
    }

    #[cfg(feature = "machine-host")]
    #[test]
    fn cache_endpoint_and_opaque_identity_are_provider_scoped_and_content_free() {
        assert_eq!(
            gateway_origin("codex-deepseek"),
            Some("http://127.0.0.1:61137")
        );
        assert_eq!(
            gateway_origin("claude-deepseek"),
            Some("http://127.0.0.1:61138")
        );
        assert_eq!(gateway_origin("codex"), None);

        let opaque = opaque_session_id("sess-private-value");
        assert_eq!(opaque.len(), 64);
        assert!(opaque.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(!opaque.contains("sess-private-value"));
    }
}
