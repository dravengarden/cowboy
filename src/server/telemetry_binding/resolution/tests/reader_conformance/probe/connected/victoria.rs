//! Exact third-party programs, disposable storage and loopback only.
use super::super::super::victoria::{DatabaseArtifact, DatabaseReport};
use super::*;
use crate::otlp::{Request, Signal};

mod query;

struct Database {
    signal: Signal,
    executable: PathBuf,
    root: PathBuf,
    address: std::net::SocketAddr,
    process: Option<Running>,
}

pub(super) struct Databases {
    entries: Vec<Database>,
    client: reqwest::Client,
}

impl Databases {
    pub async fn start(root: &Path, artifacts: &[DatabaseArtifact]) -> Result<Self, Failure> {
        check(artifacts.len() == 3)?;
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(3))
            .build()
            .map_err(|_| Failure::Setup)?;
        let mut databases = Self {
            entries: Vec::new(),
            client,
        };
        for (artifact, expected) in
            artifacts
                .iter()
                .zip([Signal::Logs, Signal::Metrics, Signal::Traces])
        {
            check(artifact.signal == expected)?;
            let directory = root.join(match expected {
                Signal::Logs => "victoria-logs",
                Signal::Metrics => "victoria-metrics",
                Signal::Traces => "victoria-traces",
            });
            for name in ["data", "tmp", "tools"] {
                let path = directory.join(name);
                std::fs::create_dir_all(&path).map_err(|_| Failure::Setup)?;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
                    .map_err(|_| Failure::Setup)?;
            }
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .map_err(|_| Failure::Setup)?;
            let address = listener.local_addr().map_err(|_| Failure::Setup)?;
            databases.entries.push(Database {
                signal: expected,
                executable: artifact.executable.clone(),
                root: directory,
                address,
                process: None,
            });
        }
        let result = databases.spawn().await;
        if let Err(failure) = result {
            databases.finish().await?;
            return Err(failure);
        }
        Ok(databases)
    }

    async fn spawn(&mut self) -> Result<(), Failure> {
        for database in &mut self.entries {
            check(database.process.is_none())?;
            let mut command = command(&database.executable, &database.root);
            command
                .env("GOMAXPROCS", "2")
                .arg(format!("-httpListenAddr={}", database.address))
                .arg(format!(
                    "-storageDataPath={}",
                    database.root.join("data").display()
                ))
                .arg("-memory.allowedBytes=128MiB")
                .arg("-retentionPeriod=1d")
                .arg("-loggerLevel=ERROR");
            if database.signal == Signal::Metrics {
                command
                    .arg("-opentelemetry.usePrometheusNaming=false")
                    .arg("-opentelemetry.convertMetricNamesToPrometheus=false");
            }
            database.process = Some(Running::spawn(&mut command)?);
            let process = database.process.as_mut().ok_or(Failure::Spawn)?;
            check(
                std::fs::read_link(format!("/proc/{}/exe", process.pid.as_raw_nonzero()))
                    .is_ok_and(|path| path == database.executable),
            )?;
            tokio::time::timeout(DEADLINE, async {
                loop {
                    if process
                        .child
                        .try_wait()
                        .map_err(|_| Failure::Spawn)?
                        .is_some()
                    {
                        return Err(Failure::ExitedBeforeReady);
                    }
                    if self
                        .client
                        .get(format!("http://{}/health", database.address))
                        .send()
                        .await
                        .is_ok_and(|response| response.status() == reqwest::StatusCode::OK)
                    {
                        return Ok(());
                    }
                    tokio::time::sleep(Duration::from_millis(40)).await;
                }
            })
            .await
            .map_err(|_| Failure::Timeout)??;
        }
        Ok(())
    }

    pub fn addresses(&self) -> [std::net::SocketAddr; 3] {
        std::array::from_fn(|index| self.entries[index].address)
    }

    async fn restart(&mut self) -> Result<(), Failure> {
        for database in &mut self.entries {
            let process = database.process.as_mut().ok_or(Failure::Setup)?;
            process.terminate().await?;
            database.process = None;
        }
        self.spawn().await
    }

    pub async fn finish(&mut self) -> Result<(), Failure> {
        let mut result = Ok(());
        for database in &mut self.entries {
            if let Some(mut process) = database.process.take() {
                result = result.and(process.finish().await);
            }
        }
        result
    }

    async fn get(
        &self,
        index: usize,
        path: &str,
        params: &[(&str, &str)],
    ) -> Result<Vec<u8>, Failure> {
        let address = self.entries[index].address;
        let mut response = self
            .client
            .get(format!("http://{address}{path}"))
            .query(params)
            .send()
            .await
            .map_err(|_| Failure::ConnectionClosed)?;
        if response.status() != reqwest::StatusCode::OK {
            return Err(Failure::DatabaseQueryHttp {
                signal: self.entries[index].signal,
                status: response.status().as_u16(),
            });
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| Failure::FrameDecode)? {
            check(bytes.len() + chunk.len() <= 64 * 1024)?;
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }

    async fn rows(&self, index: usize) -> Result<Vec<Value>, Failure> {
        let (path, params) = if index == 1 {
            (
                "/api/v1/export",
                vec![
                    ("match[]", "{__name__!=\"\"}"),
                    ("max_rows_per_line", "100"),
                ],
            )
        } else {
            ("/select/logsql/query", vec![("query", "* | limit 8")])
        };
        self.get(index, path, &params)
            .await?
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).map_err(|_| Failure::FrameDecode))
            .collect()
    }
}

