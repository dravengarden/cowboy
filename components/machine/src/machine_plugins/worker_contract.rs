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

/// A worker built before [`CHECK_FLAG`] cannot be asked, so look for the
/// capability names its own decoder was compiled with. A wrapper or stripped
/// build shows none and is refused. Delete once no retained worker bundle
/// predates the flag.
async fn ensure_legacy_worker_names(
    worker: &Path,
    required: &BTreeSet<RuntimeCapability>,
) -> Result<()> {
    let binary = tokio::fs::read(worker)
        .await
        .with_context(|| format!("reading worker {}", worker.display()))?;
    let mut unsupported = Vec::new();
    for capability in required {
        let name = serde_json::to_value(capability)?;
        let name = name.as_str().context("capability is not a string")?;
        if !binary
            .windows(name.len())
            .any(|window| window == name.as_bytes())
        {
            unsupported.push(name.to_owned());
        }
    }
    ensure!(
        unsupported.is_empty(),
        "this Machine's worker does not support {unsupported:?}; roll out a newer worker generation first"
    );
    Ok(())
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
    ensure_legacy_worker_names(worker, required).await
}

impl MachinePluginStore {
    /// Record the worker build that launches new sessions on this Machine.
    pub fn set_worker_command(&self, worker: PathBuf) {
        *self.worker_command.lock() = Some(worker);
    }

    /// Machine startup records the worker before it accepts Controller commands;
    /// only a store with no Machine around it has none.
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

    const LEGACY_REFUSAL: &str =
        "echo \"error: unexpected argument '--check-provider-package' found\" >&2; exit 2";

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
    async fn a_worker_before_the_check_is_judged_by_its_compiled_names() {
        let root = tempfile::tempdir().unwrap();
        let without = script(root.path(), LEGACY_REFUSAL);
        let mcp = required(RuntimeCapability::ProviderMcpBindingsV1);
        assert!(
            ensure_worker_reads(&without, Path::new("/pkg"), &mcp)
                .await
                .is_err()
        );
        let root = tempfile::tempdir().unwrap();
        let with = script(
            root.path(),
            &format!("# provider.mcp-bindings.v1\n{LEGACY_REFUSAL}"),
        );
        ensure_worker_reads(&with, Path::new("/pkg"), &mcp)
            .await
            .unwrap();
        assert!(
            ensure_worker_reads(
                &with,
                Path::new("/pkg"),
                &required(RuntimeCapability::ProviderManagedImageV1),
            )
            .await
            .is_err()
        );
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
