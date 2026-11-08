//! Host requirements of a Machine browser runtime: what the host must still
//! provide once the artifact is the instance's whole userland, plus the
//! confining `AppArmor` policy the Machine never weakens or substitutes.

use std::path::{Path, PathBuf};

use cowboy_plugin_sdk::{MachineBrowserHostRequirements, parse_kernel_release};

/// Profile the launcher executable attaches to (grants `userns` and the exact
/// mounts of the launcher contract).
pub const BOOTSTRAP_PROFILE: &str = "cowboy-browser-bootstrap";
/// Profile every display and browser process runs under; reached only by
/// `change_profile` from the bootstrap profile.
pub const SEALED_PROFILE: &str = "cowboy-browser-sealed";

const APPARMOR_ROOT: &str = "sys/kernel/security/apparmor";

/// Why the host cannot run a browser. The detail names the first missing
/// requirement for the Machine log; the Controller only learns the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unmet(pub String);

/// Check the host rooted at `/`.
///
/// # Errors
/// Returns the first unmet requirement.
pub fn check(requirements: &MachineBrowserHostRequirements) -> Result<(), Unmet> {
    check_in(Path::new("/"), requirements)
}

/// Check a host view rooted at `root` (tests build synthetic trees).
///
/// # Errors
/// Returns the first unmet requirement.
pub fn check_in(root: &Path, requirements: &MachineBrowserHostRequirements) -> Result<(), Unmet> {
    let release = std::fs::read_to_string(root.join("proc/sys/kernel/osrelease"))
        .map_err(|error| Unmet(format!("kernel release unreadable: {error}")))?;
    let running = parse_kernel_release(release.trim()).ok_or_else(|| {
        Unmet(format!(
            "kernel release {:?} is not parseable",
            release.trim()
        ))
    })?;
    let minimum = parse_kernel_release(&requirements.kernel_min)
        .ok_or_else(|| Unmet("contract kernel_min is not parseable".to_owned()))?;
    if running < minimum {
        return Err(Unmet(format!(
            "kernel {}.{} is older than the required {}.{}",
            running.0, running.1, minimum.0, minimum.1
        )));
    }
    if requirements.merged_usr {
        for (link, target) in [
            ("bin", "usr/bin"),
            ("lib", "usr/lib"),
            ("lib64", "usr/lib64"),
        ] {
            let path = root.join(link);
            let resolved = std::fs::read_link(&path)
                .map_err(|_| Unmet(format!("/{link} is not a merged-usr symlink")))?;
            let normalized: PathBuf = resolved
                .components()
                .filter(|component| !matches!(component, std::path::Component::CurDir))
                .collect();
            if normalized != Path::new(target) && normalized != Path::new("/").join(target) {
                return Err(Unmet(format!(
                    "/{link} resolves to {} instead of {target}",
                    resolved.display()
                )));
            }
        }
    }
    for mount_point in &requirements.mount_points {
        let relative = mount_point.trim_start_matches('/');
        if !root.join(relative).is_dir() {
            return Err(Unmet(format!("mount point {mount_point} is missing")));
        }
    }
    let apparmor = root.join(APPARMOR_ROOT);
    for feature in &requirements.apparmor_features {
        if !apparmor.join("features").join(feature).exists() {
            return Err(Unmet(format!("AppArmor feature {feature} is unavailable")));
        }
    }
    let profiles = std::fs::read_to_string(apparmor.join("profiles"))
        .map_err(|_| Unmet("AppArmor profile list is unreadable".to_owned()))?;
    for profile in [BOOTSTRAP_PROFILE, SEALED_PROFILE] {
        if !profile_enforced(&profiles, profile) {
            return Err(Unmet(format!(
                "AppArmor profile {profile} is not loaded in enforce mode"
            )));
        }
    }
    if !root.join("sys/fs/cgroup/cgroup.controllers").is_file() {
        return Err(Unmet("cgroup v2 is unavailable".to_owned()));
    }
    Ok(())
}

/// `/sys/kernel/security/apparmor/profiles` lists `name (mode)` per line.
fn profile_enforced(profiles: &str, name: &str) -> bool {
    profiles.lines().any(|line| {
        line.rsplit_once(" (")
            .is_some_and(|(listed, mode)| listed == name && mode.trim_end() == "enforce)")
    })
}

