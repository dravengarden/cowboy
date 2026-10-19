use super::*;
use std::os::unix::process::CommandExt as _;
use std::process::Stdio;
use tokio::process::Command;

#[tokio::test]
async fn login_cleanup_requires_observed_leader_exit() {
    use std::os::unix::process::ExitStatusExt as _;
    let timeout = Duration::from_millis(10);
    assert_eq!(
        confirm_leader_exit(std::future::pending(), timeout).await,
        Err(Failure::Cleanup)
    );
    assert_eq!(
        confirm_leader_exit(async { Err(std::io::Error::other("wait failed")) }, timeout).await,
        Err(Failure::Cleanup)
    );
    // A nonzero native exit is still confirmed exit, not successful login.
    assert_eq!(
        confirm_leader_exit(async { Ok(ExitStatus::from_raw(9)) }, timeout).await,
        Ok(())
    );
    assert_ne!(Failure::Cleanup, Failure::Cancelled);
}

#[tokio::test]
async fn runaway_output_terminates_before_the_login_deadline() {
    let (_cancel_tx, mut cancel) = watch::channel(false);
    let (_input_tx, mut input) = mpsc::channel(1);
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        run(
            child("head -c 100000 /dev/zero; exec sleep 60"),
            &mut cancel,
            &mut input,
            Instant::now() + TIMEOUT,
            |_| panic!("oversized native output reached the challenge parser"),
        ),
    )
    .await
    .expect("oversized output was not stopped");
    assert_eq!(result, Err(Failure::Read));
}

#[test]
fn login_input_queue_has_backpressure() {
    let (login, _connection, _) = pending_login();
    let sessions = login.sessions.lock();
    let sender = &sessions.get("fixture").unwrap().input;
    sender.try_send("first".into()).unwrap();
    assert!(matches!(
        sender.try_send("second".into()),
        Err(mpsc::error::TrySendError::Full(_))
    ));
}

#[tokio::test]
async fn cancellation_reserves_request_until_task_cleanup_and_refuses_input() {
    let (mut login, _connection, _) = pending_login();
    assert!(super::super::submit_login_code(
        &login.sessions,
        "fixture",
        "before"
    ));
    assert!(super::super::request_login_cancel(
        &login.sessions,
        "fixture"
    ));
    assert!(super::super::request_login_cancel(
        &login.sessions,
        "fixture"
    ));
    assert!(!super::super::submit_login_code(
        &login.sessions,
        "fixture",
        "after"
    ));
    // Even queued input must lose to cancellation.
    assert_eq!(
        input(
            &mut login.cancel,
            &mut login.input,
            Instant::now() + TIMEOUT
        )
        .await,
        Err(Failure::Cancelled)
    );
    assert!(matches!(
        login.sessions.lock().entry("fixture".into()),
        std::collections::hash_map::Entry::Occupied(_)
    ));
    // Only the owning task's cleanup frees the reservation.
    login.sessions.lock().remove("fixture");
    assert!(!super::super::request_login_cancel(
        &login.sessions,
        "fixture"
    ));
}

#[test]
fn cancellation_does_not_forget_an_unconfirmed_abandoned_task() {
    let (login, _connection, _) = pending_login();
    let sessions = login.sessions.clone();
    drop(login);
    assert!(!super::super::request_login_cancel(&sessions, "fixture"));
    assert!(sessions.lock().contains_key("fixture"));
}

