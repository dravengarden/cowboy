// SPDX-License-Identifier: GPL-3.0-or-later
//! Bounded read for the private sync primitive, always on the background pool.
use crate::Fs;
use anyhow::{Result, ensure};
use std::io::Read as _;
use std::path::Path;

pub(crate) async fn load<F: Fs + ?Sized>(fs: &F, path: &Path, limit: usize) -> Result<Vec<u8>> {
    let metadata = fs
        .metadata(path)
        .await?
        .ok_or_else(|| anyhow::anyhow!("sync source absent"))?;
    ensure!(
        !metadata.is_dir
            && !metadata.is_fifo
            && !metadata.is_symlink
            && metadata.len <= limit as u64,
        "sync source is not a bounded regular file"
    );
    read(fs.open_sync(path).await?, limit)
}

pub(crate) fn load_real(path: &Path, limit: usize) -> Result<Vec<u8>> {
    use rustix::fs::{CWD, Mode, OFlags, ResolveFlags, openat2};
    // Resolve every component without following symlinks, not just the final
    // filename. No stat/open race and no fallback on kernels without openat2.
    // O_NONBLOCK also prevents a substituted FIFO from pinning the reader.
    let descriptor = openat2(
        CWD,
        path,
        OFlags::RDONLY | OFlags::NONBLOCK | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
        ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
    )?;
    let file = std::fs::File::from(descriptor);
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= limit as u64,
        "sync source is not a bounded regular file"
    );
    read(file, limit)
}

fn read(reader: impl std::io::Read, limit: usize) -> Result<Vec<u8>> {
    ensure!(
        limit <= 4 * 1024 * 1024,
        "invalid private sync source budget"
    );
    let mut bytes = Vec::new();
    reader.take(limit as u64 + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= limit, "sync source exceeded its budget");
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cowboy_bounded_reader_rejects_growth_and_unbounded_streams() {
        assert_eq!(read(&b"abc"[..], 3).unwrap(), b"abc");
        assert!(read(&b"abcd"[..], 3).is_err());
        assert!(read(std::io::repeat(b'x'), 1024).is_err());
        assert!(read(std::io::empty(), 4 * 1024 * 1024 + 1).is_err());
    }

    #[test]
    fn cowboy_bounded_reader_consumes_only_one_overflow_byte() {
        struct Growing(usize);
        impl std::io::Read for Growing {
            fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
                bytes.fill(b'x');
                self.0 += bytes.len();
                Ok(bytes.len())
            }
        }
        let mut growing = Growing(0);
        assert!(read(&mut growing, 65_536).is_err());
        assert_eq!(growing.0, 65_537);
        assert_eq!(read(std::io::empty(), 0).unwrap(), b"");
        assert!(read(&b"x"[..], 0).is_err());
    }

    #[test]
    fn cowboy_opened_descriptor_rejects_symlink_directory_and_oversized_source() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let link = root.path().join("link");
        std::fs::write(&source, b"bounded").unwrap();
        assert_eq!(load_real(&source, 7).unwrap(), b"bounded");
        assert!(load_real(&source, 6).is_err());
        assert!(load_real(root.path(), 7).is_err());
        std::os::unix::fs::symlink(&source, &link).unwrap();
        assert!(load_real(&link, 7).is_err());
        std::os::unix::fs::symlink(root.path(), root.path().join("parent-link")).unwrap();
        assert!(load_real(&root.path().join("parent-link/source"), 7).is_err());
        let fifo = root.path().join("fifo");
        rustix::fs::mknodat(
            rustix::fs::CWD,
            &fifo,
            rustix::fs::FileType::Fifo,
            rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
            0,
        )
        .unwrap();
        assert!(load_real(&fifo, 7).is_err());
    }
}
