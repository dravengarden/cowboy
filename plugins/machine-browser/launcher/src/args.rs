//! The fixed argument vector of `run`. The Machine passes no shell or
//! template arguments; every value is bounded here before the supervisor
//! forks, so nothing later in the launcher re-validates them.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

/// Smallest framebuffer the launcher accepts, at open and on `resize`.
pub const MIN_WIDTH: u32 = 320;
pub const MIN_HEIGHT: u32 = 480;
/// Largest framebuffer the launcher accepts; the Machine applies its own,
/// smaller `browser.max_geometry` on top.
pub const MAX_WIDTH: u32 = 4096;
pub const MAX_HEIGHT: u32 = 4096;
const MIN_BYTES: u64 = 1 << 20;
const MAX_BYTES: u64 = 64 << 30;
const DEFAULT_TMP_BYTES: u64 = 256 << 20;
const MAX_DISPLAY: u32 = 9999;
const MAX_PROFILE_NAME: usize = 128;
const MAX_PATH: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// Host directory holding `control.sock` and `rfb.sock` (the unit's
    /// `RuntimeDirectory`).
    pub control_dir: PathBuf,
    /// Extracted artifact root containing `usr/` and `opt/`.
    pub runtime_dir: PathBuf,
    /// Published snapshot generation to restore, read-only.
    pub snapshot_dir: Option<PathBuf>,
    /// Account staging root; `gen-<n>` below it receives snapshot copies.
    pub staging_dir: PathBuf,
    pub display: u32,
    pub width: u32,
    pub height: u32,
    pub scale: u32,
    pub profile_bytes: u64,
    pub instance_bytes: u64,
    pub tmp_bytes: u64,
    /// Artifact-relative managed policy file.
    pub policy_file: PathBuf,
    /// Absolute managed-policy directory of the packaged browser build.
    pub policy_directory: PathBuf,
    pub browser: PathBuf,
    pub display_server: PathBuf,
    pub window_manager: PathBuf,
    pub xrandr: PathBuf,
    pub xkbcomp: PathBuf,
    pub xauth: PathBuf,
    /// `AppArmor` profile the children transition to; absent means no
    /// transition (conformance without `AppArmor`).
    pub sealed_profile: Option<String>,
    /// The unit's cgroup directory; absent means derive it from
    /// `/proc/self/cgroup`.
    pub cgroup_dir: Option<PathBuf>,
    /// Extra host paths masked with an empty read-only tmpfs.
    pub masks: Vec<PathBuf>,
}

impl Args {
    /// Parse the arguments following `run`.
    pub fn parse(argv: &[String]) -> Result<Self, String> {
        let mut values: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        let mut index = 0;
        while index < argv.len() {
            let flag = argv[index].as_str();
            let Some(name) = flag.strip_prefix("--") else {
                return Err(format!("unexpected argument {flag:?}"));
            };
            let value = argv
                .get(index + 1)
                .ok_or_else(|| format!("{flag} requires a value"))?;
            if value.starts_with("--") {
                return Err(format!("{flag} requires a value"));
            }
            if !KNOWN.contains(&name) {
                return Err(format!("unknown flag {flag}"));
            }
            let repeatable = name == "mask";
            let entry = values.entry(name).or_default();
            if !repeatable && !entry.is_empty() {
                return Err(format!("{flag} given twice"));
            }
            entry.push(value.as_str());
            index += 2;
        }
        let single = |name: &str| -> Result<&str, String> {
            values
                .get(name)
                .and_then(|list| list.first().copied())
                .ok_or_else(|| format!("--{name} is required"))
        };
        let optional = |name: &str| values.get(name).and_then(|list| list.first().copied());
        let (width, height) = parse_geometry(single("geometry")?)
            .ok_or_else(|| "--geometry must be WxH within 320x480..4096x4096".to_owned())?;
        let scale: u32 = single("scale")?
            .parse()
            .ok()
            .filter(|scale| matches!(scale, 1 | 2))
            .ok_or_else(|| "--scale must be 1 or 2".to_owned())?;
        let sealed_profile = match optional("sealed-profile") {
            Some(name) => Some(parse_profile(name)?),
            None => None,
        };
        let mut masks = Vec::new();
        for mask in values.get("mask").into_iter().flatten() {
            masks.push(absolute_normal(mask, "--mask")?);
        }
        Ok(Self {
            control_dir: absolute_normal(single("control-dir")?, "--control-dir")?,
            runtime_dir: absolute_normal(single("runtime-dir")?, "--runtime-dir")?,
            snapshot_dir: optional("snapshot-dir")
                .map(|path| absolute_normal(path, "--snapshot-dir"))
                .transpose()?,
            staging_dir: absolute_normal(single("staging-dir")?, "--staging-dir")?,
            display: parse_display(single("display")?)?,
            width,
            height,
            scale,
            profile_bytes: parse_bytes(single("profile-bytes")?, "--profile-bytes")?,
            instance_bytes: parse_bytes(single("instance-bytes")?, "--instance-bytes")?,
            tmp_bytes: optional("tmp-bytes")
                .map(|bytes| parse_bytes(bytes, "--tmp-bytes"))
                .transpose()?
                .unwrap_or(DEFAULT_TMP_BYTES),
            policy_file: relative_normal(single("policy-file")?, "--policy-file")?,
            policy_directory: absolute_normal(single("policy-directory")?, "--policy-directory")?,
            browser: relative_normal(single("browser")?, "--browser")?,
            display_server: relative_normal(single("display-server")?, "--display-server")?,
            window_manager: relative_normal(single("window-manager")?, "--window-manager")?,
            xrandr: relative_normal(single("xrandr")?, "--xrandr")?,
            xkbcomp: relative_normal(single("xkbcomp")?, "--xkbcomp")?,
            xauth: relative_normal(single("xauth")?, "--xauth")?,
            sealed_profile,
            cgroup_dir: optional("cgroup-dir")
                .map(|path| absolute_normal(path, "--cgroup-dir"))
                .transpose()?,
            masks,
        })
    }

