//! Authenticate a bounded, closed bootstrap bundle before any of its code runs.

use std::collections::BTreeMap;
use std::io::Read as _;
use std::os::unix::fs::OpenOptionsExt as _;
use std::path::Path;

use anyhow::{Context as _, Result, ensure};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

use crate::machine_protocol::{
    ArtifactFormat, ComponentKind, DesiredComponent, SessionDeletionReader,
};

const ARTIFACT_LIMIT: u64 = 256 * 1024 * 1024;
const PAYLOAD_LIMIT: u64 = 512 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    id: Id,
    version: String,
    generation: String,
    artifact_url: String,
    digest: String,
    artifact_format: ArtifactFormat,
    entrypoint: String,
    signature: String,
    session_deletion_journal: SessionDeletionReader,
    #[serde(default)]
    probe: Option<Probe>,
    automatic: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Id {
    kind: ComponentKind,
    #[serde(default)]
    slot: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Probe {
    args: Vec<String>,
    timeout_ms: u64,
}

pub(super) fn read_regular(path: &Path, limit: u64) -> Result<Vec<u8>> {
    ensure!(
        std::fs::symlink_metadata(path)?.is_file(),
        "bootstrap input is not a regular file"
    );
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    ensure!(
        file.metadata()?.is_file(),
        "bootstrap input is not a regular file"
    );
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "bootstrap input exceeds size limit"
    );
    Ok(bytes)
}

#[derive(Debug)]
pub(super) struct Authenticated {
    pub(super) payloads: BTreeMap<String, Vec<u8>>,
    pub(super) manifest: Vec<u8>,
    pub(super) artifact: Vec<u8>,
}

pub(super) fn authenticate(manifest: &Path, artifact: &Path, key: &Path) -> Result<Authenticated> {
    let manifest_bytes = read_regular(manifest, 64 * 1024)?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)
        .context("decoding closed signed bootstrap manifest")?;
    let desired = DesiredComponent {
        id: crate::machine_protocol::ComponentId {
            kind: manifest.id.kind,
            slot: manifest.id.slot,
        },
        version: manifest.version,
        generation: manifest.generation,
        artifact_url: manifest.artifact_url,
        digest: manifest.digest,
        artifact_format: manifest.artifact_format,
        entrypoint: Some(manifest.entrypoint),
        signature: Some(manifest.signature),
        session_deletion_journal: Some(manifest.session_deletion_journal),
        probe: manifest
            .probe
            .map(|p| crate::machine_protocol::ComponentProbe {
                args: p.args,
                timeout_ms: p.timeout_ms,
            }),
        automatic: manifest.automatic,
    };
    desired
        .validate_session_deletion_declaration()
        .map_err(anyhow::Error::msg)?;
    ensure!(
        desired.artifact_format == ArtifactFormat::TarGz
            && desired.entrypoint.as_deref() == Some("cowboy-machine"),
        "signed bootstrap requires three-file archive with cowboy-machine entrypoint"
    );
    ensure!(
        !desired.version.is_empty() && !desired.generation.is_empty(),
        "signed bootstrap requires version and generation"
    );
    let publisher = String::from_utf8(read_regular(key, 16 * 1024)?)?;
    let publisher = crate::machine_auth::validate_public_key(&publisher)?;
    ensure!(
        crate::machine_auth::verify(
            &publisher,
            &crate::component_proof::component_proof(&desired),
            desired.signature.as_deref().expect("signature present")
        )?,
        "signed bootstrap publisher signature rejected"
    );
    let bytes = read_regular(artifact, ARTIFACT_LIMIT)?;
    ensure!(
        format!("{:x}", Sha256::digest(&bytes)) == desired.digest,
        "signed bootstrap artifact digest mismatch"
    );
    // Bound extension headers and padding as well as exposed file bodies.
    let decoder = flate2::read::GzDecoder::new(bytes.as_slice()).take(PAYLOAD_LIMIT + 64 * 1024);
    let mut payloads = BTreeMap::new();
    let mut remaining = PAYLOAD_LIMIT;
    for entry in tar::Archive::new(decoder).entries()? {
        let mut entry = entry?;
        let path = entry.path()?;
        let name = path
            .to_str()
            .context("bootstrap archive path is not UTF-8")?;
        ensure!(
            super::bootstrap_probe::PAYLOADS.contains(&name)
                && entry.header().entry_type().is_file(),
            "bootstrap archive requires only the three exact regular payloads"
        );
        let name = name.to_owned();
        ensure!(
            !payloads.contains_key(&name),
            "bootstrap archive contains duplicate payload"
        );
        let mut content = Vec::new();
        (&mut entry).take(remaining + 1).read_to_end(&mut content)?;
        ensure!(
            content.len() as u64 <= remaining,
            "bootstrap expanded payloads exceed size limit"
        );
        remaining -= content.len() as u64;
        ensure!(!content.is_empty(), "bootstrap payload is empty");
        payloads.insert(name, content);
    }
    ensure!(
        payloads.len() == 3,
        "signed bootstrap archive is missing payloads"
    );
    Ok(Authenticated {
        payloads,
        manifest: manifest_bytes,
        artifact: bytes,
    })
}

