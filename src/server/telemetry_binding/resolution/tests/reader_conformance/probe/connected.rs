use super::super::connected::{
    Ack, ConnectedFixture, DatabaseHttp, DatabaseMediaType, DeliveryReport, DeliveryRound,
    DeliveryStep, Evidence, Flow, HttpExport, HttpObservation, HttpResult, Outcome, RelayRejection,
    ResponseMode, Stage, WireCounts, WireExport,
};
use super::*;
use serde_json::{Value, json};

mod delivery;
mod flows;
mod http;
mod proxy;
mod victoria;

struct Background {
    path: PathBuf,
    active: bool,
}

struct Pair<'a> {
    root: &'a Path,
    fixture: ConnectedFixture,
    address: std::net::SocketAddr,
    controller_artifact: &'a Artifact,
    machine_artifact: &'a Artifact,
    controller: Option<Running>,
    machine: Option<Running>,
    proxy: proxy::Proxy,
    http: http::Http,
    destination: Option<delivery::Destination>,
    databases: Option<victoria::Databases>,
    background: Option<Background>,
}

impl Pair<'_> {
    async fn start_controller(&mut self) -> Result<(), Failure> {
        let mut command = configured_command(self.controller_artifact, self.root, self.address);
        command
            .arg("--product-auth-enabled")
            .arg("true")
            .arg("--core-security-config")
            .arg(self.root.join("core-security.json"))
            .arg("--telemetry-writer-policy")
            .arg(self.root.join("controller-writer.json"))
            .arg("--plugin-catalog-dir")
            .arg(self.root.join("catalog"));
        if let Some(background) = &self.background {
            command
                .arg("--telemetry-managed-export-policy")
                .arg(&background.path);
        }
        self.controller = Some(Running::spawn(&mut command)?);
        let mut reader = self.fixture.reader.clone();
        reader.document = self.evidence()?.service;
        tokio::time::timeout(
            DEADLINE,
            controller_admission(
                self.controller.as_mut().unwrap(),
                self.address,
                &reader,
                self.fixture.controller_binding,
            ),
        )
        .await
        .map_err(|_| Failure::Timeout)??;
        if let Some(background) = &self.background {
            let running = self.controller.as_ref().unwrap();
            check(
                running.log_contains("managed telemetry background startup evaluated")
                    && running.log_contains(if background.active {
                        "export_active=true"
                    } else {
                        "export_active=false"
                    }),
            )?;
        }
        Ok(())
    }

    fn start_machine(&mut self) -> Result<(), Failure> {
        let mut command = configured_command(self.machine_artifact, self.root, self.proxy.address);
        self.machine = Some(Running::spawn(&mut command)?);
        Ok(())
    }

    async fn connected(&self, minimum: u32) -> Result<(), Failure> {
        tokio::time::timeout(Duration::from_secs(12), async {
            loop {
                if self.ready(minimum).await? {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .map_err(|_| Failure::Timeout)?
    }

    async fn ready(&self, minimum: u32) -> Result<bool, Failure> {
        let counts = self.proxy.counts()?;
        if counts.connections < minimum
            || counts.runtime_configurations != counts.connections
            || !self
                .machine
                .as_ref()
                .is_some_and(|m| m.log_contains("Machine controller authenticated"))
        {
            return Ok(false);
        }
        let Ok(health) = self
            .http
            .get(&format!("/api/machines/{MACHINE}/deployment-health"))
            .await
        else {
            return Ok(false);
        };
        if health["connected"] != true {
            return Ok(false);
        }
        let evidence = self.evidence()?;
        if evidence.service.is_some() {
            let ledger = evidence.ledger().map_err(|_| Failure::EvidenceChanged)?;
            if let Some(operation) = ledger.operations.last().filter(|op| {
                matches!(
                    op.progress,
                    crate::telemetry_binding::Progress::NeedsAttention { .. }
                )
            }) {
                // Pending evidence needs a real protocol-18 read, not choices.
                return Ok(self
                    .http
                    .get(&format!(
                        "/api/telemetry/binding/operations/{}/machine-recovery-audit",
                        operation.intent.operation_id
                    ))
                    .await
                    .is_ok());
            }
        }
        let choices = self.http.get("/api/telemetry/binding/choices").await?;
        Ok(choices["targets"]
            .as_array()
            .is_some_and(|v| v.iter().any(|t| t == &self.target())))
    }

    fn target(&self) -> Value {
        json!({"machine_id":MACHINE,"installation":self.fixture.installation})
    }
    fn selection(&self) -> Value {
        json!({"action":"select","target":self.target()})
    }
    fn evidence(&self) -> Result<Evidence, Failure> {
        Evidence::read(self.root).map_err(|_| Failure::EvidenceChanged)
    }

    async fn reopen(&mut self, machine: bool) -> Result<(), Failure> {
        let before = self.evidence()?;
        let previous = self.proxy.counts()?.connections;
        if machine && let Some(mut process) = self.machine.take() {
            process.finish().await?;
        }
        if let Some(mut process) = self.controller.take() {
            process.finish().await?;
        }
        self.start_controller().await?;
        if machine {
            self.start_machine()?;
        }
        // The actual login cookie is durable; process restart must not switch to
        // local-user authorization or invent a replacement session.
        let me = self.http.get("/api/auth/me").await?;
        check(me["account"] == "connected-operator" && me["role"] == "operator")?;
        self.connected(previous + 1).await?;
        before.matches(self.root)
    }

    async fn finish(&mut self) -> Result<(), Failure> {
        let mut result = Ok(());
        if let Some(mut process) = self.machine.take() {
            result = result.and(process.finish().await);
        }
        if let Some(mut process) = self.controller.take() {
            result = result.and(process.finish().await);
        }
        result = result.and(self.proxy.finish().await);
        if let Some(destination) = self.destination.as_mut() {
            result = result.and(destination.finish().await);
        }
        if let Some(databases) = self.databases.as_mut() {
            result = result.and(databases.finish().await);
        }
        result
    }
}

fn check(valid: bool) -> Result<(), Failure> {
    if valid {
        Ok(())
    } else {
        Err(Failure::WrongObservation)
    }
}

pub(in super::super) async fn run(
    controller: &Artifact,
    machine: &Artifact,
    flow: Flow,
    ssh_keygen: &Path,
) -> Outcome {
    let mut outcome = Outcome::default();
    let result = prepare(controller, machine, flow, ssh_keygen, None, &mut outcome).await;
    if let Err(failure) = result {
        outcome.failure = Some(failure);
    }
    outcome
}

pub(in super::super) async fn run_victoria(
    controller: &Artifact,
    machine: &Artifact,
    ssh_keygen: &Path,
    databases: &[super::super::victoria::DatabaseArtifact],
) -> Outcome {
    let mut outcome = Outcome::default();
    let result = prepare(
        controller,
        machine,
        Flow::ManagedDelivery,
        ssh_keygen,
        Some(databases),
        &mut outcome,
    )
    .await;
    if let Err(failure) = result {
        outcome.failure = Some(failure);
    }
    outcome
}

async fn prepare(
    controller: &Artifact,
    machine: &Artifact,
    flow: Flow,
    ssh_keygen: &Path,
    database_artifacts: Option<&[super::super::victoria::DatabaseArtifact]>,
    outcome: &mut Outcome,
) -> Result<(), Failure> {
    let root = tempfile::tempdir().map_err(|_| Failure::Setup)?;
    let databases = match database_artifacts {
        Some(artifacts) => Some(victoria::Databases::start(root.path(), artifacts).await?),
        None => None,
    };
    let destination = if let Some(databases) = &databases {
        Some(delivery::Destination::forwarding(databases.addresses()).await?)
    } else if flow == Flow::ManagedDelivery {
        Some(delivery::Destination::start().await?)
    } else {
        None
    };
    let fixture = ConnectedFixture::seed(
        root.path(),
        flow,
        ssh_keygen,
        destination.as_ref().map(|d| d.address),
    )
    .await
    .map_err(|_| Failure::Setup)?;
    outcome.fixture_package_sha256 = Some(fixture.package_sha256.clone());
    outcome.fixture_release_sha256 = Some(fixture.release_sha256.clone());
    outcome.installation_sha256 = Some(sha256(
        &serde_json::to_vec(&fixture.installation).map_err(|_| Failure::Setup)?,
    ));
    let before = Evidence::read(root.path()).map_err(|_| Failure::Setup)?;
    outcome.service_before_sha256 = before.service.as_deref().map(|s| sha256(s.as_bytes()));
    outcome.machine_before_sha256 = before.machine.as_deref().map(sha256);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|_| Failure::Setup)?;
    let address = listener.local_addr().map_err(|_| Failure::Setup)?;
    let proxy = proxy::Proxy::start(address, flow).await?;
    if flow == Flow::ManagedDelivery {
        proxy.export_target(fixture.installation.clone())?;
    }
    let http = http::Http::new(address)?;
    let mut pair = Pair {
        root: root.path(),
        fixture,
        address,
        controller_artifact: controller,
        machine_artifact: machine,
        controller: None,
        machine: None,
        proxy,
        http,
        destination,
        databases,
        background: None,
    };
    drop(listener);
    let policies = policy_snapshots(root.path())?;
    let started = std::time::Instant::now();
    let result = tokio::time::timeout(Duration::from_secs(125), async {
        outcome.stage = Stage::Start;
        pair.start_controller().await?;
        outcome.stage = Stage::Authentication;
        pair.http
            .denied(reqwest::Method::GET, "/api/telemetry/binding", None)
            .await?;
        pair.http.login(&pair.fixture.password).await?;
        before.matches(pair.root)?;
        outcome.stage = Stage::Start;
        pair.start_machine()?;
        pair.connected(1).await?;
        before.matches(pair.root)?;
        if flow == Flow::ManagedDelivery {
            let report = outcome.delivery.insert(DeliveryReport::default());
            if database_artifacts.is_some() {
                let database_report = outcome
                    .victoria
                    .insert(super::super::victoria::DatabaseReport::default());
                victoria::exercise(&mut pair, &mut outcome.stage, report, database_report).await?;
            } else {
                delivery::exercise(&mut pair, &mut outcome.stage, report).await?;
            }
        } else {
            flows::exercise(&mut pair, flow, &mut outcome.stage).await?;
            let client = pair.http.recording_client()?;
            startup::local_recording_with_client(&client, pair.address, pair.root, 1, false)
                .await?;
        }
        pair.proxy.counts()?;
        check(policy_snapshots(pair.root)? == policies)?;
        Ok(())
    })
    .await
    .unwrap_or(Err(Failure::Timeout));
    let wire = pair.proxy.counts();
    outcome.wire = pair.proxy.snapshot();
    outcome.elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    outcome.last_http = pair.http.last();
    outcome.relay_rejection = pair.proxy.rejection();
    if let Some(report) = outcome.delivery.as_mut() {
        report.exports = pair.proxy.export_snapshot();
        if let Some(destination) = &pair.destination {
            report.http = destination.records().unwrap_or_default();
        }
    }
    outcome.controller_connection_fenced = pair
        .controller
        .as_ref()
        .is_some_and(|c| c.log_contains("Machine connection fenced"));
    outcome.controller_runtime_stopped = pair.controller.as_ref().is_some_and(|c| {
        c.log_contains("Machine runtime forwarding stopped")
            || c.log_contains("Machine runtime writer stopped")
            || c.log_contains("Machine runtime handshake failed")
    });
    let after = pair.evidence();
    if let Ok(after) = &after {
        outcome.service_after_sha256 = after.service.as_deref().map(|s| sha256(s.as_bytes()));
        outcome.machine_after_sha256 = after.machine.as_deref().map(sha256);
    }
    let cleanup = pair.finish().await;
    if cleanup.is_err() {
        outcome.stage = Stage::Cleanup;
    }
    cleanup
        .and(wire.map(|_| ()))
        .and(after.map(|_| ()))
        .and(result)?;
    outcome.stage = Stage::Complete;
    Ok(())
}

fn policy_snapshots(root: &Path) -> Result<Vec<super::admission::PolicySnapshot>, Failure> {
    [
        "core-security.json",
        "controller-writer.json",
        "machine/telemetry-writer-policy.json",
        "machine/telemetry.json",
    ]
    .iter()
    .map(|path| {
        super::admission::policy_snapshot(&root.join(path))
            .map_err(|_| Failure::EvidenceChanged)?
            .ok_or(Failure::EvidenceChanged)
    })
    .collect()
}
