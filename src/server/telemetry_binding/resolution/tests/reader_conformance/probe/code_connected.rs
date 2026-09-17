//! Real supplied Controller/Machine/Zed processes; no production state or grant.
//! Reuses only the isolated process, fixture identity and HTTP test helpers.
use super::connected::http::Http;
use super::*;
use serde_json::{Value, json};

mod exercise;
mod fixture;
mod installation;
mod proxy;
mod synchronization;

const SESSION: &str = "sess-901";
const TEXT: &str = "a🙂z\nowned native buffer\n";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    schema: u16,
    controller: PathBuf,
    machine: PathBuf,
    adapter: PathBuf,
    server: PathBuf,
}

#[derive(Serialize)]
struct Binary {
    path: PathBuf,
    sha256: String,
}

impl Binary {
    fn resolve(path: PathBuf) -> Result<Self> {
        ensure!(
            path.starts_with("/nix/store") && path.canonicalize()? == path,
            "immutable native input required"
        );
        let metadata = path.metadata()?;
        ensure!(
            metadata.is_file() && metadata.len() <= 512 * 1024 * 1024,
            "bounded native file required"
        );
        let bytes = std::fs::read(&path)?;
        ensure!(
            bytes.starts_with(b"\x7fELF") && bytes.len() <= 512 * 1024 * 1024,
            "bounded native ELF required"
        );
        Ok(Self {
            path,
            sha256: sha256(&bytes),
        })
    }
}

#[derive(Serialize)]
struct Receipt {
    schema: &'static str,
    source_revision: String,
    artifacts: Vec<Artifact>,
    native: [Binary; 2],
    core_adapter: Binary,
    ssh_keygen: manifest::Executable,
    git: manifest::Executable,
    package_sha256: Option<String>,
    release_sha256: Option<String>,
    stage: &'static str,
    checks: Vec<&'static str>,
    wire: proxy::Counts,
    last_http: Option<super::super::connected::HttpObservation>,
    failure: Option<Failure>,
    cleanup: bool,
    accepted: bool,
    not_checked: [&'static str; 7],
}

fn check(value: bool) -> Result<(), Failure> {
    if value {
        Ok(())
    } else {
        Err(Failure::WrongObservation)
    }
}

struct Pair<'a> {
    root: &'a Path,
    address: std::net::SocketAddr,
    artifacts: &'a [Artifact],
    reader: &'a Fixture,
    controller: Option<Running>,
    machine: Option<Running>,
    proxy: proxy::Proxy,
    http: Http,
}

