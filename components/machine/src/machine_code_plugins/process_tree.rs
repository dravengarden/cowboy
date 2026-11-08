//! Establish the native runtime's descendant ownership before its first exec.
//! Moving the adapter after spawn races its immediate server/LSP children.
use super::*;
use crate::cgroup::containment::Containment;
use std::os::unix::process::CommandExt as _;

pub(super) mod retirement;

pub(super) struct ProcessTree {
    _containment: Option<Containment>,
    _fallback: Option<PluginProcessGroup>,
    stop_requested: bool,
    retirement: Option<retirement::Reservation>,
}

impl ProcessTree {
    pub(super) fn request_stop(&mut self) -> Result<()> {
        if !self.stop_requested {
            if let Some(containment) = &self._containment {
                containment
                    .signal_stop()
                    .context("stopping original Code descendants")?;
            } else {
                self._fallback.take();
            }
            self.stop_requested = true;
        }
        Ok(())
    }

    pub(super) fn resources(&self) -> Result<crate::cgroup::containment::ResourceSample> {
        self._containment
            .as_ref()
            .context("Code process ownership unavailable")?
            .resources()
            .context("observing original Code process tree")
    }

    #[cfg(test)]
    pub(super) fn spawn(command: &mut Command) -> Result<(Child, Self)> {
        Self::spawn_in(command, None)
    }

    pub(super) fn spawn_in(
        command: &mut Command,
        pool: Option<&Arc<retirement::Retirement>>,
    ) -> Result<(Child, Self)> {
        let retirement = pool.map(|pool| pool.reserve()).transpose()?;
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
                stop_requested: false,
                retirement,
            },
        ))
    }
}

impl Drop for ProcessTree {
    fn drop(&mut self) {
        if let Some(group) = self._containment.take() {
            let _ = group.signal_stop();
            if let Some(reservation) = self.retirement.take() {
                reservation.retire(group);
            }
            // Without a host pool, the containment retains its existing Drop
            // behavior. Unsupported platforms also retain their PGID fallback.
        }
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
        let (mut child, mut tree) = ProcessTree::spawn(&mut command).unwrap();
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
        let sample = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(sample) = tree.resources()
                    && sample.processes >= 2
                    && sample.rss_bytes > 0
                {
                    break sample;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("original process tree must have real RSS/CPU accounting");
        assert!(sample.cpu_usage_usec > 0);
        tree.request_stop().unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Ok(sample) = tree.resources()
                    && sample.processes == 0
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("stopped descendants remain accountable until observed empty");
        drop(tree);
        let status = tokio::time::timeout(Duration::from_secs(2), child.wait())
            .await
            .unwrap()
            .unwrap();
        assert!(!status.success());
    }
}