#[test]
fn disconnected_connection_cleans_queued_candidate_without_delivery() {
    let dir = tempfile::tempdir().unwrap();
    let root = std::sync::Arc::new(
        crate::machine_plugins::MachinePluginStore::new(
            dir.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap(),
    );
    let account = std::sync::Arc::new(
        root.for_account(&uuid::Uuid::new_v4().to_string().try_into().unwrap())
            .unwrap(),
    );
    let candidate_path = |store: &crate::machine_plugins::MachinePluginStore| {
        let path = store
            .auth_watch_root()
            .join("claude-code/candidates/fixture/home");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("credential"), b"synthetic").unwrap();
        path
    };
    let root_path = candidate_path(&root);
    let account_path = candidate_path(&account);
    let (login, connection, _) = pending_login();
    let mut task_guard = crate::machine_plugins::AuthCandidateCleanup::new(
        account.clone(),
        "claude-code",
        "fixture",
    );
    assert!(task_guard.matches("claude-code", account.account_id()));
    assert!(!task_guard.matches("claude-code", root.account_id()));
    assert!(!task_guard.matches("codex", account.account_id()));
    login.sessions.lock().get_mut("fixture").unwrap().candidate = Some(task_guard.transfer());
    drop(task_guard);
    assert!(account_path.exists());
    // The native task has finished; the outgoing candidate need never arrive.
    drop(login);
    drop(connection);
    assert!(!account_path.exists());
    assert!(root_path.join("credential").is_file());
}

#[test]
fn failed_candidate_cancellation_keeps_registration_until_cleanup_succeeds() {
    let dir = tempfile::tempdir().unwrap();
    let store = std::sync::Arc::new(
        crate::machine_plugins::MachinePluginStore::new(
            dir.path(),
            crate::machine_protocol::Platform::Linux,
            "x86_64".into(),
        )
        .unwrap(),
    );
    let candidate = store
        .auth_watch_root()
        .join("claude-code/candidates/fixture");
    std::fs::create_dir_all(candidate.parent().unwrap()).unwrap();
    // A damaged candidate path makes removal fail even when tests run as root.
    std::fs::write(&candidate, b"synthetic damaged reservation").unwrap();
    let untouched = candidate.parent().unwrap().join("other-request");
    std::fs::create_dir(&untouched).unwrap();
    let (login, _connection, _) = pending_login();
    login.sessions.lock().get_mut("fixture").unwrap().candidate = Some(
        crate::machine_plugins::AuthCandidateCleanup::new(store, "claude-code", "fixture"),
    );
    assert!(!super::super::request_login_cancel(
        &login.sessions,
        "fixture"
    ));
    assert!(
        login
            .sessions
            .lock()
            .get("fixture")
            .unwrap()
            .candidate
            .is_some()
    );
    assert!(candidate.is_file());
    assert!(!super::super::submit_login_code(
        &login.sessions,
        "fixture",
        "late-code"
    ));
    // Repair only this synthetic filesystem failure, then retry the same ID.
    std::fs::remove_file(&candidate).unwrap();
    std::fs::create_dir(&candidate).unwrap();
    assert!(super::super::request_login_cancel(
        &login.sessions,
        "fixture"
    ));
    assert!(!login.sessions.lock().contains_key("fixture"));
    assert!(!candidate.exists());
    assert!(untouched.is_dir());
}

fn child(script: &str) -> Child {
    let mut command = Command::new("sh");
    command
        .args(["-c", script])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command.as_std_mut().process_group(0);
    command.spawn().unwrap()
}

fn pending_login() -> (
    super::super::LoginIo,
    super::super::LoginConnection,
    watch::Sender<bool>,
) {
    let sessions = super::super::LoginSessions::default();
    let connection = super::super::LoginConnection(sessions.clone());
    let (cancel_tx, cancel) = watch::channel(false);
    let (input_tx, input) = mpsc::channel(1);
    sessions.lock().insert(
        "fixture".into(),
        super::super::LoginSession {
            cancel: cancel_tx.clone(),
            input: input_tx,
            candidate: None,
        },
    );
    (
        super::super::LoginIo {
            cancel,
            input,
            sessions,
        },
        connection,
        cancel_tx,
    )
}

#[tokio::test]
async fn revoked_or_expired_login_never_spawns_a_native_process() {
    for reason in ["disconnect", "cancel", "removed", "expired"] {
        let (login, connection, cancel) = pending_login();
        let mut connection = Some(connection);
        let mut deadline = Instant::now() + Duration::from_secs(30);
        match reason {
            "disconnect" => drop(connection.take()),
            "cancel" => cancel.send(true).unwrap(),
            "removed" => {
                login.sessions.lock().remove("fixture");
            }
            _ => deadline = Instant::now(),
        }
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("spawned");
        let mut command = Command::new("sh");
        command
            .args(["-c", "printf started > \"$1\"", "fixture"])
            .arg(&marker)
            .kill_on_drop(true);
        let error = login.spawn("fixture", deadline, &mut command).unwrap_err();
        assert_eq!(
            error,
            if reason == "expired" {
                Failure::Expired
            } else {
                Failure::Cancelled
            }
            .detail()
        );
        assert!(!marker.exists());
    }
}

