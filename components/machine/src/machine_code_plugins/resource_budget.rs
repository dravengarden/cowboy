//! Automatic warming yields to all existing interactive Code work. These are
//! fresh admission observations, not kernel limits or a promise about future
//! LSP allocation. Never stop another owner's runtime to recover capacity.
use super::*;
use crate::cgroup::containment::ResourceSample;
use crate::config::{Config, Handle, Scope, schema};
use std::time::Instant;

#[derive(Clone, Copy)]
pub struct Policy {
    rss_bytes: u64,
    processes: u64,
    cpu_millicores: u64,
}

impl Policy {
    pub fn from_config(handle: Option<&Handle>) -> Self {
        let config = handle.map_or_else(
            || Arc::new(Config::defaults(Scope::Device)),
            Handle::current,
        );
        Self {
            rss_bytes: config.get(&schema::DEVICE_CODE_AGENT_RSS),
            processes: config.get(&schema::DEVICE_CODE_AGENT_PROCESSES) as u64,
            cpu_millicores: config.get(&schema::DEVICE_CODE_AGENT_CPU) as u64,
        }
    }

    fn fits(self, sample: Totals) -> bool {
        sample.rss_bytes < self.rss_bytes
            && sample.processes < self.processes
            && sample.cpu_millicores < self.cpu_millicores
    }
}

#[derive(Clone, Copy, Default, serde::Serialize)]
struct Totals {
    processes: u64,
    rss_bytes: u64,
    cpu_millicores: u64,
}

struct Observation {
    // Retain the original instances through both samples. An allocation address
    // cannot be reused for a different runtime during the observation window.
    _runtimes: Vec<Arc<RunningCodeRuntime>>,
    samples: BTreeMap<usize, ResourceSample>,
}

pub(super) struct Admission(Observation, Instant);

impl Admission {
    pub(super) fn allows(&self, runtime: &Arc<RunningCodeRuntime>) -> bool {
        self.1.elapsed() <= Duration::from_secs(2)
            && self
                .0
                ._runtimes
                .iter()
                .any(|observed| Arc::ptr_eq(observed, runtime))
    }
}

impl Observation {
    fn totals(&self) -> Result<Totals> {
        let mut totals = Totals::default();
        for sample in self.samples.values() {
            totals.processes = totals
                .processes
                .checked_add(u64::from(sample.processes))
                .context("Code process count overflow")?;
            totals.rss_bytes = totals
                .rss_bytes
                .checked_add(sample.rss_bytes)
                .context("Code RSS overflow")?;
        }
        Ok(totals)
    }

    fn since(&self, before: &Self, elapsed: Duration) -> Result<Totals> {
        ensure!(
            self.samples.keys().eq(before.samples.keys()),
            "Code runtime membership changed"
        );
        ensure!(
            (Duration::from_millis(250)..=Duration::from_secs(2)).contains(&elapsed),
            "Code CPU observation is stale"
        );
        let mut usage = 0_u128;
        for (id, sample) in &self.samples {
            usage += u128::from(
                sample
                    .cpu_usage_usec
                    .checked_sub(before.samples[id].cpu_usage_usec)
                    .context("Code CPU identity changed")?,
            );
        }
        let mut totals = self.totals()?;
        totals.cpu_millicores =
            u64::try_from(usage * 1000 / elapsed.as_micros()).context("Code CPU rate overflow")?;
        Ok(totals)
    }
}

fn refused(state: &str, sample: Option<Totals>) -> Value {
    serde_json::json!({"type":"agentBufferBudget", "api_version":1, "state":state, "resources":sample})
}

pub(super) fn unavailable() -> Value {
    refused("unavailable", None)
}

impl CodeRuntimeHost {
    async fn resource_observation(&self) -> Result<Observation> {
        for route in self.routes.lock().await.values() {
            // Busy interactive work wins; never queue this observation behind
            // a native exchange or treat a legacy socket as an empty tree.
            let route = route.try_lock().context("Code worktree is busy")?;
            ensure!(
                !matches!(route.target, Some(CodeTarget::Legacy(_))),
                "legacy Code resources are unowned"
            );
        }
        let runtimes: Vec<_> = self
            .engines
            .lock()
            .await
            .values()
            .filter_map(Weak::upgrade)
            .collect();
        tokio::task::spawn_blocking(move || {
            let mut samples = BTreeMap::new();
            for runtime in &runtimes {
                ensure!(
                    runtime.is_running()?,
                    "Code runtime exited during observation"
                );
                let sample = runtime
                    .process_group
                    .lock()
                    .as_ref()
                    .context("Code containment unavailable")?
                    .resources()?;
                samples.insert(Arc::as_ptr(runtime) as usize, sample);
            }
            Ok(Observation {
                _runtimes: runtimes,
                samples,
            })
        })
        .await
        .context("Code resource observation interrupted")?
    }

