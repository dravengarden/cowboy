//! Disposable OS-process fixtures for the incarnation writer, never a production
//! switch: a child commits one change and is SIGKILLed at a chosen stage, then a
//! fresh process reopens the namespace.

use super::*;
use std::io::{BufRead as _, Write as _};
use std::os::unix::process::ExitStatusExt as _;
use std::process::Stdio;
use std::sync::mpsc::channel;

const CHILD: &str = "machine_broker::tests::incarnation_process::child";
const MARK: &str = "COWBOY_INCARNATION_CHECKPOINT";
const STAGES: [&str; 4] = ["Staged", "FileSynced", "Renamed", "DirectorySynced"];

pub(super) fn open(root: &Path, writer: bool) -> incarnations::Store {
    // A sibling test that forks briefly holds an inherited copy of a closed lock
    // descriptor; retry only that window.
    for _ in 0..300 {
        match incarnations::Store::open(root, &deletion_fixture_owner(), writer) {
            Ok(store) => return store,
            Err(error) if format!("{error:#}").contains("already owned") => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("opening incarnation namespace: {error:#}"),
        }
    }
    panic!("incarnation namespace stayed owned");
}

#[test]
#[ignore = "launched only by the disposable incarnation process fixture"]
fn child() {
    let root = PathBuf::from(std::env::var_os("COWBOY_TEST_INCARNATION_ROOT").expect("root"));
    let operation = std::env::var("COWBOY_TEST_INCARNATION_OP").unwrap();
    let stage = std::env::var("COWBOY_TEST_INCARNATION_STAGE").unwrap();
    let mut store = open(&root, true);
    store.set_checkpoint(move |reached| {
        if format!("{reached:?}") == stage {
            println!("{MARK} {reached:?}");
            std::io::stdout().flush().unwrap();
            loop {
                std::thread::park();
            }
        }
    });
    match operation.as_str() {
        "mint" => drop(store.mint("sess-1", incarnations::Origin::Minted).unwrap()),
        "rotate" => drop(store.rotate("sess-1").unwrap()),
        "end" => store.end("sess-1").unwrap(),
        other => panic!("unknown operation {other}"),
    }
}

fn kill_at(root: &Path, operation: &str, stage: &str) {
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", CHILD, "--ignored", "--nocapture"])
        .env("COWBOY_TEST_INCARNATION_ROOT", root)
        .env("COWBOY_TEST_INCARNATION_OP", operation)
        .env("COWBOY_TEST_INCARNATION_STAGE", stage)
        .env_remove("COWBOY_PROVIDER_PACKAGE_PATH")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn incarnation fixture");
    let stdout = child.stdout.take().unwrap();
    let (tx, lines) = channel();
    let pump = std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
        {
            let _ = tx.send(line);
        }
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let line = lines
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap_or_else(|error| panic!("{operation}@{stage} never reached its stage: {error}"));
        if line.contains(MARK) {
            break;
        }
    }
    child.kill().expect("SIGKILL incarnation fixture");
    assert_eq!(child.wait().unwrap().signal(), Some(libc::SIGKILL));
    let _ = pump.join();
}

#[test]
fn sigkill_at_each_commit_stage_leaves_a_valid_namespace_without_replaying_staging() {
    for operation in ["mint", "rotate", "end"] {
        for stage in STAGES {
            let root = tempfile::tempdir().unwrap();
            let namespace = root.path().join("incarnations");
            let baseline = if operation == "mint" {
                None
            } else {
                let mut store = open(&namespace, true);
                let value = store.mint("sess-1", incarnations::Origin::Minted).unwrap();
                drop(store);
                Some(value)
            };
            kill_at(&namespace, operation, stage);

            let published = matches!(stage, "Renamed" | "DirectorySynced");
            // The reader both validates the survivor and sees no staged data.
            let reader = open(&namespace, false);
            let entry = reader.get("sess-1");
            match (operation, published) {
                ("mint", false) => assert!(entry.is_none(), "{operation}@{stage}"),
                ("mint", true) => {
                    let entry = entry.expect("minted lineage survives");
                    assert_eq!(
                        (entry.epoch, entry.origin),
                        (1, incarnations::Origin::Minted)
                    );
                }
                ("rotate", false) | ("end", false) => {
                    let entry = entry.expect("baseline survives an unpublished change");
                    assert_eq!(
                        Some(&entry.incarnation),
                        baseline.as_ref(),
                        "{operation}@{stage}"
                    );
                    assert_eq!(entry.epoch, 1);
                }
                ("rotate", true) => {
                    let entry = entry.expect("rotated lineage survives");
                    assert_ne!(Some(&entry.incarnation), baseline.as_ref());
                    assert_eq!(
                        (entry.epoch, entry.origin),
                        (2, incarnations::Origin::Reset)
                    );
                }
                ("end", true) => assert!(entry.is_none(), "{operation}@{stage}"),
                _ => unreachable!(),
            }
            drop(reader);
            // A later writer may continue from whatever survived.
            let mut writer = open(&namespace, true);
            let next = writer.mint("sess-1", incarnations::Origin::Minted).unwrap();
            assert_eq!(next.len(), 32);
        }
    }
}