#[tokio::test]
async fn admitted_native_spawn_still_obeys_connection_cancellation() {
    let (mut login, connection, _cancel) = pending_login();
    let mut command = Command::new("sh");
    command
        .args(["-c", "exec sleep 60"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command.as_std_mut().process_group(0);
    let deadline = Instant::now() + Duration::from_secs(30);
    let child = login.spawn("fixture", deadline, &mut command).unwrap();
    drop(connection);
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        run(child, &mut login.cancel, &mut login.input, deadline, |_| {}),
    )
    .await
    .unwrap();
    assert_eq!(result, Err(Failure::Cancelled));
}

#[test]
fn closed_cancel_channel_is_not_spawn_authority() {
    let (sender, cancel) = watch::channel(false);
    drop(sender);
    assert_eq!(
        check(&cancel, Instant::now() + Duration::from_secs(30)),
        Err(Failure::Cancelled)
    );
}

#[tokio::test]
async fn losing_the_authenticated_connection_revokes_all_pending_logins() {
    let sessions = super::super::LoginSessions::default();
    let connection = super::super::LoginConnection(sessions.clone());
    let (cancel_tx, mut cancel) = watch::channel(false);
    let (input_tx, _input_rx) = mpsc::channel(1);
    sessions.lock().insert(
        "fixture".into(),
        super::super::LoginSession {
            cancel: cancel_tx,
            input: input_tx,
            candidate: None,
        },
    );
    drop(connection);
    assert!(sessions.lock().is_empty());
    tokio::time::timeout(Duration::from_secs(1), cancelled(&mut cancel))
        .await
        .unwrap();
    assert!(*cancel.borrow());
}

#[tokio::test]
async fn successful_login_reads_challenge_and_accepts_input() {
    let (_cancel_tx, mut cancel) = watch::channel(false);
    let (input_tx, mut input) = mpsc::channel(1);
    let mut lines = Vec::new();
    let status = run(
        child("printf 'challenge\\n'; read -r value; test \"$value\" = fixture"),
        &mut cancel,
        &mut input,
        Instant::now() + Duration::from_secs(3),
        |line| {
            lines.push(line);
            input_tx.try_send("fixture".into()).unwrap();
        },
    )
    .await
    .unwrap();
    assert!(status.success());
    assert_eq!(lines, ["challenge"]);
}

#[tokio::test]
async fn deadline_covers_silent_process_and_wait_after_output_eof() {
    for script in ["exec sleep 60", "exec 1>&- 2>&-; exec sleep 60"] {
        let (_cancel_tx, mut cancel) = watch::channel(false);
        let (_input_tx, mut input) = mpsc::channel(1);
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            run(
                child(script),
                &mut cancel,
                &mut input,
                Instant::now() + Duration::from_millis(100),
                |_| {},
            ),
        )
        .await
        .expect("login ignored its deadline");
        assert_eq!(result, Err(Failure::Expired));
    }
}

#[tokio::test]
async fn cancellation_interrupts_backpressured_input() {
    let (cancel_tx, mut cancel) = watch::channel(false);
    let (input_tx, mut input) = mpsc::channel(1);
    input_tx.try_send("x".repeat(1024 * 1024)).unwrap();
    let cancellation = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancel_tx.send(true).unwrap();
    });
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        run(
            child("exec sleep 60"),
            &mut cancel,
            &mut input,
            Instant::now() + Duration::from_secs(30),
            |_| {},
        ),
    )
    .await
    .expect("blocked stdin prevented cancellation");
    cancellation.await.unwrap();
    assert_eq!(result, Err(Failure::Cancelled));
}

