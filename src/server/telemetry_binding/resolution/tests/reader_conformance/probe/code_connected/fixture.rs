use super::*;
use cowboy_plugin_sdk::{
    PLUGIN_RELEASE_SIGNATURE_NAMESPACE, PluginArtifactFormat, PluginArtifactProbe, PluginManifest,
    PluginPackage, PluginPayload, PluginRelease, PluginRuntimeArtifacts, ReleasedPluginComponent,
};

pub(super) struct Seeded {
    pub password: String,
    pub package_sha256: String,
    pub release_sha256: String,
    pub install: serde_json::Value,
    artifacts: tokio::task::JoinHandle<std::io::Result<()>>,
}

impl Drop for Seeded {
    fn drop(&mut self) {
        self.artifacts.abort();
    }
}

pub(super) async fn seed(
    root: &Path,
    reader: &Fixture,
    native: &[Binary; 2],
    helper: &Path,
    git: &Path,
    core_adapter: &Path,
) -> Result<Seeded> {
    super::super::seed(root, reader, helper).await?;
    std::fs::create_dir(root.join("machine/bootstrap"))?;
    std::os::unix::fs::symlink(
        core_adapter,
        root.join("machine/bootstrap/cowboy-code-adapter"),
    )?;
    std::os::unix::fs::symlink(git, root.join("tools/git"))?;
    std::fs::create_dir(root.join("git-template"))?;
    let mut init = command(git, root);
    init.env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .arg("init")
        .arg("--quiet")
        .arg("--initial-branch=main")
        .arg(format!(
            "--template={}",
            root.join("git-template").display()
        ))
        .arg(root.join("workspace"));
    ensure!(
        tokio::time::timeout(Duration::from_secs(3), init.status())
            .await??
            .success(),
        "fixture Git initialization failed"
    );
    let manifest: PluginManifest = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/plugins/zed/plugin.json"
    )))?;
    let contract: cowboy_plugin_sdk::CodeIntelligenceContract =
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/plugins/zed/contract.json"
        )))?;
    let package = PluginPackage::new(
        manifest.clone(),
        manifest.component_release.clone(),
        PluginPayload::CodeIntelligence(contract.clone()),
    )?;
    let bytes = package.canonical_bytes()?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let mut router = axum::Router::new();
    let mut components = Vec::new();
    for (index, (component, binary)) in contract
        .runtime
        .as_ref()
        .unwrap()
        .components
        .iter()
        .zip(native)
        .enumerate()
    {
        let data = axum::body::Bytes::from(std::fs::read(&binary.path)?);
        ensure!(sha256(&data) == binary.sha256, "native input changed");
        components.push(ReleasedPluginComponent {
            kind: component.kind,
            slot: component.slot.clone(),
            dependency: component.dependency.clone(),
            version: component.version.clone(),
            command: component.command.clone(),
            artifact_url: format!("http://{address}/{index}"),
            artifact_digest: format!("sha256:{}", binary.sha256),
            artifact_format: PluginArtifactFormat::Raw,
            entrypoint: None,
            probe: PluginArtifactProbe {
                args: vec![if index == 0 { "--help" } else { "version" }.into()],
                timeout_ms: 30_000,
            },
        });
        router = router.route(
            &format!("/{index}"),
            axum::routing::get(move || {
                let data = data.clone();
                async move { data }
            }),
        );
    }
    let publisher = crate::machine_auth::MachineIdentity::load_or_create(&root.join("publisher"))?;
    let mut release = PluginRelease {
        release_schema: 1,
        plugin_id: manifest.id,
        plugin_version: manifest.version,
        plugin_kind: manifest.kind,
        package_digest: PluginPackage::artifact_digest(&bytes),
        artifact_digest: String::new(),
        artifact_url: "https://example.invalid/zed.cowboy-plugin".into(),
        publisher: manifest.publisher,
        contract_fingerprint: package.contract_fingerprint,
        component_release: manifest.component_release,
        host_bundle_digest: None,
        signature: String::new(),
        supported_platforms: contract.supported_platforms.clone(),
        runtime_artifacts: vec![PluginRuntimeArtifacts {
            os: cowboy_provider_sdk::OperatingSystem::Linux,
            architecture: cowboy_provider_sdk::Architecture::X86_64,
            components,
        }],
    };
    release.artifact_digest = release.computed_artifact_digest()?;
    release.signature =
        publisher.sign_namespaced(PLUGIN_RELEASE_SIGNATURE_NAMESPACE, &release.proof())?;
    std::fs::create_dir_all(root.join("catalog/trusted-publishers"))?;
    private_write(
        &root
            .join("catalog/trusted-publishers")
            .join(format!("{}.pub", release.publisher)),
        publisher.public_key().as_bytes(),
    )?;
    private_write(&root.join("catalog/zed.cowboy-plugin"), &bytes)?;
    let release_bytes = serde_json::to_vec(&release)?;
    private_write(&root.join("catalog/zed.release.json"), &release_bytes)?;
    let machine = crate::machine_plugins::MachinePluginStore::new(
        &root.join("machine"),
        crate::machine_protocol::Platform::Linux,
        "x86_64".into(),
    )?;
    machine.enable_installation_tracking().await?;
    let password = super::super::super::connected::seed_operator(root, &machine).await?;
    private_write(&root.join("workspace/fixture.txt"), TEXT.as_bytes())?;
    root_identity::seed(root)?;
    navigation::seed(root)?;
    budget::seed(root)?;
    // Both source files belong to the initial native worktree scan.
    private_write(
        &root.join("workspace/sync.txt"),
        synchronization::ORIGINAL.as_bytes(),
    )?;
    // Seed a stopped owned Session before startup, never fake a native worker.
    let session: crate::core::SessionMeta = serde_json::from_value(json!({
        "id":SESSION,"provider":"codex","machine_id":MACHINE,"workspace_id":"fixture",
        "cwd":root.join("workspace"),"title":"Connected Code fixture","status":"exited",
        "owner_user_id":"c".repeat(32),"owner_username":"connected-operator"
    }))?;
    let store =
        crate::store::Store::connect(&database(root), root.join("controller/artifacts")).await?;
    store.insert_session(&session).await?;
    Ok(Seeded {
        password,
        package_sha256: sha256(&bytes),
        release_sha256: sha256(&release_bytes),
        install: json!({"operation_id":"connected-code-install",
            "version":release.plugin_version,"digest":release.artifact_digest}),
        // Only publish immutable bytes. The live Machine must stage, probe and
        // install them via actual authenticated Controller admission below.
        artifacts: tokio::spawn(async move { axum::serve(listener, router).await }),
    })
}

