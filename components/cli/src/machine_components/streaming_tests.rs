//! Actual immutable startup verification under a child-only address-space limit.

use super::*;
use std::fs::File;
use std::io::{Read as _, Seek as _, Write as _};
use std::os::unix::fs::PermissionsExt as _;

use crate::machine_auth::MachineIdentity;
use crate::machine_protocol::{ComponentId, ComponentKind, SessionDeletionReader};

fn digest(path: &Path) -> String {
    let mut file = File::open(path).unwrap();
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let length = file.read(&mut buffer).unwrap();
        if length == 0 {
            break;
        }
        hash.update(&buffer[..length]);
    }
    format!("{:x}", hash.finalize())
}

fn check(release: &Path, state: &Path, key: &Path) -> std::process::Output {
    let limiter = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|p| p.join("prlimit"))
        .find(|p| p.is_file())
        .unwrap()
        .canonicalize()
        .unwrap();
    assert!(limiter.starts_with("/nix/store"));
    let mut command = std::process::Command::new(limiter);
    command
        .args(["--as=201326592:201326592", "--core=0:0", "--"])
        .arg(release.join("bin/cowboy-machine"));
    command
        .args(["--check-portable-session-deletion", "--state-dir"])
        .arg(state)
        .arg("--artifact-public-key")
        .arg(key)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("TOKIO_WORKER_THREADS", "1")
        .env("TMPDIR", state)
        .env("LANG", "C.UTF-8");
    // The pinned limiter applies only to this disposable child. Never constrain
    // the parent test runner, resident Machine or any production worker.
    command.output().unwrap()
}