    /// `:N`.
    #[must_use]
    pub fn display_name(&self) -> String {
        format!(":{}", self.display)
    }

    /// `WxH`.
    #[must_use]
    pub fn geometry(&self) -> String {
        format!("{}x{}", self.width, self.height)
    }

    /// `W,H`, the form Chromium's `--window-size` takes.
    #[must_use]
    pub fn window_size(&self) -> String {
        format!("{},{}", self.width, self.height)
    }

    /// Absolute path of an artifact-relative file once `usr/` and `opt/`
    /// are the instance's `/usr` and `/opt`.
    #[must_use]
    pub fn instance_path(relative: &Path) -> String {
        format!("/{}", relative.display())
    }
}

const KNOWN: &[&str] = &[
    "control-dir",
    "runtime-dir",
    "snapshot-dir",
    "staging-dir",
    "display",
    "geometry",
    "scale",
    "profile-bytes",
    "instance-bytes",
    "tmp-bytes",
    "policy-file",
    "policy-directory",
    "browser",
    "display-server",
    "window-manager",
    "xrandr",
    "xkbcomp",
    "xauth",
    "sealed-profile",
    "cgroup-dir",
    "mask",
];

/// Whether a framebuffer size is inside the launcher's bounds.
#[must_use]
pub fn geometry_allowed(width: u32, height: u32) -> bool {
    (MIN_WIDTH..=MAX_WIDTH).contains(&width) && (MIN_HEIGHT..=MAX_HEIGHT).contains(&height)
}

/// `WxH` within the bounds.
#[must_use]
pub fn parse_geometry(text: &str) -> Option<(u32, u32)> {
    let (width, height) = text.split_once('x')?;
    let width: u32 = width.parse().ok()?;
    let height: u32 = height.parse().ok()?;
    geometry_allowed(width, height).then_some((width, height))
}

fn parse_display(text: &str) -> Result<u32, String> {
    text.strip_prefix(':')
        .and_then(|number| number.parse::<u32>().ok())
        .filter(|number| *number <= MAX_DISPLAY)
        .ok_or_else(|| "--display must be :N with N up to 9999".to_owned())
}

fn parse_bytes(text: &str, flag: &str) -> Result<u64, String> {
    text.parse::<u64>()
        .ok()
        .filter(|bytes| (MIN_BYTES..=MAX_BYTES).contains(bytes))
        .ok_or_else(|| format!("{flag} must be a byte count between 1 MiB and 64 GiB"))
}

fn parse_profile(name: &str) -> Result<String, String> {
    if name.is_empty()
        || name.len() > MAX_PROFILE_NAME
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && byte != b'"' && byte != b'\\')
    {
        return Err("--sealed-profile must be a printable AppArmor profile name".to_owned());
    }
    Ok(name.to_owned())
}

fn checked(path: &str, flag: &str) -> Result<(), String> {
    if path.is_empty() || path.len() > MAX_PATH || path.contains('\0') || path.ends_with('/') {
        return Err(format!("{flag} must be a normalized path"));
    }
    Ok(())
}

fn relative_normal(path: &str, flag: &str) -> Result<PathBuf, String> {
    checked(path, flag)?;
    let parsed = Path::new(path);
    if !parsed
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "{flag} must be a relative path without . or .. components"
        ));
    }
    Ok(parsed.to_path_buf())
}

