//! Optional, exact-generation telemetry Plugin activation. Public packages
//! carry no credentials; only Machine-private configuration selects egress.

#[cfg(all(test, feature = "full"))]
mod controller_tests;

use std::fs::OpenOptions;
use std::io::Read as _;
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::Path;

use anyhow::{Context as _, Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PluginSelection {
    pub plugin_id: String,
    pub plugin_version: String,
    pub generation_digest: String,
}

#[cfg(feature = "full")]
fn read_private<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    read_private_snapshot(path).map(|(value, _)| value)
}

// Local observation fences, not durable policy epochs. Holding the open file
// pins its inode so atomic replacement cannot recreate the same identity while
// a request retains this snapshot. No bytes, paths or digests enter a receipt.
#[cfg_attr(not(feature = "machine-host"), allow(dead_code))] // Controller only consumes the parsed startup selection.
struct PrivateSnapshot {
    file: std::fs::File,
    digest: [u8; 32],
    changed: (i64, i64),
}

fn read_private_snapshot<T: serde::de::DeserializeOwned>(
    path: &Path,
) -> Result<(T, PrivateSnapshot)> {
    use sha2::{Digest as _, Sha256};
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
        .context("opening private telemetry configuration")?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file()
            && metadata.mode() & 0o077 == 0
            && metadata.nlink() == 1
            && metadata.uid() == rustix::process::geteuid().as_raw(),
        "telemetry configuration must be an owned private regular file"
    );
    let mut bytes = Vec::new();
    (&file).take(64 * 1024 + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 64 * 1024,
        "telemetry configuration too large"
    );
    let after = file.metadata()?;
    ensure!(
        metadata.ctime() == after.ctime()
            && metadata.ctime_nsec() == after.ctime_nsec()
            && metadata.len() == after.len()
            && metadata.mode() == after.mode()
            && metadata.nlink() == after.nlink()
            && metadata.uid() == after.uid(),
        "telemetry configuration changed during read"
    );
    let value = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("invalid telemetry configuration"))?;
    Ok((
        value,
        PrivateSnapshot {
            file,
            digest: Sha256::digest(&bytes).into(),
            changed: (after.ctime(), after.ctime_nsec()),
        },
    ))
}

impl PluginSelection {
    fn validate(&self) -> Result<()> {
        ensure!(
            !self.plugin_id.is_empty()
                && self.plugin_id.len() <= 128
                && self
                    .plugin_id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')),
            "invalid telemetry plugin id"
        );
        ensure!(
            semver::Version::parse(&self.plugin_version).is_ok(),
            "invalid telemetry plugin version"
        );
        let digest = self
            .generation_digest
            .strip_prefix("sha256:")
            .unwrap_or_default();
        ensure!(
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()),
            "invalid telemetry generation digest"
        );
        Ok(())
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExportResult {
    pub logs_delivered: bool,
    pub metrics_delivered: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub otlp: Option<OtlpResult>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OtlpResult {
    pub enabled: bool,
    pub delivered: bool,
    pub rejected_items: u64,
}

#[cfg(feature = "full")]
pub(crate) fn controller_exporter(
    path: Option<&Path>,
    control: std::sync::Arc<crate::machine_control::MachineControl>,
    catalog: std::sync::Arc<crate::plugin_catalog::PluginCatalog>,
) -> Result<Option<crate::observability::TelemetryExporter>> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Configuration {
        machine_id: String,
        plugin: PluginSelection,
    }
    let Some(path) = path else {
        return Ok(None);
    };
    let config: Configuration = read_private(path)?;
    config.plugin.validate()?;
    ensure!(
        !config.machine_id.is_empty() && config.machine_id.len() <= 128,
        "invalid telemetry Machine selection"
    );
    Ok(Some(std::sync::Arc::new(move |batch| {
        let selection = config.plugin.clone();
        let machine_id = config.machine_id.clone();
        let control = std::sync::Arc::clone(&control);
        let catalog = std::sync::Arc::clone(&catalog);
        Box::pin(async move {
            // Policy is the private, exact Service selection captured at
            // startup. A composition JSON report can never select this port.
            let Ok(release) = catalog.resolve_telemetry_backend(
                &selection.plugin_id,
                &selection.plugin_version,
                &selection.generation_digest,
            ) else {
                return crate::observability::ExportReceipt::default();
            };
            let signal = batch.otlp.as_ref().map(|v| v.signal);
            let Some(operation) = release.operation_for(signal) else {
                return crate::observability::ExportReceipt::default();
            };
            let selected = control
                .connected_plugin_inventory()
                .into_iter()
                .find(|entry| {
                    entry.machine_id == machine_id && release.matches_inventory(&entry.plugin)
                });
            let Some(selected) = selected else {
                return crate::observability::ExportReceipt::default();
            };
            let Ok(binding) = control.bind_plugin_host(&machine_id, &selected.plugin, operation)
            else {
                return crate::observability::ExportReceipt::default();
            };
            let payload = match batch.otlp {
                Some(otlp) => serde_json::to_value(otlp).expect("OTLP envelope"),
                None => serde_json::json!({"logs": batch.logs, "metrics": batch.metrics}),
            };
            let result = control.invoke_plugin_host(binding, payload).await;
            // Never log commands or private endpoint errors. Only bounded lane
            // receipts reach observability; no host payload enters history.
            let receipt = result
                .ok()
                .and_then(|value| serde_json::from_value::<ExportResult>(value).ok())
                .unwrap_or_default();
            if let Some(signal) = signal {
                let Some(otlp) = receipt.otlp else {
                    return crate::observability::ExportReceipt::default();
                };
                let success = !otlp.enabled || otlp.delivered;
                return crate::observability::ExportReceipt {
                    logs_delivered: signal == crate::otlp::Signal::Logs && success,
                    metrics_delivered: signal == crate::otlp::Signal::Metrics && success,
                    traces_delivered: signal == crate::otlp::Signal::Traces && success,
                    rejected_items: otlp.rejected_items.min(crate::otlp::MAX_ITEMS as u64),
                };
            }
            crate::observability::ExportReceipt {
                logs_delivered: receipt.logs_delivered,
                metrics_delivered: receipt.metrics_delivered,
                ..Default::default()
            }
        })
    })))
}

#[cfg(feature = "machine-host")]
mod machine {
    use super::*;
    use cowboy_plugin_sdk::{TelemetryBackendContract, TelemetryEncoding, TelemetryRoute};
    use std::time::Duration;

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub(crate) struct Configuration {
        pub plugin: PluginSelection,
        logs: Option<Endpoint>,
        metrics: Option<Endpoint>,
        traces: Option<Endpoint>,
    }