/// The enforce-mode label a process under `profile` shows in
/// `/proc/<pid>/attr/current`.
#[must_use]
pub fn enforce_label(profile: &str) -> String {
    format!("{profile} (enforce)")
}

#[cfg(test)]
mod tests {
    use super::*;
    use cowboy_plugin_sdk::MachineBrowserBuiltFrom;
    use std::os::unix::fs::symlink;

    fn requirements() -> MachineBrowserHostRequirements {
        MachineBrowserHostRequirements {
            kernel_min: "6.8".into(),
            merged_usr: true,
            mount_points: vec!["/mnt".into(), "/opt".into()],
            apparmor_features: vec!["network_v8".into(), "mount".into()],
            built_from: MachineBrowserBuiltFrom {
                id: "ubuntu".into(),
                version_id: "26.04".into(),
            },
        }
    }

    fn synthetic_host(root: &Path, profiles: &str) {
        std::fs::create_dir_all(root.join("proc/sys/kernel")).unwrap();
        std::fs::write(
            root.join("proc/sys/kernel/osrelease"),
            "6.17.0-14-generic\n",
        )
        .unwrap();
        std::fs::create_dir_all(root.join("usr/bin")).unwrap();
        std::fs::create_dir_all(root.join("usr/lib")).unwrap();
        std::fs::create_dir_all(root.join("usr/lib64")).unwrap();
        symlink("usr/bin", root.join("bin")).unwrap();
        symlink("usr/lib", root.join("lib")).unwrap();
        symlink("usr/lib64", root.join("lib64")).unwrap();
        std::fs::create_dir_all(root.join("mnt")).unwrap();
        std::fs::create_dir_all(root.join("opt")).unwrap();
        let apparmor = root.join(APPARMOR_ROOT);
        std::fs::create_dir_all(apparmor.join("features/network_v8")).unwrap();
        std::fs::create_dir_all(apparmor.join("features/mount")).unwrap();
        std::fs::write(apparmor.join("profiles"), profiles).unwrap();
        std::fs::create_dir_all(root.join("sys/fs/cgroup")).unwrap();
        std::fs::write(
            root.join("sys/fs/cgroup/cgroup.controllers"),
            "memory pids\n",
        )
        .unwrap();
    }

    #[test]
    fn a_complete_host_passes_and_each_missing_piece_is_named() {
        let temp = tempfile::tempdir().unwrap();
        synthetic_host(
            temp.path(),
            "cowboy-browser-bootstrap (enforce)\ncowboy-browser-sealed (enforce)\nother (complain)\n",
        );
        check_in(temp.path(), &requirements()).unwrap();

        let complain = tempfile::tempdir().unwrap();
        synthetic_host(
            complain.path(),
            "cowboy-browser-bootstrap (enforce)\ncowboy-browser-sealed (complain)\n",
        );
        let error = check_in(complain.path(), &requirements()).unwrap_err();
        assert!(error.0.contains("cowboy-browser-sealed"), "{error:?}");

        let old = tempfile::tempdir().unwrap();
        synthetic_host(old.path(), "");
        std::fs::write(old.path().join("proc/sys/kernel/osrelease"), "5.15.0\n").unwrap();
        let error = check_in(old.path(), &requirements()).unwrap_err();
        assert!(error.0.contains("older"), "{error:?}");

        let unmerged = tempfile::tempdir().unwrap();
        synthetic_host(unmerged.path(), "");
        std::fs::remove_file(unmerged.path().join("lib64")).unwrap();
        std::fs::create_dir(unmerged.path().join("lib64")).unwrap();
        let error = check_in(unmerged.path(), &requirements()).unwrap_err();
        assert!(error.0.contains("lib64"), "{error:?}");

        let mut without_feature = requirements();
        without_feature.apparmor_features.push("ptrace_v9".into());
        let error = check_in(temp.path(), &without_feature).unwrap_err();
        assert!(error.0.contains("ptrace_v9"), "{error:?}");
    }

    #[test]
    fn labels_follow_the_kernel_format() {
        assert_eq!(
            enforce_label(SEALED_PROFILE),
            "cowboy-browser-sealed (enforce)"
        );
        assert!(profile_enforced("a (enforce)\nb (enforce)", "b"));
        assert!(!profile_enforced("ab (enforce)", "b"));
    }
}
