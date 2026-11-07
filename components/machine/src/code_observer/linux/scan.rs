use super::*;
use rustix::fs::{AtFlags, Dir, FileType, statat};
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt as _;

impl Source {
    pub(super) fn scan(&mut self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut directories = vec![PathBuf::new()];
        let mut entries = 0;
        while let Some(relative) = directories.pop() {
            ensure!(Instant::now() < deadline, "observer scan deadline exceeded");
            let directory = self.open(&relative, true)?;
            self.watch(&directory, relative.clone(), true)?;
            for entry in Dir::read_from(&directory)? {
                let entry = entry?;
                let name = entry.file_name();
                if matches!(name.to_bytes(), b"." | b"..") {
                    continue;
                }
                entries += 1;
                ensure!(
                    entries <= 100_000 && Instant::now() < deadline,
                    "observer scan budget exhausted"
                );
                let path = relative.join(OsStr::from_bytes(name.to_bytes()));
                ensure!(
                    path.as_os_str().len() <= 4096 && path.to_str().is_some(),
                    "observer path is not bounded UTF-8"
                );
                let metadata = statat(&directory, name, AtFlags::SYMLINK_NOFOLLOW)?;
                match FileType::from_raw_mode(metadata.st_mode) {
                    FileType::Directory => {
                        ensure!(
                            directories.len() < MAX_WATCHES,
                            "observer directory budget exhausted"
                        );
                        directories.push(path);
                    }
                    FileType::RegularFile => {
                        // Inode watches also observe writes through another hard
                        // link outside the root; no external path is followed.
                        let file = self.open(&path, false)?;
                        self.watch(&file, path, false)?;
                    }
                    // Link metadata is observed through its parent. Targets
                    // outside this non-projecting root are not in its scope.
                    FileType::Symlink => {}
                    _ => anyhow::bail!("observer scope contains a special file"),
                }
            }
        }
        Ok(())
    }
}
