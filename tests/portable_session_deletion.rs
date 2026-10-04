//! Execute the actual installer-generated launcher and read-only native guard.

#![cfg(all(feature = "machine-host", unix))]

use std::os::unix::fs::PermissionsExt as _;
use std::path::Path;
use std::process::Command;

const SERVICE: &str = "svc-0123456789abcdef0123456789abcdef";

fn quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

fn executable(path: &Path, script: &str) {
    std::fs::write(path, script).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[test]
fn generated_launcher_refuses_unsigned_active_and_committed_before_selection() {
    let home = tempfile::tempdir_in("/tmp").unwrap();
    let state = home.path().join("state");
    let bundle = home.path().join("bundle");
    std::fs::create_dir(&bundle).unwrap();
    let selected = home.path().join("selected");
    executable(
        &bundle.join("cowboy-machine"),
        &format!(
            "#!/bin/sh\nif [ \"${{1-}}\" = --check-portable-session-deletion ]; then exec {} \"$@\"; fi\nprintf bootstrap > {}\n",
            quote(Path::new(env!("CARGO_BIN_EXE_cowboy-machine"))),
            quote(&selected)
        ),
    );
    for name in ["cowboy-code-adapter", "cowboy-acp-worker"] {
        executable(&bundle.join(name), "#!/bin/sh\nexit 99\n");
    }
    let path = std::env::var_os("PATH").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_cowboy-machine-install"))
        .env_clear()
        .env("HOME", home.path())
        .env("PATH", &path)
        .args([
            "--controller-url",
            "http://127.0.0.1:1",
            "--service-id",
            SERVICE,
            "--workspace",
            "fixture=/no-fixture-workspace",
            "--enrollment-token",
            "fixture-only",
            "--no-start",
        ])
        .arg("--state-dir")
        .arg(&state)
        .arg("--machine-binary")
        .arg(bundle.join("cowboy-machine"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let launcher = home
        .path()
        .join(format!(".local/bin/cowboy-machine-launch-{SERVICE}"));
    let active = state.join("components/commands/cowboy-machine");
    std::fs::create_dir_all(active.parent().unwrap()).unwrap();
    let journal = state.join("session-deletions");
    std::fs::create_dir(&journal).unwrap();
    std::fs::write(journal.join(".pending-fixture"), "uncommitted").unwrap();
    for active_present in [true, false] {
        if active_present {
            executable(
                &active,
                &format!("#!/bin/sh\nprintf active > {}\n", quote(&selected)),
            );
        } else {
            std::fs::remove_file(&active).unwrap();
        }
        let launch = || {
            Command::new("sh")
                .env_clear()
                .env("HOME", home.path())
                .env("PATH", &path)
                .arg(&launcher)
                .output()
                .unwrap()
        };
        let output = launch();
        if active_present {
            // An executable command file is not a signed cached host selection.
            assert!(!output.status.success());
            assert!(
                String::from_utf8_lossy(&output.stderr).contains("Machine host selection pointer")
            );
            assert!(!selected.exists(), "unsigned host or fallback executed");
        } else {
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(std::fs::read_to_string(&selected).unwrap(), "bootstrap");
            std::fs::remove_file(&selected).unwrap();
        }
        for case in ["record", "directory", "dangling"] {
            let committed = journal.join("deletions.json");
            match case {
                "record" => std::fs::write(&committed, "{}").unwrap(),
                "directory" => std::fs::create_dir(&committed).unwrap(),
                "dangling" => std::os::unix::fs::symlink(state.join("absent"), &committed).unwrap(),
                _ => unreachable!(),
            }
            let output = launch();
            assert!(!output.status.success(), "{case}");
            assert!(
                String::from_utf8_lossy(&output.stderr)
                    .contains("portable Session deletion reader admission")
            );
            assert!(!selected.exists(), "{case}: selected host executed");
            assert!(!state.join("provider-usage.sqlite3").exists());
            if case == "directory" {
                std::fs::remove_dir(committed).unwrap();
            } else {
                std::fs::remove_file(committed).unwrap();
            }
        }
    }
    assert_eq!(
        std::fs::read(journal.join(".pending-fixture")).unwrap(),
        b"uncommitted"
    );
    executable(
        &active,
        &format!("#!/bin/sh\nprintf active > {}\n", quote(&selected)),
    );
    executable(
        &state.join("bootstrap/cowboy-machine"),
        "#!/bin/sh\nexit 23\n",
    );
    let output = Command::new("sh")
        .env_clear()
        .env("HOME", home.path())
        .env("PATH", &path)
        .arg(&launcher)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(23));
    assert!(
        !selected.exists(),
        "failed bootstrap guard allowed active host execution"
    );
}
