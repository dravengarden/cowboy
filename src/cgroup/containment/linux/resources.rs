//! Read through original directory descriptors; never infer zero from missing
//! controllers, vanished processes, truncation, or a changing membership set.
use super::*;
use crate::cgroup::containment::ResourceSample;
use std::collections::BTreeSet;

const MAX_PROCESSES: usize = 4096;

fn text(directory: &File, name: &str, limit: u64) -> io::Result<String> {
    let mut result = String::new();
    member(directory, name, OFlags::RDONLY)?
        .take(limit + 1)
        .read_to_string(&mut result)?;
    if result.len() as u64 > limit {
        return Err(io::Error::other("resource observation exceeds bound"));
    }
    Ok(result)
}

fn members(
    directory: &File,
    depth: usize,
    remaining: &mut usize,
    pids: &mut BTreeSet<u32>,
) -> io::Result<()> {
    if depth > 16 || *remaining == 0 {
        return Err(io::Error::other("resource subtree exceeds bound"));
    }
    *remaining -= 1;
    for line in text(directory, "cgroup.procs", 64 * 1024)?.lines() {
        let pid = line.parse::<u32>().map_err(io::Error::other)?;
        if pid == 0 || !pids.insert(pid) || pids.len() > MAX_PROCESSES {
            return Err(io::Error::other("resource membership invalid or changing"));
        }
    }
    for entry in Dir::read_from(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        if matches!(name.to_bytes(), b"." | b"..") || entry.file_type() != FileType::Directory {
            continue;
        }
        let child: File = openat(directory, name, DIRECTORY, Mode::empty())?.into();
        members(&child, depth + 1, remaining, pids)?;
    }
    Ok(())
}

fn membership(directory: &File) -> io::Result<BTreeSet<u32>> {
    let mut pids = BTreeSet::new();
    members(directory, 0, &mut 256, &mut pids)?;
    Ok(pids)
}

fn process_stat(text: &str) -> io::Result<(u64, u64)> {
    // comm can itself contain whitespace and ')'. Fields 22 and 24 are the
    // boot-relative start identity and RSS in pages, respectively.
    let tail = text
        .rsplit_once(')')
        .ok_or_else(|| io::Error::other("invalid process stat"))?
        .1;
    let fields: Vec<_> = tail.split_whitespace().collect();
    let number = |index: usize| -> io::Result<u64> {
        fields
            .get(index)
            .ok_or_else(|| io::Error::other("incomplete process stat"))?
            .parse()
            .map_err(io::Error::other)
    };
    Ok((number(19)?, number(21)?))
}

fn cpu(text: &str) -> io::Result<u64> {
    let mut usage = text
        .lines()
        .filter_map(|line| line.strip_prefix("usage_usec "));
    let value = usage
        .next()
        .ok_or_else(|| io::Error::other("CPU usage unavailable"))?
        .parse()
        .map_err(io::Error::other)?;
    if usage.next().is_some() {
        return Err(io::Error::other("duplicate CPU observation"));
    }
    Ok(value)
}

#[allow(unsafe_code)]
fn page_size() -> io::Result<u64> {
    // SAFETY: sysconf reads a process-independent constant, takes no pointers,
    // and has no preconditions. Do not assume Linux uses 4 KiB pages.
    let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    u64::try_from(size)
        .ok()
        .filter(|size| *size > 0)
        .ok_or_else(|| io::Error::other("native page size unavailable"))
}

pub(in super::super) fn sample(owner: &Containment) -> io::Result<ResourceSample> {
    if owner.identity.owner.pid_namespace != Some(process_namespace()?) {
        return Err(io::Error::other("resource PID namespace changed"));
    }
    let pids = membership(&owner.directory)?;
    let mut rss_bytes = 0_u64;
    let page_size = page_size()?;
    for pid in &pids {
        let directory: File = open(format!("/proc/{pid}"), DIRECTORY, Mode::empty())?.into();
        let first = process_stat(&text(&directory, "stat", 8192)?)?;
        let second = process_stat(&text(&directory, "stat", 8192)?)?;
        if first.0 == 0 || first.0 != second.0 {
            return Err(io::Error::other("resource process identity changed"));
        }
        rss_bytes = second
            .1
            .checked_mul(page_size)
            .and_then(|bytes| rss_bytes.checked_add(bytes))
            .ok_or_else(|| io::Error::other("resource RSS overflow"))?;
    }
    let cpu_usage_usec = cpu(&text(&owner.directory, "cpu.stat", 8192)?)?;
    if pids != membership(&owner.directory)? {
        return Err(io::Error::other(
            "resource membership changed during observation",
        ));
    }
    Ok(ResourceSample {
        processes: pids.len() as u32,
        rss_bytes,
        cpu_usage_usec,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stat_uses_identity_and_rss_after_the_complete_comm() {
        let fields = (3..=24)
            .map(|n| if n == 3 { "S".into() } else { n.to_string() })
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(
            process_stat(&format!("11 (name ) with spaces) {fields}")).unwrap(),
            (22, 24)
        );
        assert!(process_stat("11 (name) S 0").is_err());
        assert!(
            process_stat(&format!(
                "11 (name) {} -1",
                fields.rsplit_once(' ').unwrap().0
            ))
            .is_err()
        );
    }

    #[test]
    fn missing_malformed_and_duplicate_cpu_values_are_unknown() {
        assert_eq!(cpu("usage_usec 42\nuser_usec 30\n").unwrap(), 42);
        for value in [
            "",
            "usage_usec -1",
            "usage_usec 1\nusage_usec 2",
            "usage_usec invalid",
        ] {
            assert!(cpu(value).is_err());
        }
    }
}
