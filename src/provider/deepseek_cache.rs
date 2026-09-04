//! Cowboy-owned policy for process-local `DeepSeek` prompt-cache protection.

#![warn(clippy::pedantic)]

use cowboy_provider_sdk::ConfigurationBehavior;
use sha2::{Digest as _, Sha256};

#[cfg(feature = "full")]
pub const CONFIG_ID: &str = "deepseek_cache_protection";
pub const SESSION_POLICY_ENV: &str = "COWBOY_DEEPSEEK_CACHE_PROTECTION";

#[must_use]
#[cfg(feature = "full")]
pub fn minimum_hit_tokens() -> u64 {
    crate::plugin_runtime_args::deepseek_cache_protection().min_hit_tokens
}

#[must_use]
#[cfg(feature = "full")]
pub fn unavailable_message() -> String {
    format!(
        "{} is unavailable for this session",
        crate::plugin_runtime_args::deepseek_cache_protection().option_name
    )
}

#[must_use]
#[cfg(feature = "full")]
pub fn boolean_required_message() -> String {
    format!(
        "{} must be a boolean",
        crate::plugin_runtime_args::deepseek_cache_protection().option_name
    )
}
#[cfg(feature = "machine-host")]
pub const SESSION_HEADER: &str = "X-Cowboy-Session-Id";
#[cfg(feature = "machine-host")]
const GATEWAY_CACHE_PATH: &str = "/internal/cache-protection";
#[cfg(feature = "machine-host")]
const GATEWAY_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);
#[cfg(feature = "machine-host")]
const GATEWAY_STATUS_MAX_BYTES: usize = 4 * 1024;

#[must_use]
pub fn supported_behavior(behavior: &ConfigurationBehavior) -> bool {
    matches!(
        behavior,
        ConfigurationBehavior::AnthropicGatewayV1 | ConfigurationBehavior::OpenaiGatewayV1
    )
}

#[must_use]
#[cfg(feature = "full")]
pub fn selected(preferences: &serde_json::Value, behavior: &ConfigurationBehavior) -> Option<bool> {
    supported_behavior(behavior).then(|| {
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
pub fn gateway_origin(behavior: &ConfigurationBehavior) -> Option<&'static str> {
    crate::plugin_runtime_args::launch_plugin_ids()
        .into_iter()
        .find(|plugin_id| {
            crate::provider_behavior::legacy_behavior(plugin_id).configuration == *behavior
                && crate::plugin_runtime_args::loopback_origin_for(plugin_id).is_some()
        })
        .and_then(crate::plugin_runtime_args::loopback_origin_for)
}

/// Best-effort removal of a process-local replay snapshot. This request is
/// deliberately independent from the worker lifecycle: callers can spawn it
/// without delaying or interrupting a real agent turn.
#[cfg(feature = "machine-host")]
pub async fn revoke_local_snapshot(
    behavior: &ConfigurationBehavior,
    session_id: &str,
) -> anyhow::Result<()> {
    let origin = gateway_origin(behavior)
        .ok_or_else(|| anyhow::anyhow!("cache protection is unavailable for {behavior:?}"))?;
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
    behavior: &ConfigurationBehavior,
    session_id: &str,
) -> anyhow::Result<serde_json::Value> {
    let origin = gateway_origin(behavior)
        .ok_or_else(|| anyhow::anyhow!("cache protection is unavailable for {behavior:?}"))?;
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
pub fn config_option(behavior: &ConfigurationBehavior, enabled: bool) -> Option<serde_json::Value> {
    supported_behavior(behavior).then(|| {
        let protection = crate::plugin_runtime_args::deepseek_cache_protection();
        serde_json::json!({
            "id": CONFIG_ID,
            "name": protection.option_name,
            "description": protection.option_description,
            "category": "model_config",
            "type": "select",
            "currentValue": enabled,
            "options": [
                { "value": true, "name": protection.option_on },
                { "value": false, "name": protection.option_off }
            ]
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLAUDE: ConfigurationBehavior = ConfigurationBehavior::AnthropicGatewayV1;
    const CODEX: ConfigurationBehavior = ConfigurationBehavior::OpenaiGatewayV1;
    const PORTABLE: ConfigurationBehavior = ConfigurationBehavior::PortableV1;

    #[cfg(feature = "full")]
    #[test]
    fn deepseek_defaults_to_auto_and_preserves_explicit_off() {
        assert_eq!(selected(&serde_json::json!({}), &CODEX), Some(true));
        assert_eq!(
            selected(
                &serde_json::json!({"deepseek_cache_protection": false}),
                &CLAUDE
            ),
            Some(false)
        );
        assert_eq!(selected(&serde_json::json!({}), &PORTABLE), None);
        let option = config_option(&CODEX, true).expect("DeepSeek cache-protection option");
        assert_eq!(minimum_hit_tokens(), 64_000);
        assert_eq!(option["name"], "Cache protection");
        assert_eq!(option["options"][0]["name"], "Auto · recommended");
        assert_eq!(option["options"][1]["name"], "Off");
        assert!(
            option["description"]
                .as_str()
                .is_some_and(|description| description.contains("64K"))
        );
        assert_eq!(
            unavailable_message(),
            "Cache protection is unavailable for this session"
        );
        assert_eq!(
            boolean_required_message(),
            "Cache protection must be a boolean"
        );
    }

    #[cfg(feature = "machine-host")]
    #[test]
    fn cache_endpoint_and_opaque_identity_are_provider_scoped_and_content_free() {
        assert_eq!(
            gateway_origin(&CODEX),
            crate::plugin_runtime_args::loopback_origin_for("codex-deepseek")
        );
        assert_eq!(
            gateway_origin(&CLAUDE),
            crate::plugin_runtime_args::loopback_origin_for("claude-deepseek")
        );
        assert_eq!(gateway_origin(&PORTABLE), None);

        let opaque = opaque_session_id("sess-private-value");
        assert_eq!(opaque.len(), 64);
        assert!(opaque.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert!(!opaque.contains("sess-private-value"));
    }
}
