//! Native-owned login. Credential files remain in the reserved runtime home.
use super::*;
use crate::provider_account::NativeLoginSpec;

pub(super) fn start(
    request: String,
    provider: String,
    spec: NativeLoginSpec,
    providers: Arc<MachinePluginStore>,
    events: tokio::sync::mpsc::UnboundedSender<MachineEvent>,
    sessions: LoginSessions,
) {
    let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    let (input_tx, input_rx) = tokio::sync::mpsc::channel(1);
    {
        let mut entries = sessions.lock();
        let std::collections::hash_map::Entry::Vacant(entry) = entries.entry(request.clone())
        else {
            let _ = events.send(MachineEvent::CommandResult {
                request_id: request,
                accepted: false,
                detail: Some("authentication request is already active".into()),
            });
            return;
        };
        entry.insert(LoginSession {
            cancel: cancel_tx,
            input: input_tx,
            candidate: None,
        });
    }
    tokio::spawn(async move {
        let mut login = LoginIo {
            cancel: cancel_rx,
            input: input_rx,
            sessions,
        };
        let result = run(&request, &provider, &spec, &providers, &events, &mut login).await;
        login.sessions.lock().remove(&request);
        let _ = events.send(MachineEvent::LoginState {
            request_id: request,
            provider,
            state: if result.is_ok() {
                AuthState::Pending
            } else {
                AuthState::Error
            },
            account_label: None,
            detail: Some(result.err().unwrap_or_else(|| {
                "native completion recorded locally; awaiting Controller admission".into()
            })),
        });
    });
}

async fn run(
    request: &str,
    provider: &str,
    spec: &NativeLoginSpec,
    providers: &MachinePluginStore,
    events: &tokio::sync::mpsc::UnboundedSender<MachineEvent>,
    login: &mut LoginIo,
) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + login_process::TIMEOUT;
    login_process::check(&login.cancel, deadline).map_err(|error| error.detail().to_owned())?;
    let prepared = providers
        .prepare_native_profile(provider, request, spec)
        .map_err(|_| {
            "native login preparation failed; reconcile any reserved profile".to_owned()
        })?;
    let cowboy_provider_sdk::AuthExecutor::CommandV1 {
        arguments,
        terminal,
        challenge,
        mut environment,
        preflight,
        ..
    } = prepared.method.executor
    else {
        return Err("native command login required".into());
    };
    apply_auth_preflight(&prepared.home, &preflight)
        .map_err(|_| "native login preflight failed; profile remains reserved".to_owned())?;
    for step in &preflight {
        if let cowboy_provider_sdk::AuthPreflight::EnvFileKeyRequiredV1 { key, .. } = step
            && let Ok(value) = std::env::var(key)
        {
            environment.entry(key.clone()).or_insert(value);
        }
    }
    let mut process = authentication_process(
        &prepared.command,
        &arguments,
        &environment,
        &terminal,
        &prepared.home,
    );
    process
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let containment = login_process::Containment::prepare(&mut process)
        .map_err(|_| "native login containment unavailable; profile remains reserved".to_owned())?;
    let child = login.spawn(request, deadline, &mut process)?;
    let _ = events.send(MachineEvent::LoginState {
        request_id: request.into(),
        provider: provider.into(),
        state: AuthState::Pending,
        account_label: None,
        detail: Some("starting native browser authorization".into()),
    });
    let mut url = None;
    let mut code = None;
    let mut sent = false;
    let expires_at_ms = unix_ms().saturating_add(login_process::TIMEOUT.as_millis() as i64);
    let status = login_process::run_contained(
        child,
        &mut login.cancel,
        &mut login.input,
        deadline,
        |line| {
            let (next_url, next_code) = login_challenge_tokens(&line);
            if next_url.is_some() {
                url = next_url;
            }
            if next_code.is_some() {
                code = next_code;
            }
            let device = challenge == cowboy_provider_sdk::AuthChallenge::DeviceCode;
            if !sent
                && (!device || code.is_some())
                && let Some(url) = url.clone()
            {
                let _ = events.send(MachineEvent::LoginChallenge {
                    request_id: request.into(),
                    provider: provider.into(),
                    verification_url: url,
                    user_code: code.clone(),
                    input_required: !device,
                    input_label: None,
                    secret_input: false,
                    expires_at_ms,
                });
                sent = true;
            }
        },
        containment,
    )
    .await;
    // run() returns Cleanup whenever the leader's wait is unconfirmed. All
    // other returned outcomes have reaped the leader, including cancellation.
    // Persist before checking cancellation or emitting a transient event.
    if !matches!(status, Err(login_process::Failure::Cleanup)) {
        providers
            .record_native_login_tree_exit(
                provider,
                spec.generation,
                request,
                matches!(&status, Ok(exit) if exit.success()),
            )
            .map_err(|_| {
                "native login exit persistence failed; reconciliation required".to_owned()
            })?;
    }
    let sessions = login.sessions.lock();
    let status = login_process::reconcile(
        status,
        sessions.contains_key(request),
        &login.cancel,
        deadline,
    )
    .map_err(|error| format!("{}; native profile remains reserved", error.detail()))?;
    if !status.success() {
        return Err("native login did not complete; profile remains reserved".into());
    }
    // Serialize completion with cancellation. Failure, task abortion and a
    // lost event all retain the native home; none rerun login or export tokens.
    let profile = providers
        .complete_native_profile(provider, spec.generation, request)
        .map_err(|_| "native identity verification failed; profile remains reserved".to_owned())?;
    events
        .send(MachineEvent::NativeProfileCompleted { profile })
        .map_err(|_| "Controller disconnected; native completion retained locally".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> NativeLoginSpec {
        NativeLoginSpec {
            generation: 1,
            method: "native".into(),
            provider_version: "fixture".into(),
            generation_digest: "sha256:fixture".into(),
        }
    }

    #[test]
    fn native_dispatch_requires_account_protocol_and_rejects_secret_fields() {
        let command = MachineCommand::BeginNativeLogin {
            request_id: "request".into(),
            provider: "claude-code".into(),
            account_id: uuid::Uuid::new_v4().to_string().try_into().unwrap(),
            spec: spec(),
        };
        assert_eq!(
            command.minimum_protocol(),
            crate::machine_protocol::PROVIDER_ACCOUNT_PROTOCOL_VERSION
        );
        let encoded = serde_json::to_value(command).unwrap();
        assert!(serde_json::from_value::<MachineCommand>(encoded).is_ok());
        let mut invalid = serde_json::to_value(spec()).unwrap();
        invalid["token"] = serde_json::json!("must not be accepted");
        assert!(serde_json::from_value::<NativeLoginSpec>(invalid).is_err());
    }

    #[tokio::test]
    async fn cancelled_native_login_never_prepares_or_dispatches() {
        let temp = tempfile::tempdir().unwrap();
        let root = MachinePluginStore::new(
            temp.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap();
        let store = root
            .for_account(&uuid::Uuid::new_v4().to_string().try_into().unwrap())
            .unwrap();
        let (_cancel_tx, cancel) = tokio::sync::watch::channel(true);
        let (_input_tx, input) = tokio::sync::mpsc::channel(1);
        let (events, mut received) = tokio::sync::mpsc::unbounded_channel();
        let mut login = LoginIo {
            cancel,
            input,
            sessions: Default::default(),
        };
        assert_eq!(
            run(
                "request",
                "claude-code",
                &spec(),
                &store,
                &events,
                &mut login
            )
            .await
            .unwrap_err(),
            "login cancelled"
        );
        assert!(received.try_recv().is_err());
    }
}
