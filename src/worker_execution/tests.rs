use super::*;
use crate::execution_environment::{
    EnvironmentLocation, ExecutionAccess, RuntimeLocation, WorkspaceLocation,
};
#[cfg(feature = "machine-host")]
use crate::machine_protocol::execution::{Action, Request};

#[cfg(feature = "machine-host")]
mod native_backpressure;
mod transport;

fn binding() -> BindingV1 {
    BindingV1 {
        schema: 1,
        id: "binding".into(),
        revision: 1,
        runtime: RuntimeLocation {
            machine_id: "runtime".into(),
            cwd: "/runtime".into(),
        },
        environment: EnvironmentLocation {
            machine_id: "target".into(),
            id: "environment".into(),
            incarnation: "incarnation".into(),
            executor_digest: format!("sha256:{}", "a".repeat(64)),
            protocol: 1,
        },
        workspace: WorkspaceLocation {
            id: "project".into(),
            worktree_id: "session".into(),
            source_path: "/source".into(),
            cwd: "/target".into(),
        },
        access: ExecutionAccess::Project,
    }
}

#[tokio::test]
async fn reconnect_preserves_effect_identity_and_fences_replies() {
    let (notify, mut notified) = mpsc::unbounded_channel();
    let client = Client::new("session".into(), "worker".into(), binding(), notify);
    let caller = Arc::clone(&client);
    let call = tokio::spawn(async move {
        caller
            .invoke(
                "fs/writeFile".into(),
                json!({"path": "file:///target/file", "data": "secret-source-sentinel"}),
            )
            .await
    });
    notified.recv().await.unwrap();
    let initial = client.frames(false);
    assert_eq!(initial.len(), 1);
    let Frame::ExecutionRequest { request } = &initial[0] else {
        panic!("execution request")
    };
    assert!(client.frames(false).is_empty());
    assert_eq!(client.frames(true), initial);
    let mut reply = RuntimeReply {
        session_id: "session".into(),
        worker_epoch: "old-worker".into(),
        request_id: request.request_id.clone(),
        scope: Scope::from_binding(&request.binding),
        response: MachineResponse::Call {
            response: Response::Operation {
                outcome: Outcome::Completed {
                    reply: json!({"result": {}}),
                },
            },
        },
    };
    client.complete(reply.clone());
    assert!(!call.is_finished());
    reply.worker_epoch = "worker".into();
    reply.scope.revision = 2;
    client.complete(reply.clone());
    assert!(!call.is_finished());
    reply.scope.revision = 1;
    let complete = reply.clone();
    reply.response = MachineResponse::Refused {
        reason: Refusal::Unavailable,
    };
    client.complete(reply);
    assert_eq!(client.frames(true), initial);
    client.complete(complete);
    assert_eq!(call.await.unwrap().unwrap(), json!({"result": {}}));
    assert!(client.frames(true).is_empty());
    assert!(!format!("{:?}", initial[0]).contains("secret-source-sentinel"));
}