// This executable is installer-owned, outside the signed publisher payload.
#[derive(clap::Parser)]
struct CheckArgs {
    #[arg(long)]
    check_signed_bootstrap: bool,
    #[arg(long)]
    state_dir: std::path::PathBuf,
    #[arg(long)]
    bundle_dir: std::path::PathBuf,
    #[arg(long)]
    artifact_public_key: std::path::PathBuf,
}

pub(super) fn run_check() -> Result<()> {
    use clap::Parser as _;
    let args = CheckArgs::parse();
    ensure!(
        args.check_signed_bootstrap,
        "signed bootstrap check required"
    );
    check_installed(&args.state_dir, &args.bundle_dir, &args.artifact_public_key)
}

pub(super) fn check_installed(state: &Path, bundle: &Path, key: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    crate::session_deletion_admission::require_empty_portable_namespace(state)?;
    let root = state.canonicalize()?.join("signed-bootstrap");
    ensure!(
        std::fs::symlink_metadata(&root)?.is_dir() && std::fs::symlink_metadata(bundle)?.is_dir(),
        "signed bootstrap directories must be regular"
    );
    ensure!(
        bundle.canonicalize()?.parent() == Some(root.as_path()),
        "signed bootstrap generation is outside its state directory"
    );
    let entries: std::collections::BTreeSet<_> = std::fs::read_dir(bundle)?
        .map(|e| e.map(|e| e.file_name()))
        .collect::<std::io::Result<_>>()?;
    let expected: std::collections::BTreeSet<_> = [
        "cowboy-machine",
        "cowboy-code-adapter",
        "cowboy-acp-worker",
        "manifest.json",
        "artifact",
        "verifier",
    ]
    .map(std::ffi::OsString::from)
    .into_iter()
    .collect();
    ensure!(
        entries == expected,
        "signed bootstrap generation has unexpected entries"
    );
    let authenticated = authenticate(&bundle.join("manifest.json"), &bundle.join("artifact"), key)?;
    for (name, bytes) in &authenticated.payloads {
        let path = bundle.join(name);
        ensure!(
            read_regular(&path, bytes.len() as u64)? == *bytes,
            "installed bootstrap bytes differ from signed package"
        );
        ensure!(
            std::fs::metadata(&path)?.permissions().mode() & 0o111 != 0,
            "signed bootstrap payload is not executable"
        );
        rustix::fs::accessat(
            rustix::fs::CWD,
            &path,
            rustix::fs::Access::EXEC_OK,
            rustix::fs::AtFlags::EACCESS,
        )?;
    }
    #[cfg(feature = "machine-host")]
    crate::machine_components::check_portable_host_cache(state, Some(key))?;
    #[cfg(not(feature = "machine-host"))]
    crate::session_deletion_admission::reader_floor::require_absent_for_install(state)?;
    Ok(())
}

