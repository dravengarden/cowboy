use super::{LaunchSpec, Producer, WorkerArgs};
use crate::provider::CODEX_REQUEST_OBSERVATION_ENV;
use std::path::Path;
use tokio::net::TcpListener;

pub(super) const CODEX_HEADER: &str = "x-cowboy-request-observation";

fn effective_env(spec: &LaunchSpec, name: &str) -> Option<String> {
    spec.env.get(name).cloned().or_else(|| {
        (!spec.removes_inherited_env(name))
            .then(|| std::env::var(name).ok())
            .flatten()
    })
}

fn owns_telemetry_environment(spec: &LaunchSpec) -> bool {
    let already_set = |name: &str| {
        name.starts_with("OTEL_")
            || matches!(
                name,
                "BETA_TRACING_ENDPOINT"
                    | "ENABLE_BETA_TRACING_DETAILED"
                    | "CLAUDE_CODE_ENABLE_TELEMETRY"
            )
    };
    spec.env.keys().any(|name| already_set(name))
        || std::env::vars_os().any(|(name, _)| {
            name.to_str()
                .is_some_and(|name| already_set(name) && !spec.removes_inherited_env(name))
        })
}

pub(super) async fn open_listener(
    spec: &LaunchSpec,
    args: &WorkerArgs,
    producer: Producer,
    codex_opt_in: bool,
) -> Option<TcpListener> {
    if args.provider_generation_digest.is_empty()
        || spec.account_sandbox.is_some()
        || owns_telemetry_environment(spec)
        || [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ]
        .iter()
        .any(|name| effective_env(spec, name).is_some_and(|value| !value.is_empty()))
    {
        return None;
    }
    if producer == Producer::Codex && !codex_allowed(spec, args, codex_opt_in) {
        return None;
    }
    TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .ok()
}

fn codex_allowed(spec: &LaunchSpec, args: &WorkerArgs, signed_opt_in: bool) -> bool {
    signed_opt_in
        && args
            .execution_binding
            .as_ref()
            .and_then(crate::execution_environment::ExecutionBinding::managed)
            .is_none()
        && spec
            .env
            .get("CODEX_PATH")
            .is_some_and(|path| Path::new(path).is_absolute())
}

pub(super) fn configure(
    spec: &mut LaunchSpec,
    producer: Producer,
    endpoint: &str,
    key: String,
) -> (String, Option<String>) {
    if producer == Producer::Codex {
        // The signed adapter owns effective native config, including project
        // and managed layers. The worker hands it a private destination and
        // never sets native exporter flags or the standard OTEL environment.
        let binding = serde_json::json!({
            "schema": 1,
            "logs_endpoint": format!("{endpoint}/v1/logs"),
            "header_name": CODEX_HEADER,
            "header_value": key,
        });
        spec.env
            .insert(CODEX_REQUEST_OBSERVATION_ENV.into(), binding.to_string());
        return ("/v1/{signal}".into(), Some(key));
    }

    let base = format!("{endpoint}/{key}");
    let logs = format!("{base}/v1/logs");
    let traces = format!("{base}/v1/traces");
    // The beta tracer supplies attempt starts. Both standard signal exporters
    // also get explicit destinations on the same private listener.
    for (name, value) in [
        ("CLAUDE_CODE_ENABLE_TELEMETRY", "1"),
        ("ENABLE_BETA_TRACING_DETAILED", "1"),
        ("BETA_TRACING_ENDPOINT", base.as_str()),
        ("OTEL_LOGS_EXPORTER", "otlp"),
        ("OTEL_TRACES_EXPORTER", "otlp"),
        ("OTEL_METRICS_EXPORTER", "none"),
        ("OTEL_EXPORTER_OTLP_PROTOCOL", "http/json"),
        ("OTEL_EXPORTER_OTLP_LOGS_ENDPOINT", logs.as_str()),
        ("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT", traces.as_str()),
        ("OTEL_LOG_USER_PROMPTS", "0"),
        ("OTEL_LOG_TOOL_DETAILS", "0"),
        ("OTEL_LOG_TOOL_CONTENT", "0"),
        ("OTEL_LOG_ASSISTANT_RESPONSES", "0"),
    ] {
        spec.env.insert(name.into(), value.into());
    }
    (format!("/{key}/v1/{{signal}}"), None)
}