fn fresh_fixtures() -> Result<Vec<(Signal, Vec<u8>)>, Failure> {
    let now = u64::try_from(
        chrono::Utc::now()
            .timestamp_nanos_opt()
            .ok_or(Failure::Setup)?,
    )
    .map_err(|_| Failure::Setup)?
    .checked_sub(1_000_000_000)
    .ok_or(Failure::Setup)?;
    crate::otlp::client_fixtures()
        .into_iter()
        .map(|(signal, bytes)| {
            let mut request = Request::decode(signal, &bytes).map_err(|_| Failure::FrameDecode)?;
            match &mut request {
                Request::Logs(logs) => {
                    for record in logs
                        .resource_logs
                        .iter_mut()
                        .flat_map(|r| &mut r.scope_logs)
                        .flat_map(|s| &mut s.log_records)
                    {
                        record.time_unix_nano = now;
                        record.observed_time_unix_nano = now;
                    }
                }
                Request::Traces(traces) => {
                    for span in traces
                        .resource_spans
                        .iter_mut()
                        .flat_map(|r| &mut r.scope_spans)
                        .flat_map(|s| &mut s.spans)
                    {
                        let duration = span
                            .end_time_unix_nano
                            .checked_sub(span.start_time_unix_nano)
                            .ok_or(Failure::Setup)?;
                        span.start_time_unix_nano = now;
                        span.end_time_unix_nano = now + duration;
                    }
                }
                Request::Metrics(metrics) => {
                    for metric in metrics
                        .resource_metrics
                        .iter_mut()
                        .flat_map(|r| &mut r.scope_metrics)
                        .flat_map(|s| &mut s.metrics)
                    {
                        use opentelemetry_proto::tonic::metrics::v1::metric::Data;
                        match metric.data.as_mut() {
                            Some(Data::Sum(sum)) => {
                                for point in &mut sum.data_points {
                                    point.start_time_unix_nano = now - 1_000_000_000;
                                    point.time_unix_nano = now;
                                }
                            }
                            Some(Data::Histogram(histogram)) => {
                                for point in &mut histogram.data_points {
                                    point.start_time_unix_nano = now - 1_000_000_000;
                                    point.time_unix_nano = now;
                                }
                            }
                            _ => return Err(Failure::Setup),
                        }
                    }
                }
            }
            Ok((
                signal,
                request.export().decode().map_err(|_| Failure::Setup)?.0,
            ))
        })
        .collect()
}

pub(super) async fn exercise(
    pair: &mut Pair<'_>,
    stage: &mut Stage,
    delivery_report: &mut DeliveryReport,
    report: &mut DatabaseReport,
) -> Result<(), Failure> {
    let fixtures = fresh_fixtures()?;
    *stage = Stage::ExportDelivery;
    delivery::round::run_fixtures(
        pair,
        DeliveryStep::Unconfigured,
        delivery_report,
        fixtures.clone(),
    )
    .await?;
    *stage = Stage::Confirmation;
    let selection = flows::plan(pair, pair.selection()).await?;
    flows::applied(pair, &selection).await?;
    delivery::round::run_fixtures(
        pair,
        DeliveryStep::BindingOnly,
        delivery_report,
        fixtures.clone(),
    )
    .await?;
    for index in 0..3 {
        check(
            pair.databases
                .as_ref()
                .ok_or(Failure::Setup)?
                .rows(index)
                .await?
                .is_empty(),
        )?;
    }
    report.empty_before_export = true;
    *stage = Stage::ExportActivation;
    let (path, policy) = delivery::activate(pair, delivery_report, false).await?;
    *stage = Stage::ExportDelivery;
    delivery::round::run_fixtures(pair, DeliveryStep::Delivered, delivery_report, fixtures).await?;
    let requests = pair
        .destination
        .as_ref()
        .ok_or(Failure::Setup)?
        .requests()?;
    check(requests.len() == 4)?;
    let expected = query::Expected::decode(&requests)?;
    let first = expected
        .verify(pair.databases.as_ref().ok_or(Failure::Setup)?, false)
        .await?;
    report.queries.push(first);
    *stage = Stage::Reopen;
    pair.databases
        .as_mut()
        .ok_or(Failure::Setup)?
        .restart()
        .await?;
    report.database_restarts = 1;
    let second = expected
        .verify(pair.databases.as_ref().ok_or(Failure::Setup)?, true)
        .await?;
    check(second.normalized_result_sha256 == report.queries[0].normalized_result_sha256)?;
    report.queries.push(second);
    delivery::reopen_without_replay(pair, true).await?;
    report.no_replay_after_host_restart = true;
    check(
        super::super::admission::policy_snapshot(&path).map_err(|_| Failure::EvidenceChanged)?
            == Some(policy),
    )?;
    let counts = pair.proxy.counts()?;
    check(
        counts.binding_commands == 1
            && counts.export_commands == 4
            && counts.export_receipts == 4
            && counts.recovery_commands == 0
            && counts.dropped_export_acks == 0
            && delivery_report.rounds.len() == 3
            && delivery_report.rounds.iter().all(|round| round.accepted),
    )
}