pub(super) fn verifier_digest(path: &Path) -> Result<Vec<u8>> {
    ensure!(
        std::fs::symlink_metadata(path)?.is_file(),
        "bootstrap verifier is not regular"
    );
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    ensure!(
        file.metadata()?.is_file(),
        "bootstrap verifier is not regular"
    );
    let mut reader = file.take(PAYLOAD_LIMIT + 1);
    let mut digest = Sha256::new();
    let mut total = 0;
    let mut buffer = [0_u8; 65536];
    loop {
        let size = reader.read(&mut buffer)?;
        if size == 0 {
            break;
        }
        total += size as u64;
        ensure!(
            total <= PAYLOAD_LIMIT,
            "bootstrap verifier exceeds size limit"
        );
        digest.update(&buffer[..size]);
    }
    Ok(digest.finalize().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::machine_auth::MachineIdentity;
    use crate::machine_protocol::{ComponentId, ComponentProbe};

    const GUARD: &str = "#!/bin/sh\nif [ -e \"$3/portable-session-deletion-reader-floor.json\" ]; then printf '%s' 'portable reader floor' >&2; exit 1; fi\nif [ -e \"$3/session-deletions/deletions.json\" ]; then printf '%s' 'portable Session deletion reader admission' >&2; exit 1; fi\nprintf '%s' '{\"admitted\":true,\"writer\":false,\"host_cache_guard\":2}'\n";

    fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        for (name, bytes) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o755);
            header.as_old_mut().name[..name.len()].copy_from_slice(name.as_bytes());
            header.set_cksum();
            builder.append(&header, *bytes).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    fn package(
        root: &Path,
        bytes: &[u8],
    ) -> (
        std::path::PathBuf,
        std::path::PathBuf,
        std::path::PathBuf,
        DesiredComponent,
    ) {
        let signer = MachineIdentity::load_or_create(&root.join("signer")).unwrap();
        let key = root.join("publisher.pub");
        std::fs::write(&key, signer.public_key()).unwrap();
        let mut desired = DesiredComponent {
            id: ComponentId {
                kind: ComponentKind::MachineHost,
                slot: String::new(),
            },
            version: "signed-bootstrap-1".into(),
            generation: "fixture".into(),
            artifact_url: "https://unused.invalid/bootstrap".into(),
            digest: format!("{:x}", Sha256::digest(bytes)),
            artifact_format: ArtifactFormat::TarGz,
            entrypoint: Some("cowboy-machine".into()),
            signature: None,
            session_deletion_journal: Some(SessionDeletionReader {
                reader_schema: 1,
                writer_schema: 0,
            }),
            probe: Some(ComponentProbe {
                args: vec!["--probe".into()],
                timeout_ms: 5000,
            }),
            automatic: true,
        };
        desired.signature = Some(
            signer
                .sign(&crate::component_proof::component_proof(&desired))
                .unwrap(),
        );
        let manifest = root.join("manifest.json");
        let artifact = root.join("bootstrap.tar.gz");
        std::fs::write(&manifest, serde_json::to_vec(&desired).unwrap()).unwrap();
        std::fs::write(&artifact, bytes).unwrap();
        (manifest, artifact, key, desired)
    }

    #[test]
    fn signed_bundle_authenticates_all_payloads_before_probe_and_rechecks_probe_effects() {
        let root = tempfile::tempdir().unwrap();
        let bytes = archive(&[
            ("cowboy-machine", GUARD.as_bytes()),
            ("cowboy-code-adapter", b"code"),
            ("cowboy-acp-worker", b"worker"),
        ]);
        let (manifest, artifact, key, mut desired) = package(root.path(), &bytes);
        let bundle =
            super::super::bootstrap_probe::Bundle::prepare_signed(&manifest, &artifact, &key)
                .unwrap();
        assert_eq!(
            std::fs::read(bundle.payload("cowboy-acp-worker")).unwrap(),
            b"worker"
        );
        std::fs::write(&artifact, b"changed source after capture").unwrap();
        bundle.verify().unwrap();
        std::fs::write(bundle.payload("cowboy-code-adapter"), b"modified").unwrap();
        assert!(bundle.verify().is_err());
        std::fs::write(&artifact, &bytes).unwrap();
        desired.generation = "tampered".into();
        std::fs::write(&manifest, serde_json::to_vec(&desired).unwrap()).unwrap();
        assert!(
            authenticate(&manifest, &artifact, &key)
                .unwrap_err()
                .to_string()
                .contains("signature")
        );

        let mutated = GUARD.replace(
            "#!/bin/sh\n",
            "#!/bin/sh\nprintf bad > \"${0%/*}/cowboy-code-adapter\"\n",
        );
        let bytes = archive(&[
            ("cowboy-machine", mutated.as_bytes()),
            ("cowboy-code-adapter", b"code"),
            ("cowboy-acp-worker", b"worker"),
        ]);
        let (manifest, artifact, key, _) = package(root.path(), &bytes);
        assert!(
            super::super::bootstrap_probe::Bundle::prepare_signed(&manifest, &artifact, &key)
                .err()
                .unwrap()
                .to_string()
                .contains("changed authenticated payload")
        );
    }

    #[test]
    fn unverified_bundle_never_executes_and_wrong_publisher_or_companion_refuses() {
        let root = tempfile::tempdir().unwrap();
        let marker = root.path().join("ran");
        let script = format!("#!/bin/sh\nprintf ran > '{}'\n", marker.display());
        let bytes = archive(&[
            ("cowboy-machine", script.as_bytes()),
            ("cowboy-code-adapter", b"code"),
            ("cowboy-acp-worker", b"worker"),
        ]);
        let (manifest, artifact, key, _) = package(root.path(), &bytes);
        let wrong = MachineIdentity::load_or_create(&root.path().join("wrong-signer")).unwrap();
        std::fs::write(&key, wrong.public_key()).unwrap();
        assert!(
            super::super::bootstrap_probe::Bundle::prepare_signed(&manifest, &artifact, &key)
                .is_err()
        );
        assert!(!marker.exists());
        let (_, _, key, _) = package(root.path(), &bytes);
        let altered = archive(&[
            ("cowboy-machine", script.as_bytes()),
            ("cowboy-code-adapter", b"changed"),
            ("cowboy-acp-worker", b"worker"),
        ]);
        std::fs::write(&artifact, altered).unwrap();
        assert!(
            super::super::bootstrap_probe::Bundle::prepare_signed(&manifest, &artifact, &key)
                .is_err()
        );
        assert!(!marker.exists());
    }

    #[test]
    fn closed_manifest_and_exact_archive_layout_refuse_ambiguous_admission() {
        let root = tempfile::tempdir().unwrap();
        for entries in [
            vec![
                ("cowboy-machine", GUARD.as_bytes()),
                ("cowboy-code-adapter", b"code".as_slice()),
            ],
            vec![
                ("cowboy-machine", GUARD.as_bytes()),
                ("cowboy-code-adapter", b"code"),
                ("cowboy-acp-worker", b"worker"),
                ("extra", b"extra"),
            ],
            vec![
                ("cowboy-machine", GUARD.as_bytes()),
                ("cowboy-code-adapter", b"code"),
                ("cowboy-acp-worker", b"worker"),
                ("cowboy-machine", GUARD.as_bytes()),
            ],
            vec![
                ("./cowboy-machine", GUARD.as_bytes()),
                ("cowboy-code-adapter", b"code"),
                ("cowboy-acp-worker", b"worker"),
            ],
        ] {
            let bytes = archive(&entries);
            let (manifest, artifact, key, _) = package(root.path(), &bytes);
            assert!(authenticate(&manifest, &artifact, &key).is_err());
        }
        let bytes = archive(&[
            ("cowboy-machine", GUARD.as_bytes()),
            ("cowboy-code-adapter", b"code"),
            ("cowboy-acp-worker", b"worker"),
        ]);
        let (manifest, artifact, key, desired) = package(root.path(), &bytes);
        let valid = serde_json::to_string(&desired).unwrap();
        for json in [
            valid.replacen('{', "{\"unknown\":true,", 1),
            valid.replacen('{', "{\"version\":\"duplicate\",", 1),
            valid.replace("\"kind\":", "\"unknown\":true,\"kind\":"),
            valid.replace("\"args\":", "\"unknown\":true,\"args\":"),
        ] {
            std::fs::write(&manifest, json).unwrap();
            assert!(authenticate(&manifest, &artifact, &key).is_err());
        }
        std::fs::write(&manifest, vec![b' '; 65537]).unwrap();
        assert!(authenticate(&manifest, &artifact, &key).is_err());
        std::fs::remove_file(&artifact).unwrap();
        std::os::unix::fs::symlink(root.path().join("missing"), &artifact).unwrap();
        assert!(read_regular(&artifact, ARTIFACT_LIMIT).is_err());
    }

    #[test]
    fn retained_generation_authenticates_original_evidence_and_all_payloads() {
        let root = tempfile::tempdir_in("/tmp").unwrap();
        let bytes = archive(&[
            ("cowboy-machine", GUARD.as_bytes()),
            ("cowboy-code-adapter", b"code"),
            ("cowboy-acp-worker", b"worker"),
        ]);
        let (manifest, artifact, key, _) = package(root.path(), &bytes);
        let bundle =
            super::super::bootstrap_probe::Bundle::prepare_signed(&manifest, &artifact, &key)
                .unwrap();
        let state = root.path().join("state");
        std::fs::create_dir(&state).unwrap();
        let generation = bundle.install_signed(&state).unwrap().unwrap();
        std::fs::write(&manifest, b"caller manifest replaced").unwrap();
        std::fs::write(&artifact, b"caller artifact replaced").unwrap();
        check_installed(&state, &generation, &key).unwrap();
        for name in [
            "cowboy-machine",
            "cowboy-code-adapter",
            "cowboy-acp-worker",
            "manifest.json",
            "artifact",
        ] {
            let path = generation.join(name);
            let original = std::fs::read(&path).unwrap();
            std::fs::write(&path, b"tampered").unwrap();
            assert!(check_installed(&state, &generation, &key).is_err());
            std::fs::write(&path, original).unwrap();
        }
        std::fs::write(generation.join("extra"), b"extra").unwrap();
        assert!(check_installed(&state, &generation, &key).is_err());
        std::fs::remove_file(generation.join("extra")).unwrap();
        std::fs::write(
            state.join(crate::session_deletion_admission::reader_floor::NAME),
            b"{}",
        )
        .unwrap();
        assert!(check_installed(&state, &generation, &key).is_err());
        std::fs::remove_file(state.join(crate::session_deletion_admission::reader_floor::NAME))
            .unwrap();
        std::fs::remove_file(generation.join("artifact")).unwrap();
        std::os::unix::fs::symlink(&artifact, generation.join("artifact")).unwrap();
        assert!(check_installed(&state, &generation, &key).is_err());
    }

    #[test]
    #[ignore = "requires exact immutable Machine release"]
    fn immutable_signed_launcher_refuses_tampering_before_any_publisher_code() {
        let release = std::path::PathBuf::from(
            std::env::var("COWBOY_TEST_PORTABLE_HOST_RELEASE").expect("release required"),
        );
        for case in [
            "healthy",
            "host",
            "code",
            "worker",
            "manifest",
            "artifact",
            "missing-proof",
            "linked-proof",
            "floor",
        ] {
            let root = tempfile::tempdir_in("/tmp").unwrap();
            let marker = root.path().join("ran");
            let script = GUARD.replace(
                "#!/bin/sh\n",
                &format!("#!/bin/sh\nprintf ran > '{}'\n", marker.display()),
            );
            let bytes = archive(&[
                ("cowboy-machine", script.as_bytes()),
                ("cowboy-code-adapter", b"code"),
                ("cowboy-acp-worker", b"worker"),
            ]);
            let (manifest, artifact, key, _) = package(root.path(), &bytes);
            let state = root.path().join("s");
            let output = std::process::Command::new(release.join("bin/cowboy-machine-install"))
                .env("HOME", root.path())
                .args([
                    "--controller-url",
                    "https://cowboy.invalid",
                    "--service-id",
                    "svc-0123456789abcdef0123456789abcdef",
                    "--workspace",
                    "main=/tmp",
                    "--enrollment-token",
                    "fixture",
                    "--no-start",
                    "--state-dir",
                ])
                .arg(&state)
                .arg("--bootstrap-manifest")
                .arg(&manifest)
                .arg("--bootstrap-artifact")
                .arg(&artifact)
                .arg("--artifact-public-key")
                .arg(&key)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let generation = std::fs::read_dir(state.join("signed-bootstrap"))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            std::fs::remove_file(&marker).unwrap();
            match case {
                "healthy" => {}
                "host" | "code" | "worker" | "manifest" | "artifact" => {
                    let name = match case {
                        "host" => "cowboy-machine",
                        "code" => "cowboy-code-adapter",
                        "worker" => "cowboy-acp-worker",
                        "manifest" => "manifest.json",
                        _ => "artifact",
                    };
                    std::fs::write(generation.join(name), b"tampered").unwrap();
                }
                "missing-proof" => {
                    std::fs::remove_file(generation.join("artifact")).unwrap();
                }
                "linked-proof" => {
                    std::fs::remove_file(generation.join("artifact")).unwrap();
                    std::os::unix::fs::symlink(&artifact, generation.join("artifact")).unwrap();
                }
                "floor" => {
                    std::fs::write(
                        state.join(crate::session_deletion_admission::reader_floor::NAME),
                        b"{}",
                    )
                    .unwrap();
                }
                _ => unreachable!(),
            }
            let launcher = root
                .path()
                .join(".local/bin/cowboy-machine-launch-svc-0123456789abcdef0123456789abcdef");
            let output = std::process::Command::new("/bin/sh")
                .arg(&launcher)
                .env("HOME", root.path())
                .output()
                .unwrap();
            assert_eq!(
                output.status.success(),
                case == "healthy",
                "{case}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                marker.exists(),
                case == "healthy",
                "{case} executed package code before refusal"
            );
            if case != "healthy" {
                assert!(!state.join("run").exists());
            }
        }
    }

    #[test]
    #[ignore = "requires exact immutable Machine release"]
    fn immutable_signed_bundle_installs_without_admitting_floor_refresh() {
        let release = std::path::PathBuf::from(
            std::env::var("COWBOY_TEST_PORTABLE_HOST_RELEASE").expect("release required"),
        );
        let root = tempfile::tempdir_in("/tmp").unwrap();
        let payloads: Vec<_> = super::super::bootstrap_probe::PAYLOADS
            .iter()
            .map(|name| {
                (
                    *name,
                    std::fs::read(release.join("bin").join(name)).unwrap(),
                )
            })
            .collect();
        let entries: Vec<_> = payloads
            .iter()
            .map(|(name, bytes)| (*name, bytes.as_slice()))
            .collect();
        let bytes = archive(&entries);
        let (manifest, artifact, key, _) = package(root.path(), &bytes);
        let state = root.path().join("s");
        let invoke = |refresh| {
            let mut command =
                std::process::Command::new(release.join("bin/cowboy-machine-install"));
            if refresh {
                command.arg("--refresh");
            } else {
                command.args(["--enrollment-token", "fixture"]);
            }
            command
                .env("HOME", root.path())
                .args([
                    "--controller-url",
                    "https://cowboy.invalid",
                    "--service-id",
                    "svc-0123456789abcdef0123456789abcdef",
                    "--workspace",
                    "main=/tmp",
                    "--no-start",
                    "--state-dir",
                ])
                .arg(&state)
                .arg("--bootstrap-manifest")
                .arg(&manifest)
                .arg("--bootstrap-artifact")
                .arg(&artifact)
                .arg("--artifact-public-key")
                .arg(&key)
                .output()
                .unwrap()
        };
        let output = invoke(false);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let generation = std::fs::read_dir(state.join("signed-bootstrap"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        for (name, expected) in &payloads {
            assert_eq!(std::fs::read(generation.join(name)).unwrap(), *expected);
        }
        assert!(
            !state
                .join(crate::session_deletion_admission::reader_floor::NAME)
                .exists()
        );
        std::fs::write(
            state.join(crate::session_deletion_admission::reader_floor::NAME),
            b"{}",
        )
        .unwrap();
        let output = invoke(true);
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("portable reader floor"));
        for (name, expected) in &payloads {
            assert_eq!(std::fs::read(generation.join(name)).unwrap(), *expected);
        }
    }
}
