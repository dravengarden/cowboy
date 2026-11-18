//! A strict ustar reader for runtime archives. Only regular files and
//! directories are accepted; links, devices, FIFOs, PAX/GNU extension headers,
//! absolute paths and `..` components are refused, as are oversize entries
//! and archives with too many entries. Files are created beneath a directory
//! descriptor with `openat2` and become root-owned and read-only (`0444`, or
//! `0555` when the archive marks them executable); directories are `0755`.

use std::io::{self, Read, Write as _};
use std::os::fd::{AsFd as _, BorrowedFd, OwnedFd};

use anyhow::{Context as _, Result, bail, ensure};

use crate::safefs;

const BLOCK: usize = 512;
const MAX_PATH_BYTES: usize = 255;
const MAX_DEPTH: usize = 32;

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_entries: usize,
    pub max_total_bytes: u64,
}

/// Unpack the ustar stream `reader` into the empty directory `root`.
pub fn unpack(reader: &mut impl Read, root: BorrowedFd<'_>, limits: Limits) -> Result<usize> {
    let mut entries = 0;
    let mut total = 0_u64;
    let mut header = [0_u8; BLOCK];
    loop {
        if !read_block(reader, &mut header)? {
            bail!("archive ends without its end-of-archive blocks");
        }
        if header.iter().all(|&b| b == 0) {
            let mut second = [0_u8; BLOCK];
            ensure!(
                read_block(reader, &mut second)? && second.iter().all(|&b| b == 0),
                "truncated end-of-archive"
            );
            let mut rest = [0_u8; BLOCK];
            while read_block(reader, &mut rest)? {
                ensure!(rest.iter().all(|&b| b == 0), "data after end-of-archive");
            }
            return Ok(entries);
        }
        entries += 1;
        ensure!(
            entries <= limits.max_entries,
            "archive has more than {} entries",
            limits.max_entries
        );
        let entry = Header::parse(&header)?;
        total = total.saturating_add(entry.size);
        ensure!(
            total <= limits.max_total_bytes,
            "archive content exceeds {} bytes",
            limits.max_total_bytes
        );
        let components = normalize(&entry.path)?;
        match entry.kind {
            Kind::Directory => {
                ensure!(entry.size == 0, "directory {} has content", entry.path);
                if !components.is_empty() {
                    let parent = parent_dir(root, &components)?;
                    safefs::ensure_dir(
                        parent_fd(root, parent.as_ref()),
                        components.last().unwrap(),
                        0o755,
                        None,
                    )?;
                }
            }
            Kind::File => {
                ensure!(!components.is_empty(), "file entry without a name");
                let parent = parent_dir(root, &components)?;
                let name = components.last().unwrap();
                let mut file = safefs::create_new(parent_fd(root, parent.as_ref()), name, 0o600)
                    .with_context(|| format!("create {}", entry.path))?;
                let copied = io::copy(&mut reader.by_ref().take(entry.size), &mut file)?;
                ensure!(copied == entry.size, "truncated entry {}", entry.path);
                skip_padding(reader, entry.size)?;
                let mode = if entry.mode & 0o111 != 0 {
                    0o555
                } else {
                    0o444
                };
                file.flush()?;
                rustix::fs::fchmod(&file, rustix::fs::Mode::from_raw_mode(mode))?;
            }
        }
    }
}

fn parent_fd<'a>(root: BorrowedFd<'a>, parent: Option<&'a OwnedFd>) -> BorrowedFd<'a> {
    parent.map_or(root, |fd| fd.as_fd())
}

/// Open (creating as needed) every ancestor directory of `components`.
fn parent_dir(root: BorrowedFd<'_>, components: &[String]) -> Result<Option<OwnedFd>> {
    let mut current: Option<OwnedFd> = None;
    for name in &components[..components.len() - 1] {
        let next = safefs::ensure_dir(parent_fd(root, current.as_ref()), name, 0o755, None)?;
        current = Some(next);
    }
    Ok(current)
}

#[derive(Debug, PartialEq, Eq)]
enum Kind {
    File,
    Directory,
}

#[derive(Debug)]
struct Header {
    path: String,
    mode: u32,
    size: u64,
    kind: Kind,
}

