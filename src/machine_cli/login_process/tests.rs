use super::*;
use std::os::unix::process::CommandExt as _;
use std::process::Stdio;
use tokio::process::Command;

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

#[tokio::test]
async fn losing_the_authenticated_connection_revokes_all_pending_logins() {
    let sessions = super::super::LoginSessions::default();
    let connection = super::super::LoginConnection(sessions.clone());
    let (cancel_tx, mut cancel) = watch::channel(false);
    let (input_tx, _input_rx) = mpsc::unbounded_channel();
    sessions.lock().insert(
        "fixture".into(),
        super::super::LoginSession {
            cancel: cancel_tx,
            input: input_tx,
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
    let (input_tx, mut input) = mpsc::unbounded_channel();
    let mut lines = Vec::new();
    let status = run(
        child("printf 'challenge\\n'; read -r value; test \"$value\" = fixture"),
        &mut cancel,
        &mut input,
        Instant::now() + Duration::from_secs(3),
        |line| {
            lines.push(line);
            input_tx.send("fixture".into()).unwrap();
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
        let (_input_tx, mut input) = mpsc::unbounded_channel();
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
    let (input_tx, mut input) = mpsc::unbounded_channel();
    input_tx.send("x".repeat(1024 * 1024)).unwrap();
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
        let (input_tx, mut input) = mpsc::unbounded_channel();
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
    let (input_tx, mut receiver) = mpsc::unbounded_channel();
    assert_eq!(
        input(&mut cancel, &mut receiver, Instant::now() + TIMEOUT).await,
        Err(Failure::Expired)
    );
    input_tx.send("fixture".into()).unwrap();
    assert_eq!(
        input(&mut cancel, &mut receiver, Instant::now()).await,
        Err(Failure::Expired)
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn cancellation_stops_owned_descendants_and_preserves_other_processes() {
    let (cancel_tx, mut cancel) = watch::channel(false);
    let (_input_tx, mut input) = mpsc::unbounded_channel();
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
