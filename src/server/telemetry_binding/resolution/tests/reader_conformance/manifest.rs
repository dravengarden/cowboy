use super::*;
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Lane {
    Controller,
    Machine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Role {
    Active,
    Rollback,
    Cold,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Readers {
    active: PathBuf,
    rollback: PathBuf,
    cold: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Matrix {
    schema: u16,
    controller: Readers,
    machine: Readers,
}

#[derive(Serialize)]
pub(super) struct Artifact {
    pub lane: Lane,
    pub role: Role,
    pub release: PathBuf,
    pub executable: PathBuf,
    source: serde_json::Value,
    source_sha256: String,
    executable_sha256: String,
    executable_chain: Vec<Executable>,
}

#[derive(Serialize)]
struct Executable {
    path: PathBuf,
    sha256: String,
}

fn executable_chain(lane: Lane, release: &Path, entry: &Path) -> Result<Vec<Executable>> {
    let mut paths = vec![entry.to_owned()];
    if lane == Lane::Machine {
        let launcher = release.join("libexec/cowboy-machine").canonicalize()?;
        let payload = launcher.with_file_name(".cowboy-machine-wrapped");
        paths.extend([launcher, payload.canonicalize()?]);
    }
    let payload = std::fs::read(paths.last().unwrap())?;
    ensure!(
        payload.starts_with(b"\x7fELF"),
        "actual ELF reader required"
    );
    paths
        .into_iter()
        .map(|path| {
            ensure!(
                path.starts_with("/nix/store") && path.canonicalize()? == path,
                "immutable executable chain required"
            );
            Ok(Executable {
                sha256: sha256(&std::fs::read(&path)?),
                path,
            })
        })
        .collect()
}

fn immutable(path: &Path) -> bool {
    path.parent() == Some(Path::new("/nix/store"))
        && path.file_name().is_some_and(|name| {
            let name = name.to_string_lossy();
            name.len() > 33
                && name.as_bytes()[32] == b'-'
                && name.as_bytes()[..32]
                    .iter()
                    .all(|b| b.is_ascii_alphanumeric())
        })
}

impl Matrix {
    pub(super) fn resolve(self) -> Result<Vec<Artifact>> {
        ensure!(self.schema == 1, "unsupported matrix schema");
        let mut artifacts = Vec::new();
        for (lane, readers) in [
            (Lane::Controller, self.controller),
            (Lane::Machine, self.machine),
        ] {
            for (role, release) in [
                (Role::Active, readers.active),
                (Role::Rollback, readers.rollback),
                (Role::Cold, readers.cold),
            ] {
                ensure!(
                    immutable(&release) && release.canonicalize()? == release,
                    "exact immutable release required"
                );
                let source_bytes = std::fs::read(release.join("etc/cowboy-release/source.json"))?;
                let source: serde_json::Value = serde_json::from_slice(&source_bytes)?;
                let expected_lane = if lane == Lane::Controller {
                    "controller"
                } else {
                    "machine"
                };
                ensure!(
                    source["schema"] == 1
                        && source["component"] == "cowboy"
                        && source["lane"] == expected_lane
                        && source["dirty"] == false
                        && source["repository"] == "git@github.com:dravengarden/cowboy.git"
                        && source["revision"].as_str().is_some_and(revision_valid),
                    "invalid release provenance"
                );
                ensure!(
                    source["bootstrap"].is_null()
                        || (source["bootstrap"] == true
                            && role == Role::Cold
                            && lane == Lane::Machine),
                    "bootstrap is only a cold Machine reader"
                );
                // Do not copy arbitrary manifest fields into evidence.
                let allowed = [
                    "schema",
                    "component",
                    "lane",
                    "dirty",
                    "repository",
                    "revision",
                    "workerGeneration",
                    "bootstrap",
                ];
                ensure!(
                    source
                        .as_object()
                        .is_some_and(|o| o.keys().all(|key| allowed.contains(&key.as_str()))),
                    "unknown provenance field"
                );
                ensure!(
                    if lane == Lane::Machine {
                        source["workerGeneration"].as_str().is_some_and(|value| {
                            value.len() == 27
                                && value.starts_with("worker-")
                                && value[7..].bytes().all(|b| b.is_ascii_hexdigit())
                        })
                    } else {
                        source["workerGeneration"].is_null()
                    },
                    "invalid worker provenance"
                );
                let executable = release
                    .join("bin")
                    .join(if lane == Lane::Controller {
                        "cowboy"
                    } else {
                        "cowboy-machine"
                    })
                    .canonicalize()?;
                ensure!(
                    executable.starts_with("/nix/store") && executable.is_file(),
                    "immutable executable required"
                );
                let executable_chain = executable_chain(lane, &release, &executable)?;
                artifacts.push(Artifact {
                    lane,
                    role,
                    release,
                    executable_sha256: sha256(&std::fs::read(&executable)?),
                    executable,
                    source_sha256: sha256(&source_bytes),
                    source,
                    executable_chain,
                });
            }
        }
        Ok(artifacts)
    }
}

fn revision_valid(revision: &str) -> bool {
    revision.len() == 40
        && revision
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(super) fn clean_revision() -> Result<String> {
    let status = Command::new("git")
        .args(["status", "--porcelain", "--untracked-files=normal"])
        .output()?;
    ensure!(
        status.status.success() && status.stdout.is_empty(),
        "conformance requires clean committed source"
    );
    let head = Command::new("git").args(["rev-parse", "HEAD"]).output()?;
    let revision = String::from_utf8(head.stdout)?.trim().to_owned();
    ensure!(
        head.status.success() && revision_valid(&revision),
        "invalid harness revision"
    );
    Ok(revision)
}

pub(super) fn require_isolation() -> Result<()> {
    let interfaces: Vec<_> = std::fs::read_dir("/sys/class/net")?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<std::io::Result<_>>()?;
    ensure!(
        interfaces == [std::ffi::OsString::from("lo")] && !rustix::process::geteuid().is_root(),
        "non-root isolated loopback required"
    );
    Ok(())
}

#[test]
fn matrix_is_closed_and_requires_every_role_without_mutable_paths() {
    let valid = serde_json::json!({"schema":1, "controller":{"active":"a", "rollback":"b", "cold":"c"}, "machine":{"active":"d", "rollback":"e", "cold":"f"}});
    for lane in ["controller", "machine"] {
        for role in ["active", "rollback", "cold"] {
            let mut missing = valid.clone();
            missing[lane].as_object_mut().unwrap().remove(role);
            assert!(serde_json::from_value::<Matrix>(missing).is_err());
        }
    }
    let mut extra = valid.clone();
    extra["environment"] = serde_json::json!({"TOKEN":"forbidden"});
    assert!(serde_json::from_value::<Matrix>(extra).is_err());
    assert!(
        serde_json::from_value::<Matrix>(valid)
            .unwrap()
            .resolve()
            .is_err()
    );
    for path in [
        "/run/cowboy-machine",
        "/tmp/release",
        "/nix/store/bad",
        "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-release/../other",
    ] {
        assert!(!immutable(Path::new(path)), "{path}");
    }
}