impl Header {
    fn parse(block: &[u8; BLOCK]) -> Result<Self> {
        let stored = octal(&block[148..156]).context("checksum field")?;
        let computed: u64 = block
            .iter()
            .enumerate()
            .map(|(index, &byte)| {
                if (148..156).contains(&index) {
                    32
                } else {
                    u64::from(byte)
                }
            })
            .sum();
        ensure!(stored == computed, "header checksum mismatch");
        ensure!(&block[257..262] == b"ustar", "not a ustar header");
        let kind = match block[156] {
            b'0' | 0 => Kind::File,
            b'5' => Kind::Directory,
            b'1' | b'2' => bail!("links are not allowed"),
            b'3' | b'4' | b'6' => bail!("device and FIFO entries are not allowed"),
            other => bail!("entry type {:?} is not allowed", char::from(other)),
        };
        let name = field(&block[0..100])?;
        let prefix = field(&block[345..500])?;
        let path = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let mode = u32::try_from(octal(&block[100..108]).context("mode field")?)?;
        ensure!(block[124] & 0x80 == 0, "base-256 sizes are not allowed");
        let size = octal(&block[124..136]).context("size field")?;
        Ok(Self {
            path,
            mode,
            size,
            kind,
        })
    }
}

fn field(bytes: &[u8]) -> Result<String> {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let text = std::str::from_utf8(&bytes[..end]).context("non-UTF-8 path")?;
    Ok(text.to_owned())
}

fn octal(bytes: &[u8]) -> Result<u64> {
    let text = std::str::from_utf8(bytes)?.trim_matches(|c| c == ' ' || c == '\0');
    if text.is_empty() {
        return Ok(0);
    }
    ensure!(
        text.bytes().all(|b| (b'0'..=b'7').contains(&b)),
        "invalid octal field"
    );
    Ok(u64::from_str_radix(text, 8)?)
}

/// Split an archive path into safe components; `./` prefixes are dropped.
fn normalize(path: &str) -> Result<Vec<String>> {
    ensure!(path.len() <= MAX_PATH_BYTES, "path too long");
    ensure!(!path.starts_with('/'), "absolute path {path:?}");
    let trimmed = path.strip_suffix('/').unwrap_or(path);
    let mut components = Vec::new();
    for (index, part) in trimmed.split('/').enumerate() {
        if index == 0 && part == "." {
            continue;
        }
        ensure!(
            !part.is_empty() && part != "." && part != "..",
            "unsafe path {path:?}"
        );
        ensure!(
            part.bytes().all(|b| b >= 0x20 && b != 0x7f),
            "control character in {path:?}"
        );
        components.push(part.to_owned());
    }
    ensure!(components.len() <= MAX_DEPTH, "path too deep");
    Ok(components)
}