#[test]
#[ignore = "requires exact current and preceding immutable Machine releases"]
fn immutable_large_cached_hosts_authenticate_with_less_memory_than_the_artifact() {
    let release = PathBuf::from(std::env::var("COWBOY_TEST_PORTABLE_HOST_RELEASE").unwrap());
    let old = PathBuf::from(std::env::var("COWBOY_TEST_PORTABLE_PRE_STREAMING_RELEASE").unwrap());
    let mut observations = Vec::new();
    for archive in [false, true] {
        let state = tempfile::tempdir_in("/tmp").unwrap();
        let identity = MachineIdentity::load_or_create(&state.path().join("signer")).unwrap();
        let key = state.path().join("publisher.pub");
        std::fs::write(&key, identity.public_key()).unwrap();
        let marker = state.path().join("publisher-executed");
        let shell = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|p| p.join("sh"))
            .find(|p| p.is_file())
            .unwrap();
        let input = state.path().join("input");
        let mut file = File::create(&input).unwrap();
        write!(
            file,
            "#!{}\nprintf executed > '{}'\nexit 0\n#",
            shell.display(),
            marker.display()
        )
        .unwrap();
        // Sparse padding makes large local evidence without a large allocation.
        // The diagnostic never executes these signed fixture bytes.
        file.set_len(240 * 1024 * 1024).unwrap();
        drop(file);
        let artifact = state.path().join("artifact-input");
        if archive {
            let encoder = flate2::write::GzEncoder::new(
                File::create(&artifact).unwrap(),
                flate2::Compression::none(),
            );
            let mut builder = tar::Builder::new(encoder);
            let mut header = tar::Header::new_gnu();
            header.set_size(std::fs::metadata(&input).unwrap().len());
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, "bin/host", File::open(&input).unwrap())
                .unwrap();
            builder.into_inner().unwrap().finish().unwrap();
        } else {
            std::fs::copy(&input, &artifact).unwrap();
        }
        let mut desired = DesiredComponent {
            id: ComponentId {
                kind: ComponentKind::MachineHost,
                slot: String::new(),
            },
            version: "large".into(),
            generation: "large-reader".into(),
            artifact_url: "https://unused.invalid".into(),
            digest: digest(&artifact),
            artifact_format: if archive {
                ArtifactFormat::TarGz
            } else {
                ArtifactFormat::Raw
            },
            entrypoint: archive.then(|| "bin/host".into()),
            signature: None,
            session_deletion_journal: Some(SessionDeletionReader {
                reader_schema: 1,
                writer_schema: 0,
            }),
            probe: None,
            automatic: false,
        };
        desired.signature = Some(identity.sign(&component_proof(&desired)).unwrap());
        let root = state.path().join("components");
        let generation = root
            .join("payloads/machine_host/large")
            .join(&desired.digest);
        std::fs::create_dir_all(&generation).unwrap();
        std::fs::copy(&artifact, generation.join("artifact")).unwrap();
        let executable = if archive {
            generation.join("content/bin/host")
        } else {
            generation.join("bin")
        };
        std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
        std::fs::copy(&input, &executable).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(
            generation.join("manifest.json"),
            serde_json::to_vec(&desired).unwrap(),
        )
        .unwrap();
        std::fs::create_dir_all(root.join("active")).unwrap();
        std::fs::create_dir_all(root.join("commands")).unwrap();
        std::os::unix::fs::symlink(&generation, root.join("active/machine_host")).unwrap();
        std::os::unix::fs::symlink(&executable, root.join("commands/cowboy-machine")).unwrap();
        for floor in [false, true] {
            if floor {
                cached_host::retain_reader_floor(&root, &desired, identity.public_key()).unwrap();
            }
            let preceding = check(&old, state.path(), &key);
            assert!(
                !preceding.status.success(),
                "preceding whole-artifact reader unexpectedly fit"
            );
            let previous_error = String::from_utf8_lossy(&preceding.stderr).into_owned();
            assert!(
                previous_error.contains("memory allocation")
                    || previous_error.contains("out of memory")
                    || previous_error.contains("allocation failed"),
                "{:?}",
                preceding
            );
            let current = check(&release, state.path(), &key);
            assert!(
                current.status.success(),
                "{archive}/{floor}: {}",
                String::from_utf8_lossy(&current.stderr)
            );
            let receipt: serde_json::Value = serde_json::from_slice(&current.stdout).unwrap();
            assert_eq!(receipt["admitted"], true);
            assert_eq!(receipt["writer"], false);
            assert!(!marker.exists());
            observations.push(serde_json::json!({
                "format": if archive { "tar_gz" } else { "raw" }, "floor": floor,
                "artifactBytes": std::fs::metadata(&artifact).unwrap().len(),
                "payloadBytes": std::fs::metadata(&input).unwrap().len(),
                "artifactSha256": desired.digest,
                "precedingMemoryFailure": previous_error,
                "currentReceipt": receipt,
                "publisherMarkerAbsent": true,
            }));
        }
        let floor_path = state
            .path()
            .join(crate::session_deletion_admission::reader_floor::NAME);
        let floor_bytes = std::fs::read(&floor_path).unwrap();
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .open(generation.join("artifact"))
            .unwrap();
        file.seek(std::io::SeekFrom::End(-1)).unwrap();
        file.write_all(&[0x33]).unwrap();
        drop(file);
        let refused = check(&release, state.path(), &key);
        assert!(!refused.status.success());
        assert!(String::from_utf8_lossy(&refused.stderr).contains("artifact digest mismatch"));
        assert_eq!(std::fs::read(&floor_path).unwrap(), floor_bytes);
        assert!(!marker.exists());
    }
    if let Ok(path) = std::env::var("COWBOY_TEST_CACHE_STREAMING_RECEIPT") {
        let receipt = serde_json::json!({
            "schema": "cowboy.cached-host-streaming-memory-acceptance/v1",
            "accepted": true, "release": release, "precedingRelease": old,
            "addressSpaceBytes": 192 * 1024 * 1024,
            "tokioWorkerThreads": 1, "coreBytes": 0,
            "observations": observations, "tamperedArtifactRefusals": 2,
            "floorUnchangedAfterRefusal": true, "publisherCodeExecuted": false,
        });
        std::fs::write(path, serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
    }
}
