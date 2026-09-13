use super::super::startup::{StartupCase, StartupFixture};
use super::*;

pub(in super::super) async fn run(
    artifact: &Artifact,
    fixture: &StartupFixture,
    root: &Path,
    cold_read: u8,
) -> Result<Option<bool>, Failure> {
    let policy_path = root.join("background-policy.json");
    if cold_read == 1
        && let Some(bytes) = &fixture.policy
    {
        private_write(&policy_path, bytes).map_err(|_| Failure::Setup)?;
    }
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|_| Failure::Setup)?;
    let address = listener.local_addr().map_err(|_| Failure::Setup)?;
    let mut command = command(&artifact.executable, root);
    command
        .arg("serve")
        .arg("--bind")
        .arg(address.to_string())
        .arg("--data-dir")
        .arg(root.join("controller"))
        .arg("--database-url")
        .arg(database(root))
        .arg("--workspace-root")
        .arg(root.join("workspace"))
        .arg("--web-root")
        .arg(root.join("no-web"))
        .arg("--telemetry-dir")
        .arg(root.join("controller/telemetry"));
    if fixture.policy.is_some() {
        command
            .arg("--telemetry-managed-export-policy")
            .arg(&policy_path);
    }
    if fixture.case == StartupCase::ConflictingModes {
        // Must reject conflicting selection before trying to open either file;
        // a stopped managed exporter must never select this fallback.
        command
            .arg("--telemetry-plugin-config")
            .arg(root.join("no-legacy-policy"));
    }
    drop(listener);
    let mut running = Running::spawn(&mut command)?;
    let result = tokio::time::timeout(DEADLINE, async {
        if let Some(marker) = fixture.case.failure_marker() {
            let status = running
                .child
                .wait()
                .await
                .map_err(|_| Failure::ExitedBeforeReady)?;
            tokio::time::sleep(Duration::from_millis(20)).await;
            if status.code().is_none_or(|code| code == 0) || !running.log_contains(marker) {
                return Err(Failure::UnexpectedReadiness);
            }
            return Ok(None);
        }
        controller(&mut running, address, &fixture.evidence).await?;
        let active = fixture.case.export_active();
        let has_activation = running.log_contains("managed telemetry background startup evaluated");
        if fixture.policy.is_some() {
            if !has_activation
                || !running.log_contains(if active {
                    "export_active=true"
                } else {
                    "export_active=false"
                })
            {
                return Err(Failure::WrongExportState);
            }
        } else if has_activation {
            return Err(Failure::WrongExportState);
        }
        local_recording(address, root, cold_read, active).await?;
        Ok(Some(active))
    })
    .await
    .unwrap_or(Err(Failure::Timeout));
    let cleanup = running.finish().await;
    let retained = unchanged(root, &fixture.evidence).map_err(|_| Failure::EvidenceChanged);
    let policy_unchanged = match &fixture.policy {
        Some(bytes) => std::fs::read(&policy_path).is_ok_and(|retained| retained == *bytes),
        None => {
            matches!(policy_path.symlink_metadata(), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
        }
    };
    cleanup
        .and(retained)
        .and(if policy_unchanged {
            Ok(())
        } else {
            Err(Failure::EvidenceChanged)
        })
        .and(result)
}

async fn metrics(
    client: &reqwest::Client,
    address: std::net::SocketAddr,
) -> Result<serde_json::Value, Failure> {
    client
        .get(format!("http://{address}/api/metrics"))
        .send()
        .await
        .map_err(|_| Failure::LocalRecording)?
        .error_for_status()
        .map_err(|_| Failure::LocalRecording)?
        .json()
        .await
        .map_err(|_| Failure::LocalRecording)
}

pub(super) async fn local_recording(
    address: std::net::SocketAddr,
    root: &Path,
    cold_read: u8,
    active: bool,
) -> Result<(), Failure> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(1))
        .build()
        .map_err(|_| Failure::Setup)?;
    local_recording_with_client(&client, address, root, cold_read, active).await
}

pub(super) async fn local_recording_with_client(
    client: &reqwest::Client,
    address: std::net::SocketAddr,
    root: &Path,
    cold_read: u8,
    active: bool,
) -> Result<(), Failure> {
    let file = root.join("controller/telemetry/telemetry.jsonl");
    let before = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
    let start = metrics(client, address).await?;
    if start["observability_accepted_batches"] != 0 {
        return Err(Failure::WrongExportState); // A cold read cannot replay local files.
    }
    let fixtures = crate::otlp::client_fixtures();
    for (index, (signal, body)) in fixtures.iter().enumerate() {
        let signal = match signal {
            crate::otlp::Signal::Logs => "logs",
            crate::otlp::Signal::Metrics => "metrics",
            crate::otlp::Signal::Traces => "traces",
        };
        let response = client
            .post(format!(
                "http://{address}/api/telemetry/v1/{signal}?batch_id=startup-{cold_read}-{index}"
            ))
            .header(reqwest::header::CONTENT_TYPE, "application/x-protobuf")
            .body(body.clone())
            .send()
            .await
            .map_err(|_| Failure::LocalRecording)?;
        if response.status() != reqwest::StatusCode::OK {
            return Err(Failure::LocalRecording);
        }
    }
    loop {
        let current = metrics(client, address).await?;
        let failures = [
            (
                "observability_failed_log_batches",
                crate::otlp::Signal::Logs,
            ),
            (
                "observability_failed_metric_batches",
                crate::otlp::Signal::Metrics,
            ),
            (
                "observability_failed_trace_batches",
                crate::otlp::Signal::Traces,
            ),
        ];
        // No Machine exists in the active-queue startup fixtures. An ACTIVE queue
        // consumes each new batch once as not-admitted; stopped/unconfigured
        // modes have no queue and must leave every remote counter at zero.
        let expected = |signal| {
            if active {
                fixtures.iter().filter(|(kind, _)| *kind == signal).count() as u64
            } else {
                0
            }
        };
        if failures.iter().any(|(key, signal)| {
            current[key]
                .as_u64()
                .is_none_or(|value| value > expected(*signal))
        }) || [
            "observability_failed_file_batches",
            "observability_dropped_export_batches",
            "observability_dropped_batches",
        ]
        .iter()
        .any(|key| current[key] != 0)
        {
            return Err(Failure::WrongExportState);
        }
        if current["observability_pending"] == 0
            && current["observability_accepted_batches"].as_u64() == Some(fixtures.len() as u64)
            && failures
                .iter()
                .all(|(key, signal)| current[key].as_u64() == Some(expected(*signal)))
        {
            let bytes = std::fs::read(&file).map_err(|_| Failure::LocalRecording)?;
            if bytes.len() as u64 <= before
                || std::fs::metadata(&file)
                    .map_err(|_| Failure::LocalRecording)?
                    .permissions()
                    .mode()
                    & 0o077
                    != 0
            {
                return Err(Failure::LocalRecording);
            }
            // Confirm actual records, not just creation of an empty local file.
            let delta = bytes
                .get(before as usize..)
                .ok_or(Failure::LocalRecording)?;
            let records: Vec<_> = delta
                .split(|byte| *byte == b'\n')
                .filter(|line| !line.is_empty())
                .collect();
            if records.len() < fixtures.len()
                || records
                    .iter()
                    .any(|line| serde_json::from_slice::<serde_json::Value>(line).is_err())
            {
                return Err(Failure::LocalRecording);
            }
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
