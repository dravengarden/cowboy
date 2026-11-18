//! The same authentication I/O over an account-sandbox unit's pipes: the
//! challenge is read from its stdout and stderr and the pasted value written
//! to its stdin. The unit runs in the daemon, so this ends when both outputs
//! close; the caller confirms the unit's end and outcome with the daemon.
use super::*;
use tokio::io::{AsyncRead, AsyncWrite};

pub(in crate::machine_cli) async fn drive_pipes(
    mut stdin: impl AsyncWrite + Unpin,
    stdout: impl AsyncRead + Unpin,
    stderr: impl AsyncRead + Unpin,
    cancel: &mut watch::Receiver<bool>,
    input: &mut mpsc::Receiver<String>,
    deadline: Instant,
    mut on_line: impl FnMut(String),
) -> Result<(), Failure> {
    let io = async {
        let mut stdout = output::Output::new(stdout);
        let mut stderr = output::Output::new(stderr);
        let (mut stdout_open, mut stderr_open) = (true, true);
        while stdout_open || stderr_open {
            let line = tokio::select! {
                line = stdout.next_line(), if stdout_open => {
                    if matches!(line, Ok(None)) { stdout_open = false; }
                    line
                },
                line = stderr.next_line(), if stderr_open => {
                    if matches!(line, Ok(None)) { stderr_open = false; }
                    line
                },
                value = input.recv() => {
                    let value = value.ok_or(Failure::Cancelled)?;
                    stdin.write_all(value.as_bytes()).await.map_err(|_| Failure::Write)?;
                    stdin.write_all(b"\n").await.map_err(|_| Failure::Write)?;
                    continue;
                },
            };
            if let Some(line) = line.map_err(|_| Failure::Read)? {
                on_line(line);
            }
        }
        Ok(())
    };
    tokio::select! {
        biased;
        () = cancelled(cancel) => Err(Failure::Cancelled),
        () = tokio::time::sleep_until(deadline) => Err(Failure::Expired),
        result = io => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncBufReadExt as _;

    #[tokio::test]
    async fn lines_and_input_cross_the_pipes_until_both_outputs_close() {
        let (stdin, mut unit_stdin) = tokio::io::duplex(256);
        let (mut unit_stdout, stdout) = tokio::io::duplex(256);
        let (unit_stderr, stderr) = tokio::io::duplex(256);
        let (_cancel_tx, mut cancel) = watch::channel(false);
        let (input_tx, mut input) = mpsc::channel(1);
        let unit = tokio::spawn(async move {
            unit_stdout
                .write_all(b"https://example.com/auth\n")
                .await
                .unwrap();
            let mut pasted = String::new();
            tokio::io::BufReader::new(&mut unit_stdin)
                .read_line(&mut pasted)
                .await
                .unwrap();
            unit_stdout.write_all(b"done\n").await.unwrap();
            drop((unit_stdout, unit_stderr));
            pasted
        });
        input_tx.send("code".into()).await.unwrap();
        let mut lines = Vec::new();
        drive_pipes(
            stdin,
            stdout,
            stderr,
            &mut cancel,
            &mut input,
            Instant::now() + Duration::from_secs(5),
            |line| lines.push(line),
        )
        .await
        .unwrap();
        assert_eq!(unit.await.unwrap(), "code\n");
        assert_eq!(lines, ["https://example.com/auth", "done"]);
    }

    #[tokio::test]
    async fn cancellation_ends_a_unit_that_keeps_its_outputs_open() {
        let (stdin, _unit_stdin) = tokio::io::duplex(64);
        let (_unit_stdout, stdout) = tokio::io::duplex(64);
        let (_unit_stderr, stderr) = tokio::io::duplex(64);
        let (cancel_tx, mut cancel) = watch::channel(false);
        let (_input_tx, mut input) = mpsc::channel(1);
        cancel_tx.send(true).unwrap();
        let result = drive_pipes(
            stdin,
            stdout,
            stderr,
            &mut cancel,
            &mut input,
            Instant::now() + Duration::from_secs(5),
            |_| {},
        )
        .await;
        assert_eq!(result, Err(Failure::Cancelled));
    }
}