    pub async fn acquire_agent(
        &self,
        plugin_id: &str,
        payload: &Value,
        active: Option<String>,
        policy: Policy,
        select: impl FnOnce() -> Result<CodeRuntimeSelection>,
    ) -> Result<Value> {
        ensure!(
            payload["type"] == "acquireAgentBuffer",
            "invalid agent warming request"
        );
        let fields = payload
            .as_object()
            .context("invalid agent warming request")?;
        ensure!(
            fields.len() == 4
                && fields
                    .keys()
                    .all(|key| matches!(key.as_str(), "type" | "worktree" | "path" | "leaseId")),
            "invalid agent warming request fields"
        );
        // Only automatic callers queue here. Interactive requests and releases
        // do not acquire this mutex or wait for CPU sampling.
        let _admission = self.agent_admission.lock().await;
        match self
            .resource_observation()
            .await
            .and_then(|sample| sample.totals())
        {
            Ok(sample) if policy.fits(sample) => {}
            Ok(sample) => return Ok(refused("saturated", Some(sample))),
            Err(_) => return Ok(refused("unavailable", None)),
        }
        request_worktree(payload)?;
        let (path, lease) = WorktreeRoute::buffer_key(payload)?;
        ensure!(
            !path.is_empty()
                && path.len() <= 4096
                && !lease.is_empty()
                && lease.len() <= 128
                && !lease.chars().any(char::is_whitespace),
            "invalid agent warming buffer identity"
        );
        let selection = select()?;
        ensure!(
            matches!(selection, CodeRuntimeSelection::Installed(_)),
            "agent warming requires an owned Code runtime"
        );
        // Keep a cold adapter alive while sampling, without taking a worktree
        // route lock. Interactive traffic never waits for this CPU window.
        let (_runtime, _) = self.target(selection.clone()).await?;
        let admission = match self.observe_budget().await {
            Ok((sample, admission)) if policy.fits(sample) => admission,
            Ok((sample, _)) => return Ok(refused("saturated", Some(sample))),
            Err(_) => return Ok(refused("unavailable", None)),
        };
        self.request_with_admission(
            plugin_id,
            payload,
            active,
            || Ok(selection),
            Some(admission),
        )
        .await
    }

    async fn observe_budget(&self) -> Result<(Totals, Admission)> {
        let before = self.resource_observation().await?;
        let start = Instant::now();
        tokio::time::sleep(Duration::from_millis(250)).await;
        // Excluding the second read time is conservative for admission.
        let elapsed = start.elapsed();
        let observed_at = Instant::now();
        let after = self.resource_observation().await?;
        ensure!(
            start.elapsed() <= Duration::from_secs(2),
            "Code resource observation is stale"
        );
        Ok((
            after.since(&before, elapsed)?,
            Admission(after, observed_at),
        ))
    }