impl Pair<'_> {
    async fn start_controller(&mut self) -> Result<(), Failure> {
        let mut command = configured_command(&self.artifacts[0], self.root, self.address);
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
            controller(self.controller.as_mut().unwrap(), self.address, self.reader),
        )
        .await
        .map_err(|_| Failure::Timeout)?
    }

    async fn connected(&self, minimum: u32) -> Result<(), Failure> {
        tokio::time::timeout(DEADLINE, async {
            loop {
                let counts = self.proxy.counts()?;
                if counts.connections >= minimum
                    && counts.configurations == counts.connections
                    && self
                        .http
                        .get(&format!("/api/machines/{MACHINE}/deployment-health"))
                        .await
                        .is_ok_and(|value| value["connected"] == true)
                {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .map_err(|_| Failure::Timeout)?
    }

    async fn finish(&mut self) -> Result<(), Failure> {
        // Native runtimes deliberately use their own groups. End the exact
        // fixture executables before killing Machine; do not leave orphans.
        let mut result = fixture::stop_native(self.root).await;
        eprintln!("Code fixture native cleanup: {result:?}");
        if let Some(mut process) = self.machine.take() {
            let stopped = process.finish_with_reaper(true).await;
            eprintln!("Code fixture Machine cleanup: {stopped:?}");
            result = result.and(stopped);
        }
        if let Some(mut process) = self.controller.take() {
            let stopped = process.finish_with_reaper(true).await;
            eprintln!("Code fixture Controller cleanup: {stopped:?}");
            result = result.and(stopped);
        }
        result = result.and(self.proxy.finish().await);
        result.and(fixture::reap_orphans().await)
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "just code-buffer-connected-conformance; immutable inputs and isolated PID/network namespaces"]
async fn immutable_connected_code_buffers() -> Result<()> {
    manifest::require_isolation()?;
    // PID namespace is the final safety net even if an assertion/process dies.
    let init = std::fs::read_link("/proc/1/exe")?;
    ensure!(
        init.starts_with("/nix/store") && init.file_name().is_some_and(|name| name == "cargo"),
        "private proc with Cargo as namespace init required"
    );
    ensure!(
        std::fs::read_dir("/sys/fs/cgroup")?.next().is_none()
            && std::fs::read_to_string("/proc/self/mountinfo")?
                .lines()
                .any(|line| {
                    let fields: Vec<_> = line.split_whitespace().collect();
                    fields.get(4) == Some(&"/sys/fs/cgroup")
                        && fields
                            .get(5)
                            .is_some_and(|options| options.split(',').any(|option| option == "ro"))
                        && line
                            .split_once(" - ")
                            .is_some_and(|(_, fs)| fs.starts_with("tmpfs "))
                }),
        "empty read-only cgroup cover required"
    );
    // Cargo is namespace init but does not reap native grandchildren. Adopt
    // them here and explicitly reap after their tracked parents have stopped.
    rustix::process::set_child_subreaper(rustix::process::Pid::from_raw(1))?;
    let input: Input = serde_json::from_slice(&std::fs::read(std::env::var(
        "COWBOY_TEST_CODE_CONNECTED_INPUT",
    )?)?)?;
    ensure!(input.schema == 1, "unsupported Code conformance input");
    let path = PathBuf::from(std::env::var("COWBOY_TEST_CODE_CONNECTED_RECEIPT")?);
    ensure!(
        path.is_absolute() && path.symlink_metadata().is_err(),
        "new absolute receipt required"
    );
    let core_adapter = Binary::resolve(
        input
            .machine
            .join("bin/cowboy-code-adapter")
            .canonicalize()?,
    )?;
    let mut receipt = Receipt {
        schema: "dravengarden.cowboy.code-buffer-connected-conformance/v3",
        source_revision: manifest::clean_revision()?,
        artifacts: manifest::supplied_pair(input.controller, input.machine)?,
        native: [
            Binary::resolve(input.adapter)?,
            Binary::resolve(input.server)?,
        ],
        core_adapter,
        ssh_keygen: manifest::ssh_keygen()?,
        git: manifest::tool("git")?,
        package_sha256: None,
        release_sha256: None,
        stage: "setup",
        checks: Vec::new(),
        wire: proxy::Counts::default(),
        last_http: None,
        failure: None,
        cleanup: false,
        accepted: false,
        not_checked: [
            "production_roles_policies_accounts_installation_and_activation",
            "review_integration_browser_storage_and_supported_devices",
            "nonempty_language_servers_or_atomic_diagnostics_freshness",
            "dirty_buffer_override_independent_restoration_and_owned_navigation_destinations",
            "abandoned_browser_restart_restoration_and_post_effect_recovery",
            "agent_authentication_sessions_and_native_worker_generation_upgrade",
            "physical_power_loss_general_graph_state_leases_and_refactor_completion",
        ],
    };
    let result = run(&mut receipt).await;
    receipt.failure = result.err();
    receipt.accepted = result.is_ok() && receipt.cleanup && receipt.checks.len() == 11;
    write_receipt(&path, &receipt)?;
    ensure!(
        receipt.accepted,
        "connected Code acceptance failed at {}; inspect bounded receipt",
        receipt.stage
    );
    Ok(())
}

async fn run(receipt: &mut Receipt) -> Result<(), Failure> {
    let root = tempfile::tempdir().map_err(|_| Failure::Setup)?;
    let reader = Fixture::build(Case::Absent)
        .await
        .map_err(|_| Failure::Setup)?;
    let seeded = fixture::seed(
        root.path(),
        &reader,
        &receipt.native,
        &receipt.ssh_keygen.path,
        &receipt.git.path,
        &receipt.core_adapter.path,
    )
    .await
    .map_err(|_| Failure::Setup)?;
    receipt.package_sha256 = Some(seeded.package_sha256.clone());
    receipt.release_sha256 = Some(seeded.release_sha256.clone());
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|_| Failure::Setup)?;
    let address = listener.local_addr().map_err(|_| Failure::Setup)?;
    let relay = proxy::Proxy::start(address).await?;
    let mut pair = Pair {
        root: root.path(),
        address,
        artifacts: &receipt.artifacts,
        reader: &reader,
        controller: None,
        machine: None,
        proxy: relay,
        http: Http::with_timeout(address, Duration::from_secs(100))?,
    };
    drop(listener);
    let result = tokio::time::timeout(Duration::from_secs(150), async {
        receipt.stage = "authentication";
        pair.start_controller().await?;
        pair.http
            .denied(
                reqwest::Method::POST,
                "/api/code/buffers",
                Some(json!({"sessionId":SESSION,"path":"fixture.txt"})),
            )
            .await?;
        pair.http.login(&seeded.password).await?;
        let mut command =
            configured_command(&receipt.artifacts[1], root.path(), pair.proxy.address);
        command
            .arg("--plugin-operation-admission")
            .arg("--code-adapter-socket")
            .arg(root.path().join("code.sock"));
        pair.machine = Some(Running::spawn(&mut command)?);
        pair.connected(1).await?;
        receipt.stage = "connected_installation";
        installation::run(&pair, &seeded.install).await?;
        receipt.checks.push("authenticated_code_installation");
        exercise::run(&mut pair, &mut receipt.stage, &mut receipt.checks).await
    })
    .await
    .unwrap_or(Err(Failure::Timeout));
    receipt.last_http = pair.http.last();
    let cleanup = pair.finish().await;
    receipt.wire = pair.proxy.snapshot();
    receipt.cleanup = cleanup.is_ok();
    pair.proxy.counts()?;
    result.and(cleanup)?;
    receipt.stage = "complete";
    Ok(())
}

#[test]
fn code_input_cannot_name_production_state_credentials_or_mutable_binaries() {
    let valid = json!({"schema":1,"controller":"c","machine":"m","adapter":"a","server":"s"});
    for field in [
        "url",
        "state_dir",
        "cookie",
        "environment",
        "policy",
        "credentials",
    ] {
        let mut value = valid.clone();
        value[field] = json!("forbidden");
        assert!(serde_json::from_value::<Input>(value).is_err());
    }
    for path in ["/tmp/adapter", "/run/cowboy-machine", "relative"] {
        assert!(Binary::resolve(PathBuf::from(path)).is_err());
    }
}