/// Test teardown only: exact installed executables confined to this private root.
/// This is NOT native release/recovery evidence. The PID namespace also contains
/// exceptional-path orphans without touching any resident Machine process.
pub(super) async fn stop_native(root: &Path) -> Result<(), Failure> {
    let mut pids = Vec::new();
    let mut directories = std::collections::BTreeSet::new();
    for entry in std::fs::read_dir("/proc").map_err(|_| Failure::Cleanup)? {
        let entry = entry.map_err(|_| Failure::Cleanup)?;
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() else {
            continue;
        };
        let Ok(exe) = std::fs::read_link(entry.path().join("exe")) else {
            continue;
        };
        if !exe.starts_with(root.join("machine")) {
            continue;
        }
        if let Ok(bytes) = std::fs::read(entry.path().join("cmdline")) {
            let arguments: Vec<_> = bytes.split(|byte| *byte == 0).collect();
            for pair in arguments.windows(2) {
                if pair[0] == b"--socket"
                    && let Ok(socket) = std::str::from_utf8(pair[1])
                    && let Some(directory) = native_directory(socket)
                {
                    directories.insert(directory);
                }
            }
        }
        let pid = rustix::process::Pid::from_raw(pid).ok_or(Failure::Cleanup)?;
        match rustix::process::kill_process(pid, rustix::process::Signal::KILL) {
            Ok(()) | Err(rustix::io::Errno::SRCH) => pids.push(pid),
            Err(_) => return Err(Failure::Cleanup),
        }
    }
    tokio::time::timeout(Duration::from_secs(4), async {
        while pids
            .iter()
            .any(|pid| std::fs::read_link(format!("/proc/{}/exe", pid.as_raw_pid())).is_ok())
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .map_err(|_| Failure::Cleanup)?;
    for directory in directories {
        let metadata = match std::fs::symlink_metadata(&directory) {
            Ok(metadata) => metadata,
            // A stopped probe may finish its own scoped directory cleanup
            // before this fixture's process termination observation settles.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(Failure::Cleanup),
        };
        check(metadata.is_dir() && !metadata.is_symlink())?;
        std::fs::remove_dir_all(directory).map_err(|_| Failure::Cleanup)?;
    }
    Ok(())
}

fn native_directory(socket: &str) -> Option<PathBuf> {
    let path = Path::new(socket);
    let directory = path.parent()?;
    let suffix = directory.file_name()?.to_str()?.strip_prefix("cw-code-")?;
    (path.file_name()? == "adapter.sock"
        && directory.parent()? == Path::new("/tmp")
        && suffix.len() == 32
        && suffix
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
    .then(|| directory.to_owned())
}

/// Called only after all fixture leaders have been waited. No live Tokio-owned
/// child remains; every adopted descendant belongs to this isolated test.
pub(super) async fn reap_orphans() -> Result<(), Failure> {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match rustix::process::wait(rustix::process::WaitOptions::NOHANG) {
                Ok(Some(_)) => continue,
                Err(rustix::io::Errno::CHILD) => return Ok(()),
                Ok(None) => tokio::time::sleep(Duration::from_millis(10)).await,
                Err(_) => return Err(Failure::Cleanup),
            }
        }
    })
    .await
    .map_err(|_| Failure::Cleanup)?
}

#[test]
fn teardown_cannot_select_broad_or_unrelated_directories() {
    for socket in [
        "/adapter.sock",
        "/tmp/adapter.sock",
        "/tmp/other/adapter.sock",
        "/tmp/cw-code-../adapter.sock",
        "/tmp/cw-code-00000000000000000000000000000000/../adapter.sock",
    ] {
        assert!(native_directory(socket).is_none());
    }
    assert!(
        native_directory("/tmp/cw-code-00000000000000000000000000000000/adapter.sock").is_some()
    );
}