fn absolute_normal(path: &str, flag: &str) -> Result<PathBuf, String> {
    checked(path, flag)?;
    let parsed = Path::new(path);
    let mut components = parsed.components();
    if components.next() != Some(Component::RootDir)
        || !components.all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "{flag} must be an absolute path without . or .. components"
        ));
    }
    Ok(parsed.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(pairs: &[(&str, &str)]) -> Vec<String> {
        pairs
            .iter()
            .flat_map(|(flag, value)| [format!("--{flag}"), (*value).to_owned()])
            .collect()
    }

    fn complete() -> Vec<(&'static str, &'static str)> {
        vec![
            ("control-dir", "/run/user/1000/cowboy-browser/i1"),
            ("runtime-dir", "/state/plugins/x/content"),
            ("staging-dir", "/state/accounts/a/browser/staging"),
            ("display", ":7"),
            ("geometry", "1280x800"),
            ("scale", "2"),
            ("profile-bytes", "268435456"),
            ("instance-bytes", "134217728"),
            ("policy-file", "usr/share/cowboy-browser/policy.json"),
            ("policy-directory", "/etc/opt/chrome/policies/managed"),
            ("browser", "opt/google/chrome/chrome"),
            ("display-server", "usr/bin/Xtigervnc"),
            ("window-manager", "usr/bin/matchbox-window-manager"),
            ("xrandr", "usr/bin/xrandr"),
            ("xkbcomp", "usr/bin/xkbcomp"),
            ("xauth", "usr/bin/xauth"),
        ]
    }

    #[test]
    fn a_complete_vector_parses_with_defaults_and_options() {
        let args = Args::parse(&argv(&complete())).unwrap();
        assert_eq!(args.display_name(), ":7");
        assert_eq!(args.geometry(), "1280x800");
        assert_eq!(args.window_size(), "1280,800");
        assert_eq!(args.tmp_bytes, 256 << 20);
        assert_eq!(args.scale, 2);
        assert!(args.snapshot_dir.is_none());
        assert!(args.sealed_profile.is_none());
        assert!(args.masks.is_empty());
        assert_eq!(
            Args::instance_path(&args.browser),
            "/opt/google/chrome/chrome"
        );

        let mut pairs = complete();
        pairs.extend([
            ("snapshot-dir", "/state/accounts/a/browser/snapshots/gen-3"),
            ("sealed-profile", "cowboy-browser-sealed"),
            ("cgroup-dir", "/sys/fs/cgroup/user.slice/u.service"),
            ("mask", "/state"),
            ("mask", "/srv/other"),
            ("tmp-bytes", "67108864"),
        ]);
        let args = Args::parse(&argv(&pairs)).unwrap();
        assert_eq!(
            args.sealed_profile.as_deref(),
            Some("cowboy-browser-sealed")
        );
        assert_eq!(args.masks.len(), 2);
        assert_eq!(args.tmp_bytes, 64 << 20);
        assert_eq!(
            args.snapshot_dir.as_deref(),
            Some(Path::new("/state/accounts/a/browser/snapshots/gen-3"))
        );
    }

    #[test]
    fn every_bound_and_shape_is_enforced() {
        let required = complete();
        for (index, _) in required.iter().enumerate() {
            let mut pairs = required.clone();
            pairs.remove(index);
            assert!(Args::parse(&argv(&pairs)).is_err(), "missing {index}");
        }
        let replace = |flag: &str, value: &'static str| {
            let mut pairs = complete();
            for pair in &mut pairs {
                if pair.0 == flag {
                    pair.1 = value;
                }
            }
            Args::parse(&argv(&pairs))
        };
        assert!(replace("geometry", "319x800").is_err());
        assert!(replace("geometry", "1280x4097").is_err());
        assert!(replace("geometry", "1280").is_err());
        assert!(replace("scale", "3").is_err());
        assert!(replace("display", "7").is_err());
        assert!(replace("display", ":10000").is_err());
        assert!(replace("profile-bytes", "1024").is_err());
        assert!(replace("browser", "/opt/google/chrome/chrome").is_err());
        assert!(replace("browser", "opt/../chrome").is_err());
        assert!(replace("policy-directory", "etc/opt/chrome").is_err());
        assert!(replace("control-dir", "/run/user/1000/").is_err());
        let mut pairs = complete();
        pairs.push(("display", ":8"));
        assert!(Args::parse(&argv(&pairs)).is_err());
        let mut pairs = complete();
        pairs.push(("sealed-profile", "has space"));
        assert!(Args::parse(&argv(&pairs)).is_err());
        let mut pairs = complete();
        pairs.push(("unknown", "x"));
        assert!(Args::parse(&argv(&pairs)).is_err());
        let mut dangling = argv(&complete());
        dangling.push("--mask".to_owned());
        assert!(Args::parse(&dangling).is_err());
        assert!(geometry_allowed(320, 480));
        assert!(!geometry_allowed(4097, 480));
        assert_eq!(parse_geometry("640x480"), Some((640, 480)));
    }
}
