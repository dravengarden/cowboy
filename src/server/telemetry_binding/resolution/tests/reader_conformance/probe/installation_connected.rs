use super::super::installation_connected::{
    Check, FIRST, Flow, InstallFixture, SECOND, Stage, WireCounts, WriterReport,
};
use super::connected::http::Http;
use super::*;
use serde_json::{Value, json};

mod copy;
mod evidence;
mod exercise;
mod proxy;
mod recovery;
use evidence::Evidence;

fn check(valid: bool) -> Result<(), Failure> {
    if valid {
        Ok(())
    } else {
        Err(Failure::WrongObservation)
    }
}

/// One real active writer pair produces each stopped template. Every supplied
/// reader pair receives an independent copy of those SAME durable bytes.
pub(in super::super) async fn run(artifacts: &[Artifact], flow: Flow, helper: &Path) -> Vec<Check> {
    let mut stage = Stage::Setup;
    let mut writer = WriterReport::default();
    let prepared = exercise::prepare(artifacts, flow, helper, &mut stage, &mut writer).await;
    let mut checks = Vec::new();
    for controller in artifacts.iter().filter(|a| a.lane == Lane::Controller) {
        for machine in artifacts.iter().filter(|a| a.lane == Lane::Machine) {
            let mut result = Check {
                controller_role: controller.role,
                machine_role: machine.role,
                flow,
                stage,
                writer: writer.clone(),
                reader_wire: WireCounts::default(),
                cold_reads: 0,
                normalized_service_sha256: None,
                accepted: false,
                failure: None,
            };
            let accepted = match &prepared {
                Ok(prepared) => recovery::run(prepared, controller, machine, &mut result).await,
                Err(failure) => Err(*failure),
            };
            result.accepted = accepted.is_ok();
            result.failure = accepted.err();
            eprintln!(
                "{:?}/{:?}/{flow:?}: {:?}/{:?}, reads={}",
                controller.role, machine.role, result.stage, result.failure, result.cold_reads
            );
            checks.push(result);
        }
    }
    checks
}

fn endpoint() -> String {
    format!("/api/machines/{MACHINE}/plugins/victoria")
}

struct Pair<'a> {
    root: &'a Path,
    fixture: &'a InstallFixture,
    address: std::net::SocketAddr,
    controller_artifact: &'a Artifact,
    machine_artifact: &'a Artifact,
    controller: Option<Running>,
    machine: Option<Running>,
    proxy: proxy::Proxy,
    http: Http,
}

impl<'a> Pair<'a> {
    async fn prepare(
        root: &'a Path,
        fixture: &'a InstallFixture,
        controller: &'a Artifact,
        machine: &'a Artifact,
        flow: Flow,
        previous: Option<&Http>,
    ) -> Result<Self, Failure> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|_| Failure::Setup)?;
        let address = listener.local_addr().map_err(|_| Failure::Setup)?;
        let proxy = proxy::Proxy::start(address, flow, previous.is_some()).await?;
        let http = if let Some(previous) = previous {
            previous.at(address)?
        } else {
            Http::with_timeout(address, Duration::from_secs(110))?
        };
        drop(listener);
        Ok(Self {
            root,
            fixture,
            address,
            controller_artifact: controller,
            machine_artifact: machine,
            controller: None,
            machine: None,
            proxy,
            http,
        })
    }

    async fn start(&mut self, login: bool) -> Result<(), Failure> {
        self.start_controller().await?;
        if login {
            self.http
                .denied(
                    reqwest::Method::GET,
                    &format!("{}/installation-operations", endpoint()),
                    None,
                )
                .await?;
            self.http.login(&self.fixture.password).await?;
        } else {
            let me = self.http.get("/api/auth/me").await?;
            check(me["account"] == "connected-operator" && me["role"] == "operator")?;
        }
        self.start_machine()?;
        self.connected(1).await
    }

    async fn start_controller(&mut self) -> Result<(), Failure> {
        let mut command = configured_command(self.controller_artifact, self.root, self.address);
        command
            .arg("--product-auth-enabled")
            .arg("true")
            .arg("--core-security-config")
            .arg(self.root.join("core-security.json"))
            .arg("--plugin-catalog-dir")
            .arg(self.root.join("catalog"));
        self.controller = Some(Running::spawn(&mut command)?);
        tokio::time::timeout(
            DEADLINE,
            controller(
                self.controller.as_mut().unwrap(),
                self.address,
                &self.fixture.reader,
            ),
        )
        .await
        .map_err(|_| Failure::Timeout)?
    }

    fn start_machine(&mut self) -> Result<(), Failure> {
        let mut command = configured_command(self.machine_artifact, self.root, self.proxy.address);
        command.arg("--plugin-operation-admission");
        self.machine = Some(Running::spawn(&mut command)?);
        Ok(())
    }

    async fn connected(&self, minimum: u32) -> Result<(), Failure> {
        tokio::time::timeout(DEADLINE, async {
            loop {
                let counts = self.proxy.counts()?;
                if counts.connections >= minimum
                    && counts.runtime_configurations == counts.connections
                    && self
                        .machine
                        .as_ref()
                        .is_some_and(|m| m.log_contains("Machine controller authenticated"))
                    && self
                        .http
                        .get(&format!("/api/machines/{MACHINE}/deployment-health"))
                        .await
                        .is_ok_and(|h| h["connected"] == true)
                {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .map_err(|_| Failure::Timeout)?
    }

    async fn stop_processes(&mut self) -> Result<(), Failure> {
        let mut result = Ok(());
        if let Some(mut machine) = self.machine.take() {
            result = result.and(machine.finish().await);
        }
        if let Some(mut controller) = self.controller.take() {
            result = result.and(controller.finish().await);
        }
        result
    }

    async fn reopen(&mut self) -> Result<(), Failure> {
        let previous = self.proxy.counts()?.connections;
        self.stop_processes().await?;
        self.start(false).await?;
        self.connected(previous + 1).await
    }

    async fn finish(&mut self) -> Result<(), Failure> {
        let result = self.stop_processes().await;
        result.and(self.proxy.finish().await)
    }

    fn request(&self, operation: &str) -> Value {
        json!({"operation_id":operation,"version":self.fixture.desired.release.plugin_version,
            "digest":self.fixture.desired.release.artifact_digest})
    }
}
