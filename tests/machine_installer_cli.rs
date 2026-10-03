//! Exercise the shipped installer without compiling unrelated host unit tests.

#![cfg(feature = "machine-host")]

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt as _;
use std::process::Command;

const SERVICE: &str = "svc-0123456789abcdef0123456789abcdef";

#[test]
fn refresh_cli_preserves_a_legacy_install_and_rejects_wrong_service() {
    for correct_service in [true, false] {
        let home = tempfile::tempdir_in("/tmp").unwrap();
        let state = home.path().join("state");
        let bundle = home.path().join("bundle");
        let runtime = home.path().join(".local/bin");
        std::fs::create_dir_all(&state).unwrap();
        std::fs::create_dir_all(&bundle).unwrap();
        std::fs::create_dir_all(&runtime).unwrap();
        for name in ["cowboy-machine", "cowboy-code-adapter", "cowboy-acp-worker"] {
            std::fs::write(bundle.join(name), name).unwrap();
        }
        let native = env!("CARGO_BIN_EXE_cowboy-machine").replace('\'', "'\\''");
        std::fs::write(
            bundle.join("cowboy-machine"),
            format!("#!/bin/sh\nexec '{native}' \"$@\"\n"),
        )
        .unwrap();
        std::fs::set_permissions(
            bundle.join("cowboy-machine"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        std::fs::write(state.join("identity_ed25519"), "existing key").unwrap();
        std::fs::write(state.join("machine-id"), "mac").unwrap();
        std::fs::write(state.join("enrollment-token"), "untouched token").unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let legacy = runtime.join("cowboy-machine-launch");
        let socket = state.join("run/original.sock");
        let old_script = format!(
            "#!/bin/sh\nexec machine --controller-url '{origin}' --service-id '{SERVICE}' --state-dir '{}' --socket '{}'\n",
            state.display(),
            socket.display()
        );
        std::fs::write(&legacy, &old_script).unwrap();
        let server = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            std::time::Instant::now() < deadline,
                            "installer never contacted the test Service"
                        );
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    Err(error) => panic!("accepting installer request: {error}"),
                }
            };
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                .unwrap();
            let mut request = [0; 4096];
            let size = stream.read(&mut request).unwrap();
            assert!(
                std::str::from_utf8(&request[..size])
                    .unwrap()
                    .starts_with("GET /api/machine/service ")
            );
            let service = if correct_service {
                SERVICE
            } else {
                "svc-ffffffffffffffffffffffffffffffff"
            };
            let body = format!("{{\"service_id\":\"{service}\"}}");
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        });
        let output = Command::new(env!("CARGO_BIN_EXE_cowboy-machine-install"))
            .env("HOME", home.path())
            .args([
                "--refresh",
                "--no-start",
                "--plugin-operation-admission",
                "--controller-url",
                &origin,
                "--service-id",
                SERVICE,
                "--workspace",
                "home=/work",
                "--state-dir",
            ])
            .arg(&state)
            .arg("--machine-binary")
            .arg(bundle.join("cowboy-machine"))
            .output()
            .unwrap();
        assert_eq!(
            output.status.success(),
            correct_service,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        server.join().unwrap();
        assert_eq!(
            std::fs::read_to_string(state.join("identity_ed25519")).unwrap(),
            "existing key"
        );
        assert_eq!(
            std::fs::read_to_string(state.join("enrollment-token")).unwrap(),
            "untouched token"
        );
        let script = std::fs::read_to_string(&legacy).unwrap();
        if correct_service {
            assert!(script.contains("--plugin-operation-admission"));
            assert!(script.contains(socket.to_str().unwrap()));
            assert!(!script.contains("--enrollment-token-file"));
            assert!(state.join("bootstrap/cowboy-acp-worker").is_file());
            #[cfg(target_os = "macos")]
            {
                assert!(
                    home.path()
                        .join("Library/LaunchAgents/xyz.stormbird.cowboy-machine.plist")
                        .is_file()
                );
                assert!(
                    !home
                        .path()
                        .join(format!(
                            "Library/LaunchAgents/xyz.stormbird.cowboy-machine.{SERVICE}.plist"
                        ))
                        .exists()
                );
            }
        } else {
            assert_eq!(script, old_script);
            assert!(!state.join("bootstrap").exists());
        }
    }
}
