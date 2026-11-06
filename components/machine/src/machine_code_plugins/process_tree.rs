//! Establish the native runtime's descendant ownership before its first exec.
//! Moving the adapter after spawn races its immediate server/LSP children.
use super::*;
use crate::cgroup::containment::Containment;
use std::os::unix::process::CommandExt as _;

pub(super) struct ProcessTree {
    _containment: Option<Containment>,
    _fallback: Option<PluginProcessGroup>,
}

impl ProcessTree {
    pub(super) fn spawn(command: &mut Command) -> Result<(Child, Self)> {
        command.as_std_mut().process_group(0);
        // The membership FD is installed in pre_exec. A successful spawn
        // therefore cannot run any child code outside the original cgroup.
        let containment = match Containment::prepare(command) {
            Ok(owner) => Some(owner),
            Err(error) => {
                // Non-Linux and undelegated hosts retain their existing
                // interactive capability, without claiming complete resource
                // accounting or detached-descendant containment.
                tracing::warn!(%error, "code runtime lacks complete process containment");
                None
            }
        };
        let child = command.spawn().context("starting code process tree")?;
        let pid = child.id().context("code process tree has no process id")?;
        let fallback = containment.is_none().then(|| PluginProcessGroup::new(pid));
        Ok((
            child,
            Self {
                _containment: containment,
                _fallback: fallback,
            },
        ))
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires delegated writable cgroup v2"]
    async fn immediate_detached_child_inherits_original_code_runtime_group() {
        let mut command = Command::new("sh");
        command
            .args([
                "-c",
                "setsid sh -c 'cat /proc/self/cgroup; exec sleep 60' & wait",
            ])
            .stdout(Stdio::piped())
            .kill_on_drop(true);
        let (mut child, tree) = ProcessTree::spawn(&mut command).unwrap();
        let containment = tree
            ._containment
            .as_ref()
            .expect("delegated fixture must be contained");
        let mut output = [0; 4096];
        let n = tokio::time::timeout(
            Duration::from_secs(2),
            child.stdout.take().unwrap().read(&mut output),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(
            std::str::from_utf8(&output[..n]).unwrap().contains(
                containment
                    .identity
                    .directory
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
            )
        );
        drop(tree);
        let status = tokio::time::timeout(Duration::from_secs(2), child.wait())
            .await
            .unwrap()
            .unwrap();
        assert!(!status.success());
    }
}