#[cfg(feature = "machine-host")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires pinned native binaries and an isolated PID/network namespace"]
async fn native_worker_execution() {
    use crate::machine_cli::execution::{Configuration, Manager};
    use sha2::{Digest as _, Sha256};

    let input: Value = serde_json::from_slice(
        &std::fs::read(std::env::var("COWBOY_TEST_EXECUTION_INPUT").unwrap()).unwrap(),
    )
    .unwrap();
    let input_path = |name| PathBuf::from(input[name].as_str().unwrap());
    let root = tempfile::tempdir().unwrap();
    let runtime = root.path().join("runtime");
    let source = root.path().join("source");
    std::fs::create_dir(&runtime).unwrap();
    std::fs::create_dir(&source).unwrap();
    std::fs::write(
        runtime.join("AGENTS.md"),
        "RUNTIME_GUIDANCE_MUST_NOT_REACH_MODEL",
    )
    .unwrap();
    std::fs::write(runtime.join("fixture.txt"), "runtime remains untouched\n").unwrap();
    std::fs::write(source.join("AGENTS.md"), "TARGET_GUIDANCE_MUST_REACH_MODEL").unwrap();
    std::fs::write(source.join("fixture.txt"), "target before\n").unwrap();
    for args in [
        vec!["init", "--initial-branch=main"],
        vec!["add", "."],
        vec![
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@invalid",
            "commit",
            "-m",
            "fixture",
        ],
    ] {
        let status = tokio::process::Command::new("git")
            .args(args)
            .current_dir(&source)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .unwrap();
        assert!(status.success());
    }
    let original = input_path("native_cli");
    let digest = format!("{:x}", Sha256::digest(std::fs::read(&original).unwrap()));
    assert_eq!(digest, input["sha256"].as_str().unwrap());
    let owned = root.path().join("component/bin");
    std::fs::create_dir_all(&owned).unwrap();
    let executor = owned.join("codex");
    std::fs::copy(&original, &executor).unwrap();
    // Native code-mode tool calls need the helper from the same pinned package.
    let code_mode = original.parent().unwrap().join("codex-code-mode-host");
    if code_mode.is_file() {
        std::fs::copy(code_mode, owned.join("codex-code-mode-host")).unwrap();
    }
    // A cold native resume may select a sandboxed policy. Keep the pinned
    // package's sandbox helper: copying only the main executable makes that
    // path fail before it can exercise the remote execution binding.
    let resources = owned.parent().unwrap().join("codex-resources");
    std::fs::create_dir_all(&resources).unwrap();
    std::fs::copy(
        original
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("codex-resources/bwrap"),
        resources.join("bwrap"),
    )
    .unwrap();
    std::fs::copy(
        original
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("codex-package.json"),
        owned.parent().unwrap().join("codex-package.json"),
    )
    .unwrap();
    let service = format!("svc-{}", "1".repeat(32));
    // Deliberately exceed sockaddr_un's limit in durable state.
    let state = root.path().join("long-machine-state-directory".repeat(4));
    let configuration = Configuration {
        schema: 1,
        retention: None,
        host_command: input_path("keeper"),
        executor: wire::Executor {
            command: executor.display().to_string(),
            sha256: digest,
            version: input["version"].as_str().unwrap().to_owned(),
        },
    };
    let manager = Arc::new(
        Manager::new(
            Some(service.clone()),
            "target".into(),
            &state,
            Some(configuration.clone()),
            false,
        )
        .unwrap(),
    );
    let workspaces = vec![crate::machine_protocol::MachineWorkspace {
        id: "fixture".into(),
        display_name: "Fixture".into(),
        canonical_path: source.display().to_string(),
    }];
    let prepare = Request {
        service_id: service.clone(),
        machine_id: "target".into(),
        action: Action::Prepare {
            session_id: "session".into(),
            workspace_id: "fixture".into(),
            runtime: RuntimeLocation {
                machine_id: "runtime".into(),
                cwd: runtime.display().to_string(),
            },
        },
    };
    let MachineResponse::Prepared { binding } = manager.request(prepare.clone(), &workspaces).await
    else {
        panic!("target preparation failed")
    };
    assert_ne!(binding.workspace.cwd, binding.workspace.source_path);
    // Machine restart reconnects to the same detached keeper and exact worktree.
    let restarted = Manager::new(
        Some(service.clone()),
        "target".into(),
        &state,
        Some(configuration),
        false,
    )
    .unwrap();
    assert_eq!(
        restarted.request(prepare, &workspaces).await,
        MachineResponse::Prepared {
            binding: binding.clone()
        }
    );
    let (notify, mut notified) = mpsc::unbounded_channel();
    let client = Client::new("session".into(), "worker".into(), binding.clone(), notify);
    let endpoint = Endpoint::start(Arc::clone(&client), &runtime)
        .await
        .unwrap();
    let relay_client = Arc::clone(&client);
    let relay_manager = Arc::clone(&manager);
    let relay_service = service.clone();
    let outage = Arc::new(parking_lot::Mutex::new(None::<tokio::time::Instant>));
    let relay_outage = Arc::clone(&outage);
    let write_operation = Arc::new(parking_lot::Mutex::new(None::<String>));
    let write_reply_lost = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let relay_write_reply_lost = Arc::clone(&write_reply_lost);
    let image_read = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let relay_image_read = Arc::clone(&image_read);
    let codeact_calls = Arc::new(std::sync::atomic::AtomicU8::new(0));
    let relay_codeact_calls = Arc::clone(&codeact_calls);
    let slow_events = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let relay_slow_events = Arc::clone(&slow_events);
    let event_gaps = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let relay_event_gaps = Arc::clone(&event_gaps);
    let claude_reconnect = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let relay_claude_reconnect = Arc::clone(&claude_reconnect);
    let file_helpers = Arc::new(std::sync::atomic::AtomicU8::new(0));
    let relay_file_helpers = Arc::clone(&file_helpers);
    let relay = tokio::spawn(async move {
        let mut timer = tokio::time::interval(Duration::from_millis(50));
        let mut tasks = tokio::task::JoinSet::new();
        loop {
            tokio::select! { _ = notified.recv() => {}, _ = timer.tick() => {}, _ = tasks.join_next(), if !tasks.is_empty() => {} }
            for frame in relay_client.frames(false) {
                // The production wire codecs, target Manager, private IPC and
                // native keeper all run; enrollment/Service authorization is
                // deliberately a separate connected gate.
                let Frame::ExecutionRequest { request } =
                    serde_json::from_slice::<Frame>(&serde_json::to_vec(&frame).unwrap()).unwrap()
                else {
                    unreachable!()
                };
                let manager = Arc::clone(&relay_manager);
                let client = Arc::clone(&relay_client);
                let service = relay_service.clone();
                let outage = Arc::clone(&relay_outage);
                let slow_events = Arc::clone(&relay_slow_events);
                let event_gaps = Arc::clone(&relay_event_gaps);
                let write_reply_lost = Arc::clone(&relay_write_reply_lost);
                let claude_reconnect = Arc::clone(&relay_claude_reconnect);
                if let Command::Invoke { invocation, .. } = &request.command {
                    if invocation.method == "process/start"
                        && invocation.params["argv"][0]
                            .as_str()
                            .is_some_and(|path| path.ends_with("/cowboy-execution-host"))
                    {
                        let bit = match invocation.params["argv"][1].as_str() {
                            Some("snapshot") => 1,
                            Some("read-range") => 2,
                            _ => 0,
                        };
                        relay_file_helpers.fetch_or(bit, std::sync::atomic::Ordering::Relaxed);
                    }
                    let params = invocation.params.to_string();
                    let filename = invocation.params["path"]
                        .as_str()
                        .and_then(|path| std::path::Path::new(path).file_name());
                    if invocation.method == "fs/writeFile"
                        && filename.is_some_and(|name| name == "lost-write-receipt.txt")
                    {
                        let mut retained = write_operation.lock();
                        if let Some(id) = retained.as_ref() {
                            assert_eq!(
                                id, &invocation.operation_id,
                                "lost write must retain its operation identity"
                            );
                        } else {
                            *retained = Some(invocation.operation_id.clone());
                        }
                    }
                    if invocation.method == "fs/readFile"
                        && filename.is_some_and(|name| name == "pixel.png")
                    {
                        relay_image_read.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    if invocation.method == "process/start"
                        && params.contains("native_codeact_expected_failure")
                    {
                        relay_codeact_calls.fetch_or(1, std::sync::atomic::Ordering::Relaxed);
                    }
                    if invocation.method == "fs/readFile"
                        && filename.is_some_and(|name| name == "codeact-pixel.png")
                    {
                        relay_codeact_calls.fetch_or(2, std::sync::atomic::Ordering::Relaxed);
                    }
                    if invocation.method == "process/start" {
                        for (marker, bit) in [
                            ("native-child-none.txt", 4),
                            ("native-child-all.txt", 8),
                            ("native-child-acp.txt", 16),
                        ] {
                            if params.contains(marker) {
                                relay_codeact_calls
                                    .fetch_or(bit, std::sync::atomic::Ordering::Relaxed);
                            }
                        }
                    }
                }
                let write_reply = match &request.command {
                    Command::Invoke { invocation, .. } => {
                        write_operation.lock().as_ref() == Some(&invocation.operation_id)
                    }
                    Command::Observe { operation_id, .. } => {
                        write_operation.lock().as_ref() == Some(operation_id)
                    }
                    _ => false,
                };
                tasks.spawn(async move {
                    let event_poll = matches!(request.command, Command::Events { .. });
                    if matches!(request.command, Command::Events { .. }) && slow_events.load(std::sync::atomic::Ordering::Relaxed) {
                        tokio::time::sleep(Duration::from_millis(500)).await;
                    }
                    let until = *outage.lock();
                    if let Some(until) = until { tokio::time::sleep_until(until).await; }
                    let trigger = matches!(&request.command, Command::Invoke { invocation, .. } if invocation.method == "process/start" && invocation.params.to_string().contains("background_started"));
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
                    let reconnect_marker = std::path::Path::new(&request.binding.workspace.cwd)
                        .join("claude-reconnect-request");
                    if event_poll && reconnect_marker.exists()
                        && !claude_reconnect.swap(true, std::sync::atomic::Ordering::SeqCst)
                    {
                        // Exercise the real endpoint's close path. The next
                        // Claude tool call must resume this exact keeper;
                        // neither the native process nor its history restarts.
                        client.complete(RuntimeReply {
                            session_id: request.session_id,
                            worker_epoch: request.worker_epoch,
                            request_id: request.request_id,
                            scope: Scope::from_binding(&request.binding),
                            response: MachineResponse::Call {
                                response: Response::Refused { reason: wire::Refusal::CursorExpired },
                            },
                        });
                        std::fs::write(reconnect_marker.with_file_name("claude-reconnect-closed"), "closed").unwrap();
                        return;
                    }
                    if write_reply
                        && matches!(&response, MachineResponse::Call { response: Response::Operation { outcome: Outcome::Completed { reply } } } if reply.get("error").is_none())
                        && !write_reply_lost.swap(true, std::sync::atomic::Ordering::SeqCst)
                    {
                        let path = std::path::Path::new(&request.binding.workspace.cwd).join("lost-write-receipt.txt");
                        assert_eq!(std::fs::read_to_string(&path).unwrap(), "native-write-before-loss\n");
                        // An independent writer changes the target after the
                        // original effect. Replaying that effect would erase
                        // this marker. Drop only its real completed response.
                        std::fs::write(path, "external-write-after-commit\n").unwrap();
                        return;
                    }
                    if trigger && outage.lock().is_none() {
                        *outage.lock() = Some(tokio::time::Instant::now() + Duration::from_secs(35));
                        // Lose the admitted start receipt. The worker must
                        // resend the same effect identity after reconnection.
                        return;
                    }
                    let until = *outage.lock();
                    if let Some(until) = until { tokio::time::sleep_until(until).await; }
                    // A delayed duplicate poll can name already-acknowledged
                    // history. Client fences that obsolete request; only an
                    // active poll's refusal can disconnect the endpoint.
                    if matches!(response, MachineResponse::Call { response: Response::Refused { reason: wire::Refusal::CursorExpired } }) && client.pending.lock().contains_key(&request.request_id) {
                        event_gaps.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    }
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
    let legacy = input["legacy_event_gap"] == true;
    if !legacy {
        native_backpressure::check(&endpoint, &binding).await;
        slow_events.store(false, std::sync::atomic::Ordering::Relaxed);
    }
    let status = tokio::process::Command::new("python3")
        .arg(if input["provider"] == "claude-code" {
            "tools/execution_claude_worker_conformance.py"
        } else {
            "tools/execution_worker_conformance.py"
        })
        .arg("--native-cli")
        .arg(&executor)
        .arg("--descriptor")
        .arg(endpoint.descriptor())
        .arg("--runtime")
        .arg(&runtime)
        .arg("--target")
        .arg(&binding.workspace.cwd)
        .arg("--receipt")
        .arg(std::env::var("COWBOY_TEST_EXECUTION_RECEIPT").unwrap())
        .status()
        .await
        .unwrap();
    relay.abort();
    drop(endpoint);
    let alias = PathBuf::from(format!(
        "/tmp/cowboy-execution-{}",
        rustix::process::geteuid().as_raw()
    ))
    .join(&binding.environment.incarnation);
    assert!(status.success(), "native worker execution failed");
    if input["require_file_helper"] == true {
        assert_eq!(
            file_helpers.load(std::sync::atomic::Ordering::Relaxed),
            3,
            "packaged Claude must use Cowboy snapshot and range helpers through the real target transport"
        );
    }
    if input["provider"] == "claude-code" {
        assert!(
            claude_reconnect.load(std::sync::atomic::Ordering::SeqCst),
            "native Claude must cross the closed endpoint fixture"
        );
        assert!(
            write_reply_lost.load(std::sync::atomic::Ordering::SeqCst),
            "native write must cross the lost completion fixture"
        );
    }
    let gaps = event_gaps.load(std::sync::atomic::Ordering::Relaxed);
    if legacy {
        assert_eq!(
            gaps, 1,
            "legacy cursor must expire once, without a reconnect loop"
        );
    } else {
        assert_eq!(
            gaps, 0,
            "bounded backpressure must retain unacknowledged events"
        );
    }
    assert!(
        image_read.load(std::sync::atomic::Ordering::Relaxed),
        "native image reads must cross the target transport"
    );
    if input["provider"] != "claude-code" {
        assert_eq!(
            codeact_calls.load(std::sync::atomic::Ordering::Relaxed),
            if input["adapter_launcher"].is_string() {
                31
            } else {
                15
            },
            "CodeAct shell/image and both native child commands must cross the target transport"
        );
    }
    assert!(
        outage.lock().is_some(),
        "real native start must cross injected outage"
    );
    assert_eq!(
        std::fs::read_to_string(source.join("fixture.txt")).unwrap(),
        "target before\n"
    );
    let request = Request {
        service_id: service.clone(),
        machine_id: "target".into(),
        action: Action::Close {
            session_id: "session".into(),
            binding: Box::new(binding.clone()),
        },
    };
    assert_eq!(
        manager.request(request.clone(), &workspaces).await,
        MachineResponse::Closed
    );
    assert_eq!(
        manager.request(request, &workspaces).await,
        MachineResponse::Closed
    );
    assert!(
        std::path::Path::new(&binding.workspace.cwd)
            .join("fixture.txt")
            .is_file()
    );
    assert!(matches!(
        manager
            .request(
                Request {
                    service_id: service,
                    machine_id: "target".into(),
                    action: Action::Prepare {
                        session_id: "session".into(),
                        workspace_id: "fixture".into(),
                        runtime: binding.runtime
                    }
                },
                &workspaces
            )
            .await,
        MachineResponse::Refused {
            reason: Refusal::EnvironmentLost
        }
    ));
    let _ = std::fs::remove_file(alias);
    let receipt_path = std::env::var("COWBOY_TEST_EXECUTION_RECEIPT").unwrap();
    let mut receipt: Value =
        serde_json::from_slice(&std::fs::read(&receipt_path).unwrap()).unwrap();
    if !legacy {
        receipt["checks"].as_array_mut().unwrap().extend([
            json!("slow_consumer_preserves_9_mib_stdout_stderr_and_terminal_events"),
            json!("concurrent_large_input_and_output_do_not_deadlock"),
            json!("post_flood_commands_and_cancellation_keep_original_executor"),
        ]);
    }
    receipt["checks"].as_array_mut().unwrap().extend([
        json!("target_image_bytes_cross_machine_transport"),
        json!("idempotent_close_stops_environment_and_preserves_worktree"),
        json!("closed_environment_cannot_be_recreated"),
    ]);
    receipt["cursor_expirations"] = gaps.into();
    if input["provider"] != "claude-code" {
        receipt["checks"].as_array_mut().unwrap().push(json!(
            "native_codeact_shell_and_image_cross_target_transport"
        ));
        receipt["checks"].as_array_mut().unwrap().push(json!(
            "native_fresh_and_forked_children_cross_target_transport"
        ));
        if input["adapter_launcher"].is_string() {
            receipt["checks"]
                .as_array_mut()
                .unwrap()
                .push(json!("packaged_acp_child_crosses_target_transport"));
        }
    }
    receipt["accepted"] = true.into();
    std::fs::write(&receipt_path, serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
}