    pub(crate) struct PreparedPolicy {
        config: Configuration,
        snapshot: PrivateSnapshot,
    }

    impl PreparedPolicy {
        pub(crate) fn unchanged(&self, path: &Path) -> bool {
            let Ok((_, current)) = read_private_snapshot::<Configuration>(path) else {
                return false;
            };
            let Ok(original) = self.snapshot.file.metadata() else {
                return false;
            };
            let Ok(observed) = current.file.metadata() else {
                return false;
            };
            original.dev() == observed.dev()
                && original.ino() == observed.ino()
                && self.snapshot.digest == current.digest
                && self.snapshot.changed == current.changed
        }
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Endpoint {
        base_url: String,
        #[serde(default)]
        bearer_token: Option<String>,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub(crate) struct LegacyPayload {
        logs: String,
        metrics: String,
    }

    pub(crate) enum Payload {
        Legacy(LegacyPayload),
        Otlp(crate::otlp::Export),
    }

    pub(crate) fn prepare(
        path: &Path,
        selection: &PluginSelection,
        value: serde_json::Value,
        otlp: bool,
    ) -> Result<(PreparedPolicy, Payload)> {
        let (config, snapshot): (Configuration, _) = read_private_snapshot(path)?;
        config.plugin.validate()?;
        ensure!(
            config.plugin.plugin_id == selection.plugin_id
                && config.plugin.plugin_version == selection.plugin_version
                && config.plugin.generation_digest == selection.generation_digest,
            "telemetry release is not enabled by Machine policy"
        );
        let payload = if otlp {
            let payload: crate::otlp::Export = serde_json::from_value(value)
                .map_err(|_| anyhow::anyhow!("invalid OTLP export payload"))?;
            payload.decode()?;
            Payload::Otlp(payload)
        } else {
            let payload: LegacyPayload = serde_json::from_value(value)
                .map_err(|_| anyhow::anyhow!("invalid telemetry export payload"))?;
            ensure!(
                payload.logs.len().saturating_add(payload.metrics.len()) <= 512 * 1024,
                "telemetry export payload too large"
            );
            Payload::Legacy(payload)
        };
        for endpoint in [&config.logs, &config.metrics, &config.traces]
            .into_iter()
            .flatten()
        {
            endpoint.validate()?;
        }
        Ok((PreparedPolicy { config, snapshot }, payload))
    }

    impl Endpoint {
        fn validate(&self) -> Result<()> {
            let url = url::Url::parse(&self.base_url)
                .map_err(|_| anyhow::anyhow!("invalid telemetry endpoint URL"))?;
            let loopback = url.host_str().is_some_and(|host| {
                host.trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
            });
            ensure!(
                url.scheme() == "https" || (url.scheme() == "http" && loopback),
                "telemetry endpoints require HTTPS (HTTP allowed only for literal loopback)"
            );
            ensure!(
                url.has_host()
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.query().is_none()
                    && url.fragment().is_none()
                    && self.base_url.len() <= 2048,
                "telemetry endpoint must not embed credentials, query or fragment"
            );
            if let Some(token) = &self.bearer_token {
                ensure!(
                    !token.is_empty()
                        && token.len() <= 4096
                        && token.bytes().all(|byte| byte.is_ascii_graphic()),
                    "invalid telemetry authorization value"
                );
            }
            Ok(())
        }
    }

    pub(crate) async fn export<F, Fut>(
        contract: &TelemetryBackendContract,
        policy: &PreparedPolicy,
        payload: Payload,
        admit: &F,
    ) -> ExportResult
    where
        F: Fn() -> Fut + Sync,
        Fut: std::future::Future<Output = bool> + Send,
    {
        let config = &policy.config;
        let Ok(client) = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(1))
            .timeout(Duration::from_secs(3))
            .build()
        else {
            return ExportResult::default();
        };
        let payload = match payload {
            Payload::Otlp(payload) => {
                let (route, endpoint) = match payload.signal {
                    crate::otlp::Signal::Logs => (contract.logs.as_ref(), config.logs.as_ref()),
                    crate::otlp::Signal::Metrics => {
                        (contract.metrics.as_ref(), config.metrics.as_ref())
                    }
                    crate::otlp::Signal::Traces => {
                        (contract.traces.as_ref(), config.traces.as_ref())
                    }
                };
                let receipt = post_otlp(&client, route, endpoint, &payload, admit).await;
                return ExportResult {
                    otlp: Some(receipt),
                    ..Default::default()
                };
            }
            Payload::Legacy(payload) => payload,
        };
        let (logs_delivered, metrics_delivered) = tokio::join!(
            post(
                &client,
                contract.logs.as_ref(),
                config.logs.as_ref(),
                payload.logs,
                admit
            ),
            post(
                &client,
                contract.metrics.as_ref(),
                config.metrics.as_ref(),
                payload.metrics,
                admit
            ),
        );
        ExportResult {
            logs_delivered,
            metrics_delivered,
            otlp: None,
        }
    }