#[tokio::test]
async fn closed_channels_cancel_without_spinning() {
    for close_cancel in [true, false] {
        let (cancel_tx, mut cancel) = watch::channel(false);
        let (input_tx, mut input) = mpsc::channel(1);
        let retained_cancel = (!close_cancel).then_some(cancel_tx);
        let retained_input = close_cancel.then_some(input_tx);
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            run(
                child("exec sleep 60"),
                &mut cancel,
                &mut input,
                Instant::now() + Duration::from_secs(30),
                |_| {},
            ),
        )
        .await
        .expect("closed channel spun instead of cancelling");
        assert_eq!(result, Err(Failure::Cancelled));
        drop((retained_cancel, retained_input));
    }
}

#[tokio::test(start_paused = true)]
async fn secret_input_expires_and_cannot_beat_an_elapsed_deadline() {
    let (_cancel_tx, mut cancel) = watch::channel(false);
    let (input_tx, mut receiver) = mpsc::channel(1);
    assert_eq!(
        input(&mut cancel, &mut receiver, Instant::now() + TIMEOUT).await,
        Err(Failure::Expired)
    );
    input_tx.try_send("fixture".into()).unwrap();
    assert_eq!(
        input(&mut cancel, &mut receiver, Instant::now()).await,
        Err(Failure::Expired)
    );
}

#[tokio::test]
async fn secret_publication_rechecks_admission_and_reports_delivery_failure() {
    let (cancel_tx, cancel) = watch::channel(false);
    let deadline = Instant::now() + TIMEOUT;
    assert_eq!(
        publish(false, &cancel, deadline, || panic!(
            "removed login published"
        )),
        Err(Failure::Cancelled)
    );
    assert_eq!(
        publish(true, &cancel, Instant::now(), || panic!(
            "expired login published"
        )),
        Err(Failure::Expired)
    );
    assert_eq!(
        publish(true, &cancel, deadline, || Err(())),
        Err(Failure::Write)
    );
    assert_eq!(publish(true, &cancel, deadline, || Ok(())), Ok(()));
    cancel_tx.send(true).unwrap();
    assert_eq!(
        publish(true, &cancel, deadline, || panic!(
            "cancelled login published"
        )),
        Err(Failure::Cancelled)
    );
    drop(cancel_tx);
    assert_eq!(
        publish(true, &cancel, deadline, || panic!(
            "disconnected login published"
        )),
        Err(Failure::Cancelled)
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn cancellation_stops_owned_descendants_and_preserves_other_processes() {
    let (cancel_tx, mut cancel) = watch::channel(false);
    let (_input_tx, mut input) = mpsc::channel(1);
    let mut unrelated = child("exec sleep 60");
    let mut descendant = None;
    let result = run(
        child("sleep 60 & printf '%s\\n' \"$!\"; wait"),
        &mut cancel,
        &mut input,
        Instant::now() + Duration::from_secs(3),
        |line| {
            descendant = Some(line.parse::<u32>().unwrap());
            cancel_tx.send(true).unwrap();
        },
    )
    .await;
    assert_eq!(result, Err(Failure::Cancelled));
    let isolated = unrelated.try_wait().unwrap().is_none();
    unrelated.kill().await.unwrap();
    assert!(isolated);
    let descendant = descendant.expect("fixture started a descendant");
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match std::fs::read_to_string(format!("/proc/{descendant}/status")) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                Ok(status)
                    if status
                        .lines()
                        .any(|line| line.starts_with("State:") && line.contains("Z (zombie)")) =>
                {
                    break;
                }
                _ => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
    })
    .await
    .expect("owned descendant survived cancellation");
}
#[test]
fn cancellation_and_disconnect_cannot_hide_unconfirmed_exit() {
    let (sender, cancel) = watch::channel(true);
    for registered in [false, true] {
        assert_eq!(
            reconcile(Err(Failure::Cleanup), registered, &cancel, Instant::now()),
            Err(Failure::Cleanup)
        );
    }
    drop(sender);
    assert_eq!(
        reconcile(Err(Failure::Cleanup), true, &cancel, Instant::now()),
        Err(Failure::Cleanup)
    );
}
