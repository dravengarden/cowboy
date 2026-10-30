//! Packaged managed Claude and Codex image turns against a Machine-prepared
//! snapshot and keeper-enforced read-only environment through worker transport.

use super::*;
use crate::machine_cli::execution::{Configuration, Manager};
use crate::machine_protocol::execution::{Action, Request};
use sha2::{Digest as _, Sha256};

async fn git(directory: &std::path::Path, args: &[&str]) {
    let status = tokio::process::Command::new("git")
        .args(args)
        .current_dir(directory)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires pinned native binaries and an isolated PID/network namespace"]
async fn native_managed_claude_execution() {
    native_managed_execution(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires pinned native binaries and an isolated PID/network namespace"]
async fn native_managed_image_execution() {
    native_managed_execution(true).await;
}

async fn native_managed_execution(images: bool) {
    let input: Value = serde_json::from_slice(
        &std::fs::read(std::env::var("COWBOY_TEST_EXECUTION_INPUT").unwrap()).unwrap(),
    )
    .unwrap();
    let input_path = |name: &str| PathBuf::from(input[name].as_str().unwrap());
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("runtime");
    let source = root.path().join("source");
    std::fs::create_dir(&runtime).unwrap();
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("fixture.txt"), "snapshot fixture\n").unwrap();
    if images {
        assert_eq!(
            format!(
                "{:x}",
                Sha256::digest(std::fs::read(input_path("provider_cli")).unwrap())
            ),
            input["provider_sha256"].as_str().unwrap(),
            "native image CLI differs from the accepted input"
        );
        let mut bytes = Vec::new();
        let mut writer = png::Encoder::new(&mut bytes, 1, 1).write_header().unwrap();
        writer.write_image_data(&[1]).unwrap();
        drop(writer);
        std::fs::write(source.join("input.png"), bytes).unwrap();
    }
    git(&source, &["init", "--initial-branch=main"]).await;
    git(&source, &["add", "."]).await;
    git(
        &source,
        &[
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@invalid",
            "commit",
            "-m",
            "fixture",
        ],
    )
    .await;
    // An uncommitted change is part of the reviewed snapshot.
    std::fs::write(
        source.join("fixture.txt"),
        "snapshot fixture\nworking tree\n",
    )
    .unwrap();
    // The executor keeps its pinned package layout, including the sandbox
    // helper the keeper relies on for read-only processes.
    let original = input_path("native_cli");
    let digest = format!("{:x}", Sha256::digest(std::fs::read(&original).unwrap()));
    assert_eq!(digest, input["sha256"].as_str().unwrap());
    let owned = root.path().join("component/bin");
    std::fs::create_dir_all(&owned).unwrap();
    let executor = owned.join("codex");
    std::fs::copy(&original, &executor).unwrap();
    let package = original.parent().unwrap().parent().unwrap();
    let resources = owned.parent().unwrap().join("codex-resources");
    std::fs::create_dir_all(&resources).unwrap();
    std::fs::copy(
        package.join("codex-resources/bwrap"),
        resources.join("bwrap"),
    )
    .unwrap();
    std::fs::copy(
        package.join("codex-package.json"),
        owned.parent().unwrap().join("codex-package.json"),
    )
    .unwrap();
    let service = format!("svc-{}", "2".repeat(32));
    let state = root.path().join("state");
    let manager = Arc::new(
        Manager::new(
            Some(service.clone()),
            "target".into(),
            &state,
            Some(Configuration {
                schema: 1,
                retention: None,
                host_command: input_path("keeper"),
                executor: wire::Executor {
                    command: executor.display().to_string(),
                    sha256: digest,
                    version: input["version"].as_str().unwrap().to_owned(),
                },
            }),
            false,
        )
        .unwrap(),
    );
    let workspaces = vec![crate::machine_protocol::MachineWorkspace {
        id: "fixture".into(),
        display_name: "Fixture".into(),
        canonical_path: source.display().to_string(),
    }];
    let request = |action| Request {
        service_id: service.clone(),
        machine_id: "target".into(),
        action,
    };
    let schema = (!images).then(|| {
        json!({
            "type": "object", "additionalProperties": false, "required": ["verdict"],
            "properties": {"verdict": {"type": "string", "enum": ["approve", "needs-attention"]}},
        })
    });
    let profile = if images {
        cowboy_provider_sdk::ManagedRuntimeProfile::ImageV1
    } else {
        cowboy_provider_sdk::ManagedRuntimeProfile::ReadOnlyV1
    };
    let round = crate::managed_calls::protocol::ChildRound {
        child_session_id: "child".into(),
        parent_session_id: "parent".into(),
        call_id: "call-1".into(),
        workspace_id: "fixture".into(),
        source_cwd: source.display().to_string(),
        files: if images {
            vec!["input.png".into()]
        } else {
            vec![]
        },
        output_schema: schema.clone(),
        profile,
    };
    let prepared = match manager
        .request(
            request(Action::PrepareManagedRound {
                round: Box::new(round),
            }),
            &workspaces,
        )
        .await
    {
        MachineResponse::ManagedRound { prepared } => prepared,
        MachineResponse::ManagedRoundRefused { code } => {
            panic!("managed round preparation refused: {code}")
        }
        _ => panic!("managed round preparation failed"),
    };
    let environment = Action::PrepareManagedEnvironment {
        child_session_id: "child".into(),
        runtime: RuntimeLocation {
            machine_id: "runtime".into(),
            cwd: runtime.display().to_string(),
        },
    };
    let MachineResponse::Prepared { binding } = manager
        .request(request(environment.clone()), &workspaces)
        .await
    else {
        panic!("managed environment preparation failed")
    };
    assert_eq!(binding.workspace.cwd, prepared.cwd);
    assert_eq!(
        binding
            .managed
            .as_ref()
            .map(|managed| managed.parent_session_id.as_str()),
        Some("parent")
    );
    // Repeating the request observes the same environment.
    assert_eq!(
        manager.request(request(environment), &workspaces).await,
        MachineResponse::Prepared {
            binding: binding.clone()
        }
    );
    // A managed environment can never be reopened as an ordinary one.
    assert!(matches!(
        manager
            .request(
                request(Action::Prepare {
                    session_id: "child".into(),
                    workspace_id: "fixture".into(),
                    runtime: binding.runtime.clone(),
                    base_session_id: None,
                }),
                &workspaces,
            )
            .await,
        MachineResponse::Refused { .. }
    ));
    let (notify, mut notified) = mpsc::unbounded_channel();
    let client = Client::new("child".into(), "worker".into(), binding.clone(), notify);
    let endpoint = Endpoint::start(Arc::clone(&client), &runtime)
        .await
        .unwrap();
    let relay_client = Arc::clone(&client);
    let relay_manager = Arc::clone(&manager);
    let relay_service = service.clone();
    let remote_image_read = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observed_image_read = Arc::clone(&remote_image_read);
    let relay = tokio::spawn(async move {
        let mut timer = tokio::time::interval(Duration::from_millis(50));
        let mut tasks = tokio::task::JoinSet::new();
        loop {
            tokio::select! { _ = notified.recv() => {}, _ = timer.tick() => {}, _ = tasks.join_next(), if !tasks.is_empty() => {} }
            for frame in relay_client.frames(false) {
                let Frame::ExecutionRequest { request } =
                    serde_json::from_slice::<Frame>(&serde_json::to_vec(&frame).unwrap()).unwrap()
                else {
                    unreachable!()
                };
                if let wire::Command::Invoke { invocation, .. } = &request.command
                    && invocation.method == "fs/readFile"
                    && invocation.params["path"]
                        .as_str()
                        .is_some_and(|path| path.ends_with("/input.png"))
                {
                    observed_image_read.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                let manager = Arc::clone(&relay_manager);
                let client = Arc::clone(&relay_client);
                let service = relay_service.clone();
                tasks.spawn(async move {
                    let response = manager
                        .request(
                            Request {
                                service_id: service,
                                machine_id: "target".into(),
                                action: Action::Call {
                                    session_id: request.session_id.clone(),
                                    binding: Box::new(request.binding.clone()),
                                    command: request.command,
                                },
                            },
                            &[],
                        )
                        .await;
                    client.complete(RuntimeReply {
                        session_id: request.session_id,
                        worker_epoch: request.worker_epoch,
                        request_id: request.request_id,
                        scope: Scope::from_binding(&request.binding),
                        response,
                    });
                });
            }
        }
    });
    // The runtime worker reads the round the target keeper announces.
    let marker = client.managed_round(profile).await.unwrap();
    assert_eq!(marker.call_id, "call-1");
    assert_eq!(marker.output_schema, schema);
    let mut command = tokio::process::Command::new("python3");
    // Cargo runs this crate's tests from components/machine; the harnesses
    // read repository paths such as plugins/codex/provider.json.
    command.current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."));
    command.arg(if images {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tools/execution_managed_image_conformance.py"
        )
    } else {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tools/execution_claude_managed_conformance.py"
        )
    });
    if images {
        command
            .arg("--native-cli")
            .arg(input_path("provider_cli"))
            .arg("--launcher")
            .arg(input_path("adapter_launcher"));
    }
    let receipt = std::env::var("COWBOY_TEST_EXECUTION_RECEIPT").unwrap();
    let status = command
        .arg("--descriptor")
        .arg(endpoint.descriptor())
        .arg("--runtime")
        .arg(&runtime)
        .arg("--target")
        .arg(&binding.workspace.cwd)
        .arg("--receipt")
        .arg(&receipt)
        .status()
        .await
        .unwrap();
    relay.abort();
    drop(endpoint);
    assert!(status.success(), "managed native execution failed");
    if images {
        use crate::managed_calls::media::{self, Delivery};
        use base64::Engine as _;
        assert!(
            remote_image_read.load(std::sync::atomic::Ordering::Relaxed),
            "native image input bypassed the execution Machine"
        );
        let report: Value = serde_json::from_slice(&std::fs::read(&receipt).unwrap()).unwrap();
        let artifacts =
            crate::artifacts::ArtifactStore::new(root.path().join("artifacts")).unwrap();
        for update in report["native_images"].as_array().unwrap() {
            for artifact in media::collect(update, &artifacts).unwrap() {
                let bytes = std::fs::read(artifacts.path(&artifact.id).unwrap()).unwrap();
                let action = Action::DeliverManagedArtifact {
                    delivery: Box::new(Delivery {
                        parent_session_id: "parent".into(),
                        child_session_id: "child".into(),
                        call_id: "call-1".into(),
                        artifact,
                        data: base64::engine::general_purpose::STANDARD.encode(&bytes),
                    }),
                };
                let MachineResponse::ManagedArtifact { path } =
                    manager.request(request(action.clone()), &workspaces).await
                else {
                    panic!("image delivery failed")
                };
                assert_eq!(std::fs::read(&path).unwrap(), bytes);
                assert_eq!(
                    manager.request(request(action), &workspaces).await,
                    MachineResponse::ManagedArtifact { path }
                );
            }
        }
    }
    // The source repository is untouched by the reviewed turn.
    assert_eq!(
        std::fs::read_to_string(source.join("fixture.txt")).unwrap(),
        "snapshot fixture\nworking tree\n"
    );
    assert!(!source.join("managed-probe.txt").exists());
    // Closing the child stops its keeper and removes the snapshot.
    assert_eq!(
        manager
            .request(
                request(Action::CloseManagedChild {
                    child_session_id: "child".into(),
                }),
                &workspaces,
            )
            .await,
        MachineResponse::Closed
    );
    assert!(!std::path::Path::new(&binding.workspace.cwd).exists());
    if images {
        let mut report: Value = serde_json::from_slice(&std::fs::read(&receipt).unwrap()).unwrap();
        report["checks"].as_array_mut().unwrap().extend(
            [
                "native_input_read_through_execution_machine",
                "verified_png_delivery_to_execution_machine",
                "idempotent_artifact_delivery",
                "source_unchanged",
                "managed_child_cleanup",
            ]
            .into_iter()
            .map(Value::from),
        );
        report["keeper_sha256"] = Value::from(format!(
            "{:x}",
            Sha256::digest(std::fs::read(input_path("keeper")).unwrap())
        ));
        std::fs::write(&receipt, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}