    pub(super) async fn acquire_admitted(
        &self,
        route: &mut WorktreeRoute,
        socket: &Path,
        payload: &Value,
    ) -> Result<Value> {
        let open = serde_json::json!({"type":"openBuffer", "worktree":payload["worktree"], "path":payload["path"], "leaseId":payload["leaseId"]});
        // Reserve before either native effect. Cancellation while ensuring the
        // worktree cannot lose its original route or admission ownership.
        route.dispatching(&open)?;
        let ensure = serde_json::json!({"type":"ensureWorktree", "path":payload["worktree"], "trusted":true});
        let ready = exchange(socket, &ensure).await?;
        route.observe(&ensure, &ready)?;
        ensure!(
            ready["state"] == "ready",
            "agent Code worktree is not ready"
        );
        let response = exchange(socket, &open).await?;
        route.observe(&open, &response)?;
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(cpu: u64, id: usize) -> Observation {
        Observation {
            _runtimes: vec![],
            samples: BTreeMap::from([(
                id,
                ResourceSample {
                    processes: 2,
                    rss_bytes: 100,
                    cpu_usage_usec: cpu,
                },
            )]),
        }
    }

    #[test]
    fn cpu_is_a_fresh_original_runtime_delta_and_all_dimensions_gate() {
        let before = observation(1_000_000, 1);
        let after = observation(1_500_000, 1);
        let totals = after.since(&before, Duration::from_millis(250)).unwrap();
        assert_eq!(totals.cpu_millicores, 2000);
        let policy = Policy {
            rss_bytes: 101,
            processes: 3,
            cpu_millicores: 2001,
        };
        assert!(policy.fits(totals));
        assert!(
            !Policy {
                rss_bytes: 100,
                ..policy
            }
            .fits(totals)
        );
        assert!(
            !Policy {
                processes: 2,
                ..policy
            }
            .fits(totals)
        );
        assert!(
            !Policy {
                cpu_millicores: 2000,
                ..policy
            }
            .fits(totals)
        );
        assert!(before.since(&after, Duration::from_millis(250)).is_err());
        assert!(
            observation(1_500_000, 2)
                .since(&before, Duration::from_millis(250))
                .is_err()
        );
        assert!(after.since(&before, Duration::from_secs(3)).is_err());
        assert!(after.since(&before, Duration::ZERO).is_err());
    }

    #[tokio::test]
    async fn disabled_warming_never_selects_or_launches_a_runtime() {
        let host = CodeRuntimeHost::default();
        let response = host
            .acquire_agent(
                "zed",
                &serde_json::json!({"type":"acquireAgentBuffer", "worktree":"/unused", "path":"file.rs", "leaseId":"agent"}),
                None,
                Policy {
                    rss_bytes: 0,
                    processes: 32,
                    cpu_millicores: 2000,
                },
                || panic!("disabled admission cannot launch"),
            )
            .await
            .unwrap();
        assert_eq!(response["state"], "saturated");
        assert_eq!(response["resources"]["rss_bytes"], 0);
    }

    #[tokio::test]
    async fn unowned_legacy_resources_are_unknown_not_zero() {
        let host = CodeRuntimeHost::default();
        host.routes.lock().await.insert(
            ("zed".into(), "/work".into()),
            Arc::new(Mutex::new(WorktreeRoute {
                target: Some(CodeTarget::Legacy("/not-a-live-socket".into())),
                ..Default::default()
            })),
        );
        let response = host
            .acquire_agent(
                "zed",
                &serde_json::json!({"type":"acquireAgentBuffer", "worktree":"/unused", "path":"file.rs", "leaseId":"agent"}),
                None,
                Policy::from_config(None),
                || panic!("unknown resources cannot launch"),
            )
            .await
            .unwrap();
        assert_eq!(response["state"], "unavailable");
        assert!(response["resources"].is_null());
    }

    #[tokio::test]
    #[ignore = "requires delegated writable cgroup v2"]
    async fn real_resource_budget_refuses_warming_and_preserves_interactive_access() {
        let root = PrivateRuntimeDirectory::create().unwrap();
        let host = CodeRuntimeHost::default();
        let plan = super::super::tests::fixture_plan(&root.0, "resources");
        let payload = serde_json::json!({"type":"acquireAgentBuffer", "worktree":root.0, "path":"file.rs", "leaseId":"cowboy-agent-fixture"});
        let opened = host
            .acquire_agent(
                "fixture-code",
                &payload,
                Some("resources".into()),
                Policy::from_config(None),
                || Ok(CodeRuntimeSelection::Installed(plan.clone())),
            )
            .await
            .unwrap();
        assert_eq!(opened["type"], "buffer", "{opened}");
        let observed = host.resource_observation().await.unwrap().totals().unwrap();
        assert!(observed.processes >= 2 && observed.rss_bytes > 0);
        let denied = host
            .acquire_agent(
                "fixture-code",
                &payload,
                Some("resources".into()),
                Policy {
                    processes: 1,
                    ..Policy::from_config(None)
                },
                || panic!("saturated admission cannot select a runtime"),
            )
            .await
            .unwrap();
        assert_eq!(denied["state"], "saturated");
        assert!(denied["resources"]["rss_bytes"].as_u64().unwrap() > 0);
        let mut interactive = payload.clone();
        interactive["type"] = serde_json::json!("openBuffer");
        interactive["leaseId"] = serde_json::json!("interactive-tab");
        let opened = host
            .request_active(
                "fixture-code",
                &interactive,
                Some("resources".into()),
                || panic!("original route must survive saturation"),
            )
            .await
            .unwrap();
        assert_eq!(opened["type"], "buffer");
    }
}
