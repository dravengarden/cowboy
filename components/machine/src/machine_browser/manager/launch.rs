//! Launch arguments and the profile compatibility fence.

use super::*;

/// Fixed X display number inside each instance's private namespaces.
const DISPLAY: &str = ":7";
const TMP_BYTES: u64 = 256 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum VersionOrder {
    Unrecorded,
    Same,
    RuntimeNewer,
    RecordNewer,
}

/// The profile compatibility fence compares full browser versions, not
/// majors: an older build of the same major may not read a newer profile.
pub(super) fn compare_versions(recorded: Option<&str>, runtime: &str) -> VersionOrder {
    let Some(recorded) = recorded else {
        return VersionOrder::Unrecorded;
    };
    let (Some(recorded), Some(runtime)) = (
        cowboy_plugin_sdk::parse_browser_version(recorded),
        cowboy_plugin_sdk::parse_browser_version(runtime),
    ) else {
        // Unparseable versions are compared textually and never silently
        // treated as compatible.
        return if recorded == runtime {
            VersionOrder::Same
        } else {
            VersionOrder::RecordNewer
        };
    };
    match recorded.cmp(&runtime) {
        std::cmp::Ordering::Less => VersionOrder::RuntimeNewer,
        std::cmp::Ordering::Equal => VersionOrder::Same,
        std::cmp::Ordering::Greater => VersionOrder::RecordNewer,
    }
}

/// Least gap kept between a unit's `MemoryHigh` and its `MemoryMax`.
const MEMORY_HIGH_GAP: u64 = 64 << 20;
const MIB: u64 = 1 << 20;

/// Unit limits for an admitted memory `budget`: `MemoryMax` is the budget
/// itself, so the unit can never take shared memory beyond what admission
/// granted, and `MemoryHigh` throttles it an eighth (at least 64 MiB) below,
/// never above `browser.memory_high`.
pub(super) fn admitted_limits(settings: &Settings, budget: u64) -> Limits {
    let throttle = budget.saturating_sub((budget / 8).max(MEMORY_HIGH_GAP));
    Limits {
        memory_max: budget,
        memory_high: (throttle - throttle % MIB).min(settings.memory_high),
        cpu_quota_percent: settings.cpu_quota_percent,
        tasks_max: settings.tasks_max,
        max_runtime: settings.max_runtime,
    }
}

