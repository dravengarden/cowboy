//! Readers and parsers for the `/proc` facts the launcher reports (start
//! times, namespace identities, uid maps, cgroup membership, the mount table,
//! `AppArmor` labels) and for the host facts copied into the `/etc` scaffold
//! (`resolv.conf`, the timezone).

use std::io;
use std::path::Path;

/// Field 22 of `/proc/<pid>/stat`: start time in clock ticks since boot.
/// The value is boot-relative, so it reads the same from every pid namespace.
#[must_use]
pub fn start_time(stat: &str) -> Option<u64> {
    // The command name is parenthesised and may contain spaces; fields after
    // it start at field 3 (state), so starttime (field 22) is index 19.
    let after_comm = stat.rsplit_once(')')?.1;
    after_comm.split_whitespace().nth(19)?.parse().ok()
}

/// Start time of `pid` (`None` for the calling process), read through the
/// `/proc` the caller sees.
pub fn read_start_time(pid: Option<i32>) -> io::Result<u64> {
    let path = match pid {
        Some(pid) => format!("/proc/{pid}/stat"),
        None => "/proc/self/stat".to_owned(),
    };
    let stat = std::fs::read_to_string(&path)?;
    start_time(&stat)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("{path} is malformed")))
}

/// `/proc/self/uid_map` with its padding removed: one `inside outside count`
/// triple per line.
#[must_use]
pub fn normalize_map(content: &str) -> String {
    content
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// The unified-hierarchy path of `/proc/self/cgroup` (`0::/path`).
#[must_use]
pub fn cgroup_path(content: &str) -> Option<String> {
    content
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .map(|path| path.trim().to_owned())
        .filter(|path| path.starts_with('/'))
}

/// `readlink(/proc/<who>/ns/<name>)`, e.g. `user:[4026532641]`.
pub fn read_namespace(who: &str, name: &str) -> io::Result<String> {
    let link = std::fs::read_link(format!("/proc/{who}/ns/{name}"))?;
    Ok(link.to_string_lossy().into_owned())
}

/// The confinement label of `who`, or `unconfined` where the kernel exposes
/// none (no `AppArmor`: reading the attribute fails with `EINVAL`).
#[must_use]
pub fn read_label(who: &str) -> String {
    std::fs::read_to_string(format!("/proc/{who}/attr/current"))
        .map(|label| label.trim_end_matches(['\n', '\0']).to_owned())
        .ok()
        .filter(|label| !label.is_empty())
        .unwrap_or_else(|| "unconfined".to_owned())
}

/// One line of `/proc/<pid>/mountinfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountInfo {
    pub mount_id: u32,
    pub parent_id: u32,
    pub major_minor: String,
    pub root: String,
    pub target: String,
    pub vfs_options: String,
    pub fstype: String,
    pub source: String,
    pub super_options: String,
}

impl MountInfo {
    /// `source[root]` for a bind of a subtree, like `findmnt` shows it.
    #[must_use]
    pub fn source_display(&self) -> String {
        if self.root == "/" {
            self.source.clone()
        } else {
            format!("{}[{}]", self.source, self.root)
        }
    }

    /// Per-mount options followed by the superblock options, so sizes and
    /// read-only flags are both visible.
    #[must_use]
    pub fn options_display(&self) -> String {
        let extra: Vec<&str> = self
            .super_options
            .split(',')
            .filter(|option| !option.is_empty() && *option != "rw" && *option != "ro")
            .collect();
        if extra.is_empty() {
            self.vfs_options.clone()
        } else {
            format!("{},{}", self.vfs_options, extra.join(","))
        }
    }
}

/// Parse `/proc/<pid>/mountinfo`; malformed lines are skipped.
#[must_use]
pub fn parse_mountinfo(text: &str) -> Vec<MountInfo> {
    text.lines()
        .filter_map(|line| {
            let (left, right) = line.split_once(" - ")?;
            let left: Vec<&str> = left.split(' ').collect();
            let right: Vec<&str> = right.splitn(3, ' ').collect();
            if left.len() < 6 || right.len() < 3 {
                return None;
            }
            Some(MountInfo {
                mount_id: left[0].parse().ok()?,
                parent_id: left[1].parse().ok()?,
                major_minor: left[2].to_owned(),
                root: unescape(left[3]),
                target: unescape(left[4]),
                vfs_options: left[5].to_owned(),
                fstype: right[0].to_owned(),
                source: unescape(right[1]),
                super_options: right[2].to_owned(),
            })
        })
        .collect()
}

/// Undo the octal escapes `/proc` uses for space, tab, newline and backslash.
fn unescape(field: &str) -> String {
    let bytes = field.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' && index + 4 <= bytes.len() {
            let digits = &bytes[index + 1..index + 4];
            if digits.iter().all(|digit| (b'0'..=b'7').contains(digit)) {
                let value = digits
                    .iter()
                    .fold(0_u32, |acc, digit| acc * 8 + u32::from(digit - b'0'));
                if let Ok(byte) = u8::try_from(value) {
                    output.push(byte);
                    index += 4;
                    continue;
                }
            }
        }
        output.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&output).into_owned()
}