    async fn post<F, Fut>(
        client: &reqwest::Client,
        route: Option<&TelemetryRoute>,
        endpoint: Option<&Endpoint>,
        body: String,
        admit: &F,
    ) -> bool
    where
        F: Fn() -> Fut + Sync,
        Fut: std::future::Future<Output = bool> + Send,
    {
        if body.is_empty() {
            return true;
        }
        let (Some(route), Some(endpoint)) = (route, endpoint) else {
            return false;
        };
        let Ok(mut url) = url::Url::parse(&endpoint.base_url) else {
            return false;
        };
        url.set_path(&format!(
            "{}{}",
            url.path().trim_end_matches('/'),
            route.path
        ));
        if !route.query.is_empty() {
            url.query_pairs_mut().extend_pairs(&route.query);
        }
        for attempt in 0..2 {
            let mut request = client
                .post(url.clone())
                .header(
                    reqwest::header::CONTENT_TYPE,
                    match route.encoding {
                        TelemetryEncoding::JsonLines => "application/stream+json",
                        TelemetryEncoding::PrometheusText => "text/plain; version=0.0.4",
                        TelemetryEncoding::OtlpHttpProtobuf => return false,
                    },
                )
                .body(body.clone());
            if let Some(token) = &endpoint.bearer_token {
                request = request.bearer_auth(token);
            }
            if !admit().await {
                return false;
            }
            let retry = match request.send().await {
                Ok(response) if response.status().is_success() => return true,
                Ok(response) => {
                    response.status().is_server_error()
                        || matches!(response.status().as_u16(), 408 | 429)
                }
                Err(_) => true,
            };
            if !retry || attempt == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        false
    }

    async fn post_otlp<F, Fut>(
        client: &reqwest::Client,
        route: Option<&TelemetryRoute>,
        endpoint: Option<&Endpoint>,
        payload: &crate::otlp::Export,
        admit: &F,
    ) -> OtlpResult
    where
        F: Fn() -> Fut + Sync,
        Fut: std::future::Future<Output = bool> + Send,
    {
        // Explicitly disabled lanes stay local. Missing/incompatible routes on
        // an enabled lane are failures, never a legacy-encoding fallback.
        let Some(endpoint) = endpoint else {
            return OtlpResult::default();
        };
        let failed = || OtlpResult {
            enabled: true,
            ..Default::default()
        };
        let Some(route) = route.filter(|r| r.encoding == TelemetryEncoding::OtlpHttpProtobuf)
        else {
            return failed();
        };
        let Ok((body, count)) = payload.decode() else {
            return failed();
        };
        let Ok(mut url) = url::Url::parse(&endpoint.base_url) else {
            return failed();
        };
        url.set_path(&format!(
            "{}{}",
            url.path().trim_end_matches('/'),
            route.path
        ));
        for attempt in 0..2 {
            let mut request = client
                .post(url.clone())
                .header(reqwest::header::CONTENT_TYPE, "application/x-protobuf")
                .body(body.clone());
            if let Some(token) = &endpoint.bearer_token {
                request = request.bearer_auth(token);
            }
            if !admit().await {
                return failed();
            }
            let retry = match request.send().await {
                Ok(mut response) if response.status() == reqwest::StatusCode::OK => {
                    if response
                        .headers()
                        .get(reqwest::header::CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .is_some_and(|v| v != "application/x-protobuf")
                    {
                        return failed();
                    }
                    let mut bytes = Vec::new();
                    loop {
                        match response.chunk().await {
                            Ok(Some(chunk))
                                if bytes.len().saturating_add(chunk.len()) <= 64 * 1024 =>
                            {
                                bytes.extend_from_slice(&chunk)
                            }
                            Ok(None) => break,
                            _ => return failed(),
                        }
                    }
                    // No retries for partial success (including warning-only),
                    // invalid response bodies or a successful HTTP receipt.
                    return match payload.signal.rejected(&bytes, count) {
                        Ok(rejected_items) => OtlpResult {
                            enabled: true,
                            delivered: rejected_items == 0,
                            rejected_items,
                        },
                        Err(_) => failed(),
                    };
                }
                Ok(response) => matches!(response.status().as_u16(), 429 | 502 | 503 | 504),
                Err(_) => true,
            };
            if !retry || attempt == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        failed()
    }
    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn endpoint_policy_rejects_secret_urls_and_non_loopback_plaintext() {
            for url in [
                "http://example.test",
                "http://localhost:8428",
                "https://user:pass@example.test",
                "https://example.test?token=secret",
                "https://example.test#secret",
                "file:///tmp/export",
            ] {
                assert!(
                    Endpoint {
                        base_url: url.into(),
                        bearer_token: None
                    }
                    .validate()
                    .is_err()
                );
            }
            for url in [
                "http://127.0.0.1:8428",
                "http://[::1]:8428",
                "https://example.test/tenant",
            ] {
                Endpoint {
                    base_url: url.into(),
                    bearer_token: None,
                }
                .validate()
                .unwrap();
            }
            assert!(
                Endpoint {
                    base_url: "https://example.test".into(),
                    bearer_token: Some("secret\r\nX-Header: injected".into())
                }
                .validate()
                .is_err()
            );
        }

        #[tokio::test]
        #[cfg(feature = "full")]
        async fn redirects_do_not_forward_credentials_and_slow_destinations_are_bounded() {
            use std::sync::Arc;
            use std::sync::atomic::{AtomicUsize, Ordering};
            let _ = rustls::crypto::ring::default_provider().install_default();
            let count = Arc::new(AtomicUsize::new(0));
            let requests = Arc::clone(&count);
            let app = axum::Router::new().fallback(move |uri: axum::extract::OriginalUri| {
                let requests = Arc::clone(&requests);
                async move {
                    requests.fetch_add(1, Ordering::Relaxed);
                    if uri.path() == "/slow" {
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                    (
                        axum::http::StatusCode::TEMPORARY_REDIRECT,
                        [("location", "/credential-trap")],
                    )
                }
            });
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = Endpoint {
                base_url: format!("http://{}", listener.local_addr().unwrap()),
                bearer_token: Some("fixture-not-real".into()),
            };
            let server = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            let client = reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_millis(40))
                .build()
                .unwrap();
            let mut route = TelemetryRoute {
                encoding: TelemetryEncoding::JsonLines,
                path: "/redirect".into(),
                query: Default::default(),
            };
            let admit = || async { true };
            assert!(
                !post(
                    &client,
                    Some(&route),
                    Some(&endpoint),
                    "{}\n".into(),
                    &admit
                )
                .await
            );
            assert_eq!(count.load(Ordering::Relaxed), 1);
            route.path = "/slow".into();
            assert!(
                !tokio::time::timeout(
                    Duration::from_secs(1),
                    post(
                        &client,
                        Some(&route),
                        Some(&endpoint),
                        "{}\n".into(),
                        &admit
                    )
                )
                .await
                .unwrap()
            );
            assert_eq!(count.load(Ordering::Relaxed), 3);
            server.abort();
        }
    }
}

#[cfg(feature = "machine-host")]
pub(crate) use machine::{export, prepare};
