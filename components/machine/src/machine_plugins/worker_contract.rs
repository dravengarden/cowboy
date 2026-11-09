//! Whether the Machine's current worker build can read a Provider package.
//!
//! The resident Machine and its workers are released separately, so a package
//! the Machine accepts may still be unreadable to the worker that must launch
//! it. Refusing at install keeps every session on the release it can run.

use super::*;

use cowboy_provider_sdk::RuntimeCapability;
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::process::{Output, Stdio};

const CHECK_FLAG: &str = "--check-provider-package";

async fn run_worker(worker: &Path, arguments: &[&OsStr]) -> Result<Output> {
    let mut attempts = 0;
    loop {
        let output = tokio::process::Command::new(worker)
            .args(arguments)
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output()
            .await;
        match output {
            // A concurrent fork can briefly hold a freshly written file open.
            Err(error) if error.raw_os_error() == Some(libc::ETXTBSY) && attempts < 20 => {
                attempts += 1;
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            output => {
                return output.with_context(|| format!("running worker {}", worker.display()));
            }
        }
    }
}

/// A worker built before [`CHECK_FLAG`] existed cannot be asked. Every
/// capability named here is one those builds already shipped, except MCP
/// bindings, which arrived together with `--mcp-plan`. The match is exhaustive
/// so a new capability is a compile error here rather than an assumed yes.
/// Delete once no retained worker bundle predates the flag.
fn legacy_worker_supports(capability: &RuntimeCapability, help: &str) -> bool {
    match capability {
        RuntimeCapability::ProviderRuntimeV1
        | RuntimeCapability::ProviderGatewayV1
        | RuntimeCapability::ProviderExecutionJsonrpcV1
        | RuntimeCapability::ProviderManagedProfilesV1
        | RuntimeCapability::ProviderManagedImageV1 => true,
        RuntimeCapability::ProviderMcpBindingsV1 => help.contains("--mcp-plan"),
    }
}

async fn ensure_worker_reads(
    worker: &Path,
    package: &Path,
    required: &BTreeSet<RuntimeCapability>,
) -> Result<()> {
    let check = run_worker(worker, &[OsStr::new(CHECK_FLAG), package.as_os_str()]).await?;
    if check.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&check.stderr);
    ensure!(
        stderr.contains(&format!("unexpected argument '{CHECK_FLAG}'")),
        "this Machine's worker cannot read the Provider package; roll out a newer worker generation first: {}",
        stderr.trim()
    );
    let help = run_worker(worker, &[OsStr::new("--help")]).await?;
    let help = String::from_utf8_lossy(&help.stdout);
    let unsupported: Vec<_> = required
        .iter()
        .filter(|capability| !legacy_worker_supports(capability, &help))
        .collect();
    ensure!(
        unsupported.is_empty(),
        "this Machine's worker does not support {unsupported:?}; roll out a newer worker generation first"
    );
    Ok(())
}

impl MachinePluginStore {
    /// Record the worker build that launches new sessions on this Machine.
    pub fn set_worker_command(&self, worker: PathBuf) {
        *self.worker_command.lock() = Some(worker);
    }

    /// No command is known only before the broker exists, when nothing can launch.
    pub(super) async fn ensure_worker_reads_package(
        &self,
        package: &Path,
        required: &BTreeSet<RuntimeCapability>,
    ) -> Result<()> {
        let worker = self.worker_command.lock().clone();
        match worker {
            Some(worker) => ensure_worker_reads(&worker, package, required).await,
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn script(root: &Path, body: &str) -> PathBuf {
        let path = root.join("worker");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    fn required(capability: RuntimeCapability) -> BTreeSet<RuntimeCapability> {
        BTreeSet::from([capability])
    }

    const LEGACY_REFUSAL: &str = r#"[ "$1" = --help ] && { echo "$HELP_TEXT"; exit 0; }; echo "error: unexpected argument '--check-provider-package' found" >&2; exit 2"#;

    #[tokio::test]
    async fn a_worker_that_reads_the_package_admits_it() {
        let root = tempfile::tempdir().unwrap();
        let worker = script(
            root.path(),
            r#"[ "$1" = --check-provider-package ] && [ "$2" = /pkg ]"#,
        );
        ensure_worker_reads(&worker, Path::new("/pkg"), &BTreeSet::new())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn a_worker_that_rejects_the_package_refuses_the_install() {
        let root = tempfile::tempdir().unwrap();
        let worker = script(root.path(), "echo 'unknown variant' >&2; exit 1");
        let error = ensure_worker_reads(&worker, Path::new("/pkg"), &BTreeSet::new())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("unknown variant"), "{error:#}");
    }

    #[tokio::test]
    async fn a_worker_before_the_check_is_judged_by_its_arguments() {
        let root = tempfile::tempdir().unwrap();
        let without = script(
            root.path(),
            &format!("HELP_TEXT='--socket'; {LEGACY_REFUSAL}"),
        );
        let mcp = required(RuntimeCapability::ProviderMcpBindingsV1);
        assert!(
            ensure_worker_reads(&without, Path::new("/pkg"), &mcp)
                .await
                .is_err()
        );
        ensure_worker_reads(
            &without,
            Path::new("/pkg"),
            &required(RuntimeCapability::ProviderManagedImageV1),
        )
        .await
        .unwrap();

        let root = tempfile::tempdir().unwrap();
        let with = script(
            root.path(),
            &format!("HELP_TEXT='--mcp-plan <MCP_PLAN>'; {LEGACY_REFUSAL}"),
        );
        ensure_worker_reads(&with, Path::new("/pkg"), &mcp)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn an_unrunnable_worker_refuses_the_install() {
        let missing = Path::new("/nonexistent/cowboy-acp-worker");
        assert!(
            ensure_worker_reads(missing, Path::new("/pkg"), &BTreeSet::new())
                .await
                .is_err()
        );
    }
}