fn read_block(reader: &mut impl Read, block: &mut [u8; BLOCK]) -> Result<bool> {
    let mut filled = 0;
    while filled < BLOCK {
        match reader.read(&mut block[filled..]) {
            Ok(0) if filled == 0 => return Ok(false),
            Ok(0) => bail!("truncated block"),
            Ok(n) => filled += n,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(true)
}

fn skip_padding(reader: &mut impl Read, size: u64) -> Result<()> {
    let padding = (BLOCK as u64 - size % BLOCK as u64) % BLOCK as u64;
    let skipped = io::copy(&mut reader.by_ref().take(padding), &mut io::sink())?;
    ensure!(skipped == padding, "truncated padding");
    Ok(())
}

/// Test and fixture helper: build a ustar archive from `(path, type, data,
/// mode)` entries.
#[cfg(test)]
pub fn build(entries: &[(&str, u8, &[u8], u32)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (path, kind, data, mode) in entries {
        let mut header = [0_u8; BLOCK];
        header[..path.len()].copy_from_slice(path.as_bytes());
        header[100..107].copy_from_slice(format!("{mode:07o}").as_bytes());
        header[108..115].copy_from_slice(b"0000000");
        header[116..123].copy_from_slice(b"0000000");
        header[124..135].copy_from_slice(format!("{:011o}", data.len()).as_bytes());
        header[136..147].copy_from_slice(b"00000000000");
        header[156] = *kind;
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        header[148..156].copy_from_slice(b"        ");
        let sum: u32 = header.iter().map(|&b| u32::from(b)).sum();
        header[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
        out.extend_from_slice(&header);
        out.extend_from_slice(data);
        out.resize(out.len().div_ceil(BLOCK) * BLOCK, 0);
    }
    out.resize(out.len() + 2 * BLOCK, 0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    const LIMITS: Limits = Limits {
        max_entries: 100,
        max_total_bytes: 1 << 20,
    };

    fn unpack_bytes(bytes: &[u8]) -> (tempfile::TempDir, Result<usize>) {
        let temp = tempfile::tempdir().unwrap();
        let root = safefs::open_root(temp.path()).unwrap();
        let result = unpack(&mut &bytes[..], root.as_fd(), LIMITS);
        (temp, result)
    }

    #[test]
    fn regular_archives_unpack_read_only() {
        let archive = build(&[
            ("./", b'5', b"", 0o755),
            ("./bin/agy", b'0', b"#!/bin/sh\n", 0o755),
            ("cowboy-runtime.json", b'0', b"{}", 0o644),
        ]);
        let (temp, result) = unpack_bytes(&archive);
        assert_eq!(result.unwrap(), 3);
        let agy = std::fs::metadata(temp.path().join("bin/agy")).unwrap();
        assert_eq!(agy.permissions().mode() & 0o777, 0o555);
        let manifest = std::fs::metadata(temp.path().join("cowboy-runtime.json")).unwrap();
        assert_eq!(manifest.permissions().mode() & 0o777, 0o444);
    }

    #[test]
    fn unsafe_entries_are_refused() {
        for (path, kind) in [
            ("link", b'2'),
            ("hard", b'1'),
            ("dev", b'3'),
            ("blk", b'4'),
            ("fifo", b'6'),
            ("pax", b'x'),
            ("longname", b'L'),
            ("/etc/passwd", b'0'),
            ("../escape", b'0'),
            ("a/../../escape", b'0'),
            ("a//b", b'0'),
            ("a/./b", b'0'),
        ] {
            let (_temp, result) = unpack_bytes(&build(&[(path, kind, b"x", 0o644)]));
            assert!(
                result.is_err(),
                "{path} ({}) was accepted",
                char::from(kind)
            );
        }
    }

    #[test]
    fn bounds_duplicates_and_corruption_are_refused() {
        let many: Vec<(String, u8, &[u8], u32)> = (0..101)
            .map(|i| (format!("f{i}"), b'0', &b""[..], 0o644))
            .collect();
        let refs: Vec<(&str, u8, &[u8], u32)> = many
            .iter()
            .map(|(p, k, d, m)| (p.as_str(), *k, *d, *m))
            .collect();
        assert!(unpack_bytes(&build(&refs)).1.is_err());
        let big = vec![0_u8; (1 << 20) + 1];
        assert!(
            unpack_bytes(&build(&[("big", b'0', &big, 0o644)]))
                .1
                .is_err()
        );
        assert!(
            unpack_bytes(&build(&[
                ("a", b'0', b"1", 0o644),
                ("a", b'0', b"2", 0o644)
            ]))
            .1
            .is_err()
        );
        let mut corrupt = build(&[("a", b'0', b"1", 0o644)]);
        corrupt[0] = b'b';
        assert!(unpack_bytes(&corrupt).1.is_err());
        let full = build(&[("a", b'0', b"1", 0o644)]);
        assert!(unpack_bytes(&full[..full.len() - BLOCK]).1.is_err());
        let mut trailing = full.clone();
        trailing.extend_from_slice(&[1; BLOCK]);
        assert!(unpack_bytes(&trailing).1.is_err());
    }

    #[test]
    fn symlinked_parent_in_target_is_not_followed() {
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), temp.path().join("bin")).unwrap();
        let root = safefs::open_root(temp.path()).unwrap();
        let archive = build(&[("bin/agy", b'0', b"x", 0o755)]);
        assert!(unpack(&mut &archive[..], root.as_fd(), LIMITS).is_err());
        assert!(std::fs::read_dir(outside.path()).unwrap().next().is_none());
    }
}