/// Argument vector of `cowboy-browser-launcher run` for one start.
pub(super) fn launch_arguments(
    plan: &MachineBrowserRuntimePlan,
    settings: &Settings,
    runtime_dir: &Path,
    staging_root: &Path,
    snapshot_dir: Option<&Path>,
    viewport: Viewport,
    state_dir: &Path,
) -> Result<Vec<OsString>> {
    let runtime = &plan.contract.runtime;
    let helper = |name: &str| -> Result<&str> {
        runtime
            .helpers
            .iter()
            .find(|helper| {
                Path::new(helper)
                    .file_name()
                    .is_some_and(|file| file == name)
            })
            .map(String::as_str)
            .with_context(|| format!("Machine browser runtime has no {name} helper"))
    };
    let mut arguments: Vec<OsString> = vec!["run".into()];
    let mut push = |flag: &str, value: &dyn AsRef<std::ffi::OsStr>| {
        arguments.push(flag.into());
        arguments.push(value.as_ref().to_owned());
    };
    push("--control-dir", &runtime_dir);
    push("--runtime-dir", &plan.runtime_root);
    if let Some(snapshot_dir) = snapshot_dir {
        push("--snapshot-dir", &snapshot_dir);
    }
    push("--staging-dir", &staging_root);
    push("--display", &DISPLAY);
    push(
        "--geometry",
        &format!("{}x{}", viewport.width, viewport.height),
    );
    push("--scale", &viewport.scale.to_string());
    push("--profile-bytes", &settings.profile_max_bytes.to_string());
    push("--instance-bytes", &settings.instance_max_bytes.to_string());
    push("--tmp-bytes", &TMP_BYTES.to_string());
    push("--policy-file", &runtime.policy.file);
    push("--policy-directory", &runtime.policy.directory);
    push("--browser", &runtime.browser);
    push("--display-server", &runtime.display_server);
    push("--window-manager", &helper("matchbox-window-manager")?);
    push("--xrandr", &helper("xrandr")?);
    push("--xkbcomp", &helper("xkbcomp")?);
    push("--xauth", &helper("xauth")?);
    push("--sealed-profile", &SEALED_PROFILE);
    push("--mask", &state_dir);
    Ok(arguments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_versions_compare_fully_and_unparseable_ones_never_pass() {
        assert_eq!(
            compare_versions(None, "143.0.7499.40"),
            VersionOrder::Unrecorded
        );
        assert_eq!(
            compare_versions(Some("143.0.7499.40"), "143.0.7499.40"),
            VersionOrder::Same
        );
        assert_eq!(
            compare_versions(Some("143.0.7499.40"), "143.0.7499.41"),
            VersionOrder::RuntimeNewer
        );
        assert_eq!(
            compare_versions(Some("143.0.7499.41"), "143.0.7499.40"),
            VersionOrder::RecordNewer
        );
        assert_eq!(
            compare_versions(Some("144.0.1.1"), "143.0.7499.40"),
            VersionOrder::RecordNewer
        );
        assert_eq!(
            compare_versions(Some("weird"), "143.0.1.1"),
            VersionOrder::RecordNewer
        );
    }

    #[test]
    fn unit_limits_follow_the_admitted_budget() {
        let settings = Settings::from_config(None);
        // Ample headroom: the configured limits.
        let ample = admitted_limits(&settings, settings.memory_max);
        assert_eq!(ample.memory_max, 2 << 30);
        assert_eq!(ample.memory_high, 1536 << 20);
        // A tight budget throttles an eighth below its own MemoryMax.
        let tight = admitted_limits(&settings, 1280 * MIB);
        assert_eq!(tight.memory_max, 1280 * MIB);
        assert_eq!(tight.memory_high, 1120 * MIB);
        // The smallest admissible budget (the default reserve).
        let least = admitted_limits(&settings, settings.memory_reserve);
        assert_eq!(least.memory_max, 768 * MIB);
        assert_eq!(least.memory_high, 672 * MIB);
        // The 64 MiB gap applies below 512 MiB; never 0 or above MemoryMax.
        let small = admitted_limits(&settings, 256 * MIB);
        assert_eq!(small.memory_high, 192 * MIB);
        for budget in [256 * MIB, 1000 * MIB + 7, 6 << 30] {
            let limits = admitted_limits(&settings, budget);
            assert!(limits.memory_high > 0 && limits.memory_high < limits.memory_max);
            assert!(limits.memory_high <= settings.memory_high);
            assert_eq!(limits.cpu_quota_percent, settings.cpu_quota_percent);
            assert_eq!(limits.tasks_max, settings.tasks_max);
            assert_eq!(limits.max_runtime, settings.max_runtime);
        }
        // The unit is started with exactly these limits.
        let launch = Launch {
            unit: "cowboy-browser-inst",
            instance_id: "inst",
            limits: tight,
            launcher: Path::new("/launcher"),
            arguments: Vec::new(),
        };
        let arguments: Vec<String> = unit::systemd_run_arguments(&launch)
            .into_iter()
            .map(|argument| argument.into_string().unwrap())
            .collect();
        for expected in [
            format!("--property=MemoryMax={}", 1280 * MIB),
            format!("--property=MemoryHigh={}", 1120 * MIB),
            "--property=MemorySwapMax=0".to_owned(),
            "--property=OOMScoreAdjust=1000".to_owned(),
        ] {
            assert!(arguments.contains(&expected), "{expected}");
        }
    }

    #[test]
    fn launch_arguments_follow_the_contract_and_name_every_helper() {
        let contract: cowboy_plugin_sdk::MachineBrowserContract = serde_json::from_str(
            &std::fs::read_to_string(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../plugins/machine-browser/contract.json"),
            )
            .unwrap(),
        )
        .unwrap();
        let plan = MachineBrowserRuntimePlan {
            plugin_id: "machine-browser".into(),
            plugin_version: contract.version.clone(),
            generation_digest: "sha256:aa".into(),
            command: "cowboy-browser-launcher".into(),
            runtime_root: PathBuf::from("/state/plugins/machine-browser/x/content"),
            launcher: PathBuf::from(
                "/state/plugins/machine-browser/x/content/usr/libexec/cowboy-browser-launcher",
            ),
            contract,
        };
        let arguments = launch_arguments(
            &plan,
            &Settings::from_config(None),
            Path::new("/run/user/1000/cowboy-browser/inst-1"),
            Path::new("/state/provider-auth/accounts/a/browser/staging"),
            Some(Path::new(
                "/state/provider-auth/accounts/a/browser/snapshots/gen-3",
            )),
            Viewport {
                width: 1280,
                height: 800,
                scale: 2,
            },
            Path::new("/state"),
        )
        .unwrap();
        let text: Vec<String> = arguments
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect();
        assert_eq!(text[0], "run");
        for flag in [
            "--control-dir",
            "--runtime-dir",
            "--snapshot-dir",
            "--staging-dir",
            "--geometry",
            "--scale",
            "--profile-bytes",
            "--policy-file",
            "--policy-directory",
            "--browser",
            "--display-server",
            "--window-manager",
            "--xrandr",
            "--xkbcomp",
            "--xauth",
            "--sealed-profile",
            "--mask",
        ] {
            let position = text
                .iter()
                .position(|argument| argument == flag)
                .unwrap_or_else(|| panic!("{flag}"));
            assert!(!text[position + 1].starts_with("--"), "{flag} has a value");
        }
        let geometry = text
            .iter()
            .position(|argument| argument == "--geometry")
            .unwrap();
        assert_eq!(text[geometry + 1], "1280x800");
        let sealed = text
            .iter()
            .position(|argument| argument == "--sealed-profile")
            .unwrap();
        assert_eq!(text[sealed + 1], SEALED_PROFILE);
    }
}
