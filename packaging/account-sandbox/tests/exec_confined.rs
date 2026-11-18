//! `exec-confined` end to end: the wrapped program cannot signal a process
//! outside its Landlock domain. Skipped on kernels without Landlock ABI 6.

use std::process::Command;

const BINARY: &str = env!("CARGO_BIN_EXE_cowboy-account-sandbox");

fn landlock_available() -> bool {
    // The wrapper refuses to exec below ABI 6; probe with a harmless command.
    let Ok(output) = Command::new(BINARY)
        .args(["exec-confined", "--", "true"])
        .output()
    else {
        return false;
    };
    let stderr = String::from_utf8_lossy(&output.stderr);
    if output.status.success() {
        true
    } else {
        eprintln!("skipping: {stderr}");
        false
    }
}

#[test]
fn confined_programs_cannot_signal_outside_their_domain() {
    if !landlock_available() {
        return;
    }
    let mut outside = Command::new("sleep").arg("30").spawn().unwrap();
    let pid = outside.id().to_string();
    let unconfined = Command::new("sh")
        .args(["-c", &format!("kill -0 {pid}")])
        .status()
        .unwrap();
    assert!(unconfined.success());
    let confined = Command::new(BINARY)
        .args(["exec-confined", "--", "sh", "-c", &format!("kill -0 {pid}")])
        .status()
        .unwrap();
    assert!(
        !confined.success(),
        "a confined process signalled an outside process"
    );
    let inside = Command::new(BINARY)
        .args([
            "exec-confined",
            "--",
            "sh",
            "-c",
            "sleep 30 & kill -0 $!; status=$?; kill $!; exit $status",
        ])
        .status()
        .unwrap();
    assert!(
        inside.success(),
        "a confined process could not signal its own child"
    );
    outside.kill().unwrap();
    outside.wait().unwrap();
}

#[test]
fn exec_confined_requires_a_command() {
    let output = Command::new(BINARY)
        .args(["exec-confined"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let output = Command::new(BINARY)
        .args(["exec-confined", "--", "/nonexistent/program"])
        .output()
        .unwrap();
    assert!(!output.status.success());
}
