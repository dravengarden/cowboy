#![cfg(target_os = "linux")]
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::io::{AsyncBufReadExt as _, BufReader};

fn executable(name: &str) -> PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|path| path.join(name))
        .find(|path| path.is_file())
        .and_then(|path| path.canonicalize().ok())
        .expect("required sandbox tool")
}

#[tokio::test]
async fn host_eof_stops_active_bridge_and_sandbox_without_waiting_for_deadline() {
    let root = tempfile::tempdir().unwrap();
    let program = root.path().join("program.ts");
    let runner = root.path().join("runner.js");
    std::fs::write(&program, "").unwrap();
    std::fs::write(
        &runner,
        r#"
process.stdout.write(JSON.stringify({id:1,op:'ready',args:{}})+'\n');
setInterval(()=>{},1000);
"#,
    )
    .unwrap();
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_cowboy-codeact"))
        .args([
            program,
            runner,
            executable("bun"),
            executable("prlimit"),
            executable("bwrap"),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let pid = child.id().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(3), output.read_line(&mut line))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&line).unwrap()["op"],
        "ready"
    );
    let descendants = descendants(pid);
    assert!(
        !descendants.is_empty(),
        "sandbox must be running before host disconnect"
    );
    drop(child.stdin.take());
    tokio::time::timeout(Duration::from_secs(2), child.wait())
        .await
        .expect("bridge survived host EOF")
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while descendants.iter().any(|pid| live(*pid)) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("sandbox descendants survived host EOF");
}

fn descendants(pid: u32) -> Vec<u32> {
    let mut result = Vec::new();
    // Tokio may spawn the sandbox from any worker thread.
    for task in std::fs::read_dir(format!("/proc/{pid}/task"))
        .unwrap()
        .flatten()
    {
        for child in std::fs::read_to_string(task.path().join("children"))
            .unwrap_or_default()
            .split_whitespace()
        {
            let child = child.parse().unwrap();
            result.push(child);
            result.extend(descendants(child));
        }
    }
    result
}

fn live(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
        stat.rsplit_once(") ")
            .is_some_and(|(_, fields)| !fields.starts_with('Z'))
    })
}