/// The instance's `resolv.conf`: the host's nameservers and nothing else,
/// plus `options use-vc` so every query goes over TCP.
#[must_use]
pub fn resolv_conf(host: &str) -> String {
    let mut output = String::from("# generated by cowboy-browser-launcher\n");
    for line in host.lines() {
        let line = line.trim();
        let Some(address) = line.strip_prefix("nameserver") else {
            continue;
        };
        let address = address.trim();
        if !address.is_empty()
            && address.len() <= 64
            && address
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() || matches!(byte, b'.' | b':' | b'%'))
        {
            output.push_str("nameserver ");
            output.push_str(address);
            output.push('\n');
        }
    }
    output.push_str("options use-vc\n");
    output
}

/// The `TZ` value the browser receives: the host's own `TZ`, else the zone
/// `/etc/localtime` links to, else `/etc/timezone`, else the bound
/// `/etc/localtime` file itself, else UTC.
#[must_use]
pub fn timezone(
    env_tz: Option<&str>,
    localtime_link: Option<&Path>,
    etc_timezone: Option<&str>,
    localtime_exists: bool,
) -> String {
    if let Some(tz) = env_tz.filter(|tz| valid_tz(tz)) {
        return tz.to_owned();
    }
    if let Some(name) = localtime_link
        .and_then(|link| link.to_str())
        .and_then(|link| link.rsplit_once("zoneinfo/"))
        .map(|(_, name)| name)
        .filter(|name| valid_zone(name))
    {
        return name.to_owned();
    }
    if let Some(name) = etc_timezone.map(str::trim).filter(|name| valid_zone(name)) {
        return name.to_owned();
    }
    if localtime_exists {
        return ":/etc/localtime".to_owned();
    }
    "UTC".to_owned()
}

fn valid_tz(tz: &str) -> bool {
    !tz.is_empty()
        && tz.len() <= 128
        && tz
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && byte != b'"' && byte != b'\\')
}

fn valid_zone(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('/')
        && !name.contains("..")
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'_' | b'-' | b'+'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_start_time_survives_spaces_in_the_command_name() {
        let stat = "1234 (Web Content) S 1 1 1 0 -1 4194560 100 0 0 0 10 5 0 0 20 0 1 0 987654 1000 10 18446744073709551615 1 1 0 0 0 0 0 0 0 0 0 0 17 3 0 0 0 0 0 0 0 0 0 0 0 0 0";
        assert_eq!(start_time(stat), Some(987_654));
        assert_eq!(start_time("garbage"), None);
    }

    #[test]
    fn maps_cgroups_and_mountinfo_parse() {
        assert_eq!(
            normalize_map("      1000       1000          1\n"),
            "1000 1000 1"
        );
        assert_eq!(
            cgroup_path("0::/user.slice/user-1000.slice/cowboy-browser-a.service\n"),
            Some("/user.slice/user-1000.slice/cowboy-browser-a.service".to_owned())
        );
        assert_eq!(cgroup_path("1:name=systemd:/x\n"), None);
        let mounts = parse_mountinfo(
            "36 25 0:32 / /mnt rw,nosuid,nodev,noexec,relatime - tmpfs tmpfs rw,size=1024k,mode=755\n\
             37 36 259:2 /plugins/x/usr /usr ro,nosuid,nodev,relatime - ext4 /dev/nvme0n1p2 rw,errors=remount-ro\n\
             38 36 0:33 / /mnt/with\\040space rw - tmpfs tmpfs rw\nbroken line\n",
        );
        assert_eq!(mounts.len(), 3);
        assert_eq!(mounts[1].target, "/usr");
        assert_eq!(mounts[1].source_display(), "/dev/nvme0n1p2[/plugins/x/usr]");
        assert_eq!(mounts[1].fstype, "ext4");
        assert_eq!(
            mounts[1].options_display(),
            "ro,nosuid,nodev,relatime,errors=remount-ro"
        );
        assert_eq!(
            mounts[0].options_display(),
            "rw,nosuid,nodev,noexec,relatime,size=1024k,mode=755"
        );
        assert_eq!(mounts[2].target, "/mnt/with space");
        assert_eq!(mounts[2].options_display(), "rw");
    }

    #[test]
    fn resolv_conf_keeps_only_nameservers_and_forces_tcp() {
        let generated = resolv_conf(
            "# comment\nnameserver 127.0.0.53\nsearch example.test\noptions edns0 trust-ad\nnameserver fd00::1\nnameserver bogus host\n",
        );
        assert_eq!(
            generated,
            "# generated by cowboy-browser-launcher\nnameserver 127.0.0.53\nnameserver fd00::1\noptions use-vc\n"
        );
    }

    #[test]
    fn timezone_prefers_the_host_tz_then_the_zone_name_then_the_file() {
        assert_eq!(
            timezone(Some("Europe/Paris"), None, None, false),
            "Europe/Paris"
        );
        assert_eq!(
            timezone(
                None,
                Some(Path::new("/usr/share/zoneinfo/Asia/Shanghai")),
                None,
                true
            ),
            "Asia/Shanghai"
        );
        assert_eq!(timezone(None, None, Some("Etc/UTC\n"), true), "Etc/UTC");
        assert_eq!(timezone(None, None, None, true), ":/etc/localtime");
        assert_eq!(timezone(None, None, None, false), "UTC");
        assert_eq!(timezone(Some("bad value"), None, None, false), "UTC");
        assert_eq!(
            timezone(
                None,
                Some(Path::new("/nix/store/abc-tzdata/share/zoneinfo/../etc")),
                None,
                true
            ),
            ":/etc/localtime"
        );
    }
}
