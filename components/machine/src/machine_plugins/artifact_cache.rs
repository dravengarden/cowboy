//! Untrusted, content-addressed download input. Cache import is not installation
//! authority: the installer validates the signed release before consulting it.

use super::*;
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::os::fd::{AsFd as _, BorrowedFd};

fn parse_sha256_hex(digest: &str) -> Result<String> {
    let hash = digest
        .strip_prefix("sha256:")
        .context("expected SHA-256 digest")?;
    ensure!(
        hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "expected SHA-256 digest"
    );
    Ok(hash.to_ascii_lowercase())
}

fn cache_path(state: &Path, digest: &str) -> Result<PathBuf> {
    Ok(state.join("artifact-cache").join(parse_sha256_hex(digest)?))
}

/// Advisory cross-process lock on `<state>/artifact-cache/.lock` coordinating
/// standalone preload (`--cache-runtime-artifact`), installation caching,
/// corrupt-entry repair, and cache GC / temp-file sweeping.
struct CacheLock(fs::File);

impl CacheLock {
    fn acquire(directory: &Path) -> Result<Self> {
        let lock_path = directory.join(".lock");
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&lock_path)
            .with_context(|| format!("opening {}", lock_path.display()))?;
        ensure!(
            file.metadata()?.is_file(),
            "artifact cache lock must be a regular file"
        );
        rustix::fs::flock(&file, rustix::fs::FlockOperation::LockExclusive)
            .with_context(|| format!("locking {}", lock_path.display()))?;
        Ok(Self(file))
    }
}

impl Drop for CacheLock {
    fn drop(&mut self) {
        let _ = rustix::fs::flock(&self.0, rustix::fs::FlockOperation::Unlock);
    }
}

/// Native `(st_dev, st_ino)` pair obtained via `rustix::fs::fstat`/`statat` so
/// Linux and macOS compare identical native types without width mismatches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileId {
    dev: libc::dev_t,
    ino: libc::ino_t,
}

#[allow(clippy::useless_conversion)]
fn fstat_id(file: &fs::File) -> Result<(FileId, u32, u32)> {
    let stat = rustix::fs::fstat(file)?;
    let is_reg = (stat.st_mode & libc::S_IFMT) == libc::S_IFREG;
    ensure!(is_reg, "artifact cache input must be a regular file");
    Ok((
        FileId {
            dev: stat.st_dev,
            ino: stat.st_ino,
        },
        stat.st_uid,
        u32::from(stat.st_mode),
    ))
}

/// Stream-verify an already-open file against `digest` using a bounded buffer
/// (never allocating an archive-sized buffer) and return `(FileId, uid, mode)`.
fn verify_open_file(file: &mut fs::File, digest: &str) -> Result<(FileId, u32, u32)> {
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= MAX_PROVIDER_RUNTIME_ARTIFACT_BYTES as u64,
        "artifact cache input must be a bounded regular file"
    );
    let id_uid_mode = fstat_id(file)?;
    file.seek(SeekFrom::Start(0))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut total: usize = 0;
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        total = total
            .checked_add(n)
            .context("artifact cache input size overflow")?;
        ensure!(
            total <= MAX_PROVIDER_RUNTIME_ARTIFACT_BYTES,
            "artifact cache input exceeds limit"
        );
        hasher.update(&buffer[..n]);
    }
    ensure!(
        format!("sha256:{:x}", hasher.finalize()).eq_ignore_ascii_case(digest),
        "artifact cache digest mismatch"
    );
    Ok(id_uid_mode)
}

fn read_verified(path: &Path, digest: &str) -> Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() <= MAX_PROVIDER_RUNTIME_ARTIFACT_BYTES as u64,
        "artifact cache input must be a bounded regular file"
    );
    let mut bytes = Vec::new();
    file.take(MAX_PROVIDER_RUNTIME_ARTIFACT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= MAX_PROVIDER_RUNTIME_ARTIFACT_BYTES,
        "artifact cache input exceeds limit"
    );
    ensure!(
        format!("sha256:{:x}", Sha256::digest(&bytes)).eq_ignore_ascii_case(digest),
        "artifact cache digest mismatch"
    );
    Ok(bytes)
}

fn validate_directory(directory: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt as _;
    let metadata = directory.symlink_metadata()?;
    ensure!(
        metadata.is_dir()
            && metadata.uid() == rustix::process::geteuid().as_raw()
            && metadata.permissions().mode().trailing_zeros() >= 6,
        "artifact cache directory must be private and owned by the Machine user"
    );
    Ok(())
}

pub(super) fn read(state: &Path, digest: &str) -> Result<Option<Vec<u8>>> {
    let path = cache_path(state, digest)?;
    let directory = path.parent().context("artifact cache directory")?;
    if !directory.try_exists()? {
        return Ok(None);
    }
    validate_directory(directory)?;
    match path.symlink_metadata() {
        Ok(_) => read_verified(&path, digest).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Drop guard that cleans up `.runtime.*.partial` even if `stage_provider_runtime`
/// is cancelled or dropped by an `InstallGuard` timeout.
pub(super) struct StagedRuntimeDirectory(Option<PathBuf>);

impl StagedRuntimeDirectory {
    pub(super) fn new(path: PathBuf) -> Self {
        Self(Some(path))
    }

    pub(super) fn path(&self) -> &Path {
        self.0
            .as_deref()
            .expect("staging directory already committed")
    }

    pub(super) fn commit(&mut self) {
        self.0 = None;
    }
}

impl Drop for StagedRuntimeDirectory {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = remove_projection::remove(&path);
        }
    }
}

/// What one artifact cache retention pass removed.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Retired {
    pub files: usize,
    pub bytes: u64,
}

/// A verified read-only (`0o400`) cache entry pinned open by file descriptor so
/// its `(dev, ino)` cannot be reused before hardlinking completes, without
/// retaining a second in-memory copy of the archive bytes.
pub(super) struct CachedArtifact {
    cache_dir: fs::File,
    name: String,
    file: fs::File,
    id: FileId,
}

impl CachedArtifact {
    /// Stream-verify the pinned descriptor against `expected_digest` and return
    /// a cloned reader positioned at offset 0.
    pub(super) fn verified_reader(&self, expected_digest: &str) -> Result<fs::File> {
        let mut cloned = self.file.try_clone()?;
        let (id, uid, mode) = verify_open_file(&mut cloned, expected_digest)?;
        let expected_uid = rustix::process::geteuid().as_raw();
        ensure!(
            id == self.id && uid == expected_uid && (mode & 0o222) == 0,
            "cached runtime artifact changed or became writable before verification"
        );
        cloned.seek(SeekFrom::Start(0))?;
        Ok(cloned)
    }
}

fn open_verified_readonly_cached(
    directory: &Path,
    name: &str,
    digest: &str,
) -> Result<Option<CachedArtifact>> {
    let path = directory.join(name);
    let mut file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let (id, uid, mode) = verify_open_file(&mut file, digest)?;
    let expected_uid = rustix::process::geteuid().as_raw();
    ensure!(
        uid == expected_uid && (mode & 0o222) == 0,
        "artifact cache entry is writable or owned by another user"
    );
    let cache_dir = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
        .open(directory)?;
    Ok(Some(CachedArtifact {
        cache_dir,
        name: name.to_owned(),
        file,
        id,
    }))
}

/// Ensure `bytes` (already verified against `digest`) exists once under
/// `<state>/artifact-cache/<hex>` with mode `0o400` using a single locked
/// create-if-absent path shared by preload and installation.
pub(super) fn ensure_cached(state: &Path, digest: &str, bytes: &[u8]) -> Result<CachedArtifact> {
    let hash = parse_sha256_hex(digest)?;
    let directory = state.join("artifact-cache");
    match fs::DirBuilder::new().mode(0o700).create(&directory) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    validate_directory(&directory)?;
    let _lock = CacheLock::acquire(&directory)?;

    if let Ok(Some(existing)) = open_verified_readonly_cached(&directory, &hash, digest) {
        return Ok(existing);
    }

    let cached = directory.join(&hash);
    let temp = directory.join(format!(
        ".cache.{}.{}",
        std::process::id(),
        ATOMIC_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut created = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o400)
        .open(&temp)?;
    let write_result = (|| -> Result<()> {
        created.write_all(bytes)?;
        created.sync_all()?;
        Ok(())
    })();
    if let Err(error) = write_result {
        drop(created);
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    drop(created);

    let pinned = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&temp)?;
    let (id, _, _) = fstat_id(&pinned)?;

    if let Err(error) = fs::rename(&temp, &cached) {
        let _ = fs::remove_file(&temp);
        return Err(error.into());
    }
    let cache_dir = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
        .open(&directory)?;
    cache_dir.sync_all()?;
    Ok(CachedArtifact {
        cache_dir,
        name: hash,
        file: pinned,
        id,
    })
}

/// Stream a private standalone copy of a verified [`CachedArtifact`] into
/// `dest_dir/dest_name` using directory-FD-relative syscalls only (used on
/// `EXDEV` or when a host probe's descendant exit could not be confirmed).
pub(super) fn copy_verified_at(
    cached: &CachedArtifact,
    expected_digest: &str,
    dest_dir: BorrowedFd<'_>,
    dest_name: &str,
) -> Result<()> {
    let mut source = cached.verified_reader(expected_digest)?;
    let copy_name = format!(
        ".artifact.copy.{}.{}",
        std::process::id(),
        ATOMIC_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let fd = rustix::fs::openat(
        dest_dir,
        copy_name.as_str(),
        rustix::fs::OFlags::WRONLY
            | rustix::fs::OFlags::CREATE
            | rustix::fs::OFlags::EXCL
            | rustix::fs::OFlags::NOFOLLOW,
        rustix::fs::Mode::from_raw_mode(0o400),
    )?;
    let mut file = fs::File::from(fd);
    let result = (|| -> Result<()> {
        std::io::copy(&mut source, &mut file)?;
        file.sync_all()?;
        drop(file);
        rustix::fs::renameat(dest_dir, copy_name.as_str(), dest_dir, dest_name)?;
        rustix::fs::fsync(dest_dir)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = rustix::fs::unlinkat(dest_dir, copy_name.as_str(), rustix::fs::AtFlags::empty());
    }
    result
}

/// Atomically hardlink a previously verified [`CachedArtifact`] into `dest_dir`
/// under `dest_name` using directory-FD-relative syscalls only (never following
/// symlinks), checking the temporary link's file type, `(st_dev, st_ino)`,
/// ownership, and read-only mode before renaming into place.
#[allow(clippy::useless_conversion)]
pub(super) fn link_verified_at(
    cached: &CachedArtifact,
    expected_digest: &str,
    dest_dir: BorrowedFd<'_>,
    dest_name: &str,
) -> Result<()> {
    let temp_name = format!(
        ".artifact.link.{}.{}",
        std::process::id(),
        ATOMIC_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    match rustix::fs::linkat(
        cached.cache_dir.as_fd(),
        cached.name.as_str(),
        dest_dir,
        temp_name.as_str(),
        rustix::fs::AtFlags::empty(),
    ) {
        Ok(()) => {
            let result = (|| -> Result<()> {
                let stat = rustix::fs::statat(
                    dest_dir,
                    temp_name.as_str(),
                    rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
                )?;
                let is_reg = (stat.st_mode & libc::S_IFMT) == libc::S_IFREG;
                let linked_id = FileId {
                    dev: stat.st_dev,
                    ino: stat.st_ino,
                };
                ensure!(
                    is_reg
                        && linked_id == cached.id
                        && stat.st_uid == rustix::process::geteuid().as_raw()
                        && (u32::from(stat.st_mode) & 0o222) == 0,
                    "cached runtime artifact changed during hardlink"
                );
                rustix::fs::renameat(dest_dir, temp_name.as_str(), dest_dir, dest_name)?;
                rustix::fs::fsync(dest_dir)?;
                Ok(())
            })();
            if result.is_err() {
                let _ = rustix::fs::unlinkat(
                    dest_dir,
                    temp_name.as_str(),
                    rustix::fs::AtFlags::empty(),
                );
            }
            result
        }
        Err(rustix::io::Errno::XDEV) => {
            copy_verified_at(cached, expected_digest, dest_dir, dest_name)
        }
        Err(error) => Err(error.into()),
    }
}

/// Atomically write standalone archive bytes into `dest_dir/dest_name` using
/// directory-FD-relative syscalls only (used only when `cache_state` is `None`).
pub(super) fn write_bytes_at(
    dest_dir: BorrowedFd<'_>,
    dest_name: &str,
    bytes: &[u8],
) -> Result<()> {
    let temp_name = format!(
        ".artifact.write.{}.{}",
        std::process::id(),
        ATOMIC_WRITE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    );
    let fd = rustix::fs::openat(
        dest_dir,
        temp_name.as_str(),
        rustix::fs::OFlags::WRONLY
            | rustix::fs::OFlags::CREATE
            | rustix::fs::OFlags::EXCL
            | rustix::fs::OFlags::NOFOLLOW,
        rustix::fs::Mode::from_raw_mode(0o400),
    )?;
    let mut file = fs::File::from(fd);
    let result = (|| -> Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        rustix::fs::renameat(dest_dir, temp_name.as_str(), dest_dir, dest_name)?;
        rustix::fs::fsync(dest_dir)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = rustix::fs::unlinkat(dest_dir, temp_name.as_str(), rustix::fs::AtFlags::empty());
    }
    result
}

/// Remove abandoned `.runtime.*` partial staging directories under every
/// Plugin's generations. Must be called only while the caller holds the
/// installation owner lock (`MachinePluginStore::lifecycle`).
pub(super) fn sweep_abandoned_runtime_staging(plugins: &Path) -> Result<usize> {
    if !plugins.try_exists()? {
        return Ok(0);
    }
    let mut removed = 0;
    for plugin in fs::read_dir(plugins)? {
        let plugin = plugin?;
        if !plugin.file_type()?.is_dir() {
            continue;
        }
        let generations = plugin.path().join("generations");
        if !generations.try_exists()? {
            continue;
        }
        for generation in fs::read_dir(&generations)? {
            let generation = generation?;
            if !generation.file_type()?.is_dir() {
                continue;
            }
            let content = generation.path().join("content");
            if !content.try_exists()? {
                continue;
            }
            let mut swept_in_content = false;
            for child in fs::read_dir(&content)? {
                let child = child?;
                if child.file_name().to_string_lossy().starts_with(".runtime.") {
                    remove_projection::remove(&child.path()).with_context(|| {
                        format!(
                            "removing abandoned runtime staging {}",
                            child.path().display()
                        )
                    })?;
                    removed += 1;
                    swept_in_content = true;
                }
            }
            if swept_in_content {
                fs::File::open(&content)?.sync_all()?;
            }
        }
    }
    Ok(removed)
}

/// Remove abandoned `.cache.*` temporary files under the cache lock first
/// (so damaged generation metadata cannot leave up to 1 GiB temp files stuck on
/// disk), then retire aged unreferenced canonical blobs only if the generation
/// reference scan succeeds cleanly.
pub(super) fn retire_unreferenced(
    state: &Path,
    plugins: &Path,
    unreferenced_after: std::time::Duration,
) -> Result<Retired> {
    use std::os::unix::fs::MetadataExt as _;
    let directory = state.join("artifact-cache");
    if !directory.try_exists()? {
        return Ok(Retired::default());
    }
    validate_directory(&directory)?;
    let _lock = CacheLock::acquire(&directory)?;

    // Sweep abandoned `.cache.*` temps first while holding `_lock`.
    let mut swept_temps = false;
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with(".cache.") {
            let metadata = entry.path().symlink_metadata()?;
            if metadata.is_file() {
                fs::remove_file(entry.path())?;
                swept_temps = true;
            }
        }
    }
    if swept_temps {
        fs::File::open(&directory)?.sync_all()?;
    }

    let referenced = referenced_runtime_digests(plugins)?;
    let cutoff = SystemTime::now()
        .checked_sub(unreferenced_after)
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let mut retired = Retired::default();
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if name.len() != 64
            || !name.bytes().all(|byte| byte.is_ascii_hexdigit())
            || referenced.contains(&name)
        {
            continue;
        }
        let metadata = entry.path().symlink_metadata()?;
        if !metadata.is_file() || metadata.nlink() > 1 || metadata.modified()? > cutoff {
            continue;
        }
        fs::remove_file(entry.path())?;
        retired.files += 1;
        retired.bytes = retired
            .bytes
            .saturating_add(metadata.blocks().saturating_mul(512));
    }
    if retired.files > 0 {
        fs::File::open(&directory)?.sync_all()?;
    }
    Ok(retired)
}

/// Verify a stored signed release (`plugin-release.json` or `release.json`)
/// against `plugin_id`, `gen_name`, its package bytes, and its publisher
/// signature before using its component matrix as reference authority.
fn verified_release_runtime_digests(
    plugin_id: &str,
    gen_name: &str,
    content: &Path,
) -> Result<Option<BTreeSet<String>>> {
    let expected_digest = format!("sha256:{gen_name}");
    let plugin_release_path = content.join("plugin-release.json");
    if plugin_release_path.try_exists()? {
        let release: cowboy_plugin_sdk::PluginRelease =
            serde_json::from_slice(&fs::read(&plugin_release_path)?)?;
        ensure!(
            release
                .artifact_digest
                .eq_ignore_ascii_case(&expected_digest),
            "stored Plugin release digest mismatch"
        );
        let package_bytes = fs::read(content.join("package.cowboy-plugin"))?;
        let package = release.validate_bytes(&package_bytes)?;
        ensure!(
            package.manifest.id == plugin_id,
            "stored Plugin id mismatch"
        );
        let publisher_key = fs::read_to_string(content.join("publisher.pub"))?;
        ensure!(
            crate::machine_auth::verify_namespaced(
                &publisher_key,
                PLUGIN_RELEASE_SIGNATURE_NAMESPACE,
                &release.proof(),
                &release.signature,
            )?,
            "stored Plugin release signature is invalid"
        );
        let mut digests = BTreeSet::new();
        for platform in &release.runtime_artifacts {
            for component in &platform.components {
                digests.insert(parse_sha256_hex(&component.artifact_digest)?);
            }
        }
        return Ok(Some(digests));
    }
    let legacy_release_path = content.join("release.json");
    if legacy_release_path.try_exists()? {
        let release: LegacyProviderRelease =
            serde_json::from_slice(&fs::read(&legacy_release_path)?)?;
        ensure!(
            release
                .artifact_digest
                .eq_ignore_ascii_case(&expected_digest),
            "stored legacy Provider release digest mismatch"
        );
        let package_bytes = fs::read(content.join("package.cowboy-provider"))?;
        let package: ProviderPackage = serde_json::from_slice(&package_bytes)?;
        ensure!(
            package.manifest.id == plugin_id
                && format!("sha256:{:x}", Sha256::digest(&package_bytes))
                    .eq_ignore_ascii_case(&expected_digest),
            "stored legacy Provider package mismatch"
        );
        let publisher_key = fs::read_to_string(content.join("publisher.pub"))?;
        ensure!(
            crate::machine_auth::verify_namespaced(
                &publisher_key,
                LEGACY_PROVIDER_RELEASE_SIGNATURE_NAMESPACE,
                &release.proof()?,
                &release.signature,
            )?,
            "stored legacy Provider release signature is invalid"
        );
        let mut digests = BTreeSet::new();
        for platform in &release.runtime_artifacts {
            for component in &platform.components {
                digests.insert(parse_sha256_hex(&component.artifact_digest)?);
            }
        }
        return Ok(Some(digests));
    }
    Ok(None)
}

/// Lowercase hex digests of every runtime artifact any retained generation
/// of any Plugin records. Unions cryptographically verified release digests
/// with runtime metadata so incomplete or unverified metadata cannot authorize
/// cache deletion.
fn referenced_runtime_digests(plugins: &Path) -> Result<BTreeSet<String>> {
    let mut referenced = BTreeSet::new();
    if !plugins.try_exists()? {
        return Ok(referenced);
    }
    for plugin in fs::read_dir(plugins)? {
        let plugin = plugin?;
        if !plugin.file_type()?.is_dir() {
            continue;
        }
        let plugin_id = plugin.file_name().to_string_lossy().into_owned();
        let generations = plugin.path().join("generations");
        if !generations.try_exists()? {
            continue;
        }
        for generation in fs::read_dir(&generations)? {
            let generation = generation?;
            let gen_name_os = generation.file_name();
            let gen_name = gen_name_os.to_string_lossy();
            if gen_name.starts_with(RETIRED_GENERATION_PREFIX) || !generation.file_type()?.is_dir()
            {
                continue;
            }
            let recognized = is_generation_name(&gen_name);
            let content = generation.path().join("content");
            if !content.try_exists()? {
                ensure!(
                    !recognized,
                    "retained generation {} is missing its content directory",
                    generation.path().display()
                );
                continue;
            }
            for child in fs::read_dir(&content)? {
                let child = child?;
                if child.file_name().to_string_lossy().starts_with(".runtime.") {
                    bail!(
                        "in-flight runtime staging {} blocks artifact cache retention",
                        child.path().display()
                    );
                }
            }
            let verified_release = if recognized {
                verified_release_runtime_digests(&plugin_id, &gen_name, &content)?
            } else {
                None
            };
            let has_verified_release = verified_release.is_some();
            if let Some(digests) = verified_release {
                referenced.extend(digests);
            }
            if content.join("runtime").try_exists()? {
                let metadata = read_installed_runtime(&content).with_context(|| {
                    format!(
                        "reading installed runtime metadata under {} for artifact cache retention",
                        content.display()
                    )
                })?;
                if recognized && !has_verified_release && metadata.commands.is_empty() {
                    bail!(
                        "retained generation {} has empty runtime metadata and no verified signed release",
                        content.display()
                    );
                }
                for command in metadata.commands.values() {
                    referenced.insert(parse_sha256_hex(&command.artifact_digest).with_context(
                        || {
                            format!(
                                "invalid artifact digest in runtime metadata under {}",
                                content.display()
                            )
                        },
                    )?);
                }
            } else if !has_verified_release
                && (recognized
                    || content.join("package.cowboy-plugin").try_exists()?
                    || content.join("package.cowboy-provider").try_exists()?)
            {
                bail!(
                    "retained generation {} lacks both runtime metadata and a verified signed release",
                    content.display()
                );
            }
        }
    }
    Ok(referenced)
}

const MAX_RETAINED_GENERATIONS: usize = 64;

pub(super) fn read_retained(content: &Path, digest: &str) -> Option<Vec<u8>> {
    let current = content.parent()?;
    let generations = current.parent()?;
    for entry in fs::read_dir(generations)
        .ok()?
        .flatten()
        .take(MAX_RETAINED_GENERATIONS)
    {
        let generation = entry.path();
        if generation == current || !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let candidate = generation.join("content");
        let Ok(metadata) = read_installed_runtime(&candidate) else {
            continue;
        };
        for command in metadata.commands.values() {
            if !command.artifact_digest.eq_ignore_ascii_case(digest) {
                continue;
            }
            let path = candidate.join(&command.artifact);
            if ensure_within(&candidate, &path).is_ok()
                && let Ok(bytes) = read_verified(&path, digest)
            {
                return Some(bytes);
            }
        }
    }
    None
}

impl MachinePluginStore {
    /// Import opaque public bytes only; never opens identity, credentials,
    /// operation journals, generation pointers, or a Controller connection.
    pub fn cache_runtime_artifact(state: &Path, source: &Path, digest: &str) -> Result<usize> {
        ensure!(
            state.is_absolute() && state.is_dir(),
            "existing absolute Service state directory required"
        );
        let bytes = read_verified(source, digest)?;
        ensure_cached(state, digest, &bytes)?;
        Ok(bytes.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt as _, symlink};

    fn link_at(cached: &CachedArtifact, digest: &str, destination: &Path) -> Result<()> {
        let dir = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
            .open(destination.parent().unwrap())?;
        link_verified_at(
            cached,
            digest,
            dir.as_fd(),
            destination.file_name().unwrap().to_str().unwrap(),
        )
    }

    #[tokio::test]
    async fn verified_cache_stages_without_contacting_the_artifact_origin() {
        use cowboy_provider_sdk::{
            PrivateComponentKind, ProviderArtifactFormat, ProviderArtifactProbe,
        };
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("download");
        let bytes = b"#!/bin/sh\nexit 0\n";
        fs::write(&source, bytes).unwrap();
        let digest = format!("sha256:{:x}", Sha256::digest(bytes));
        MachinePluginStore::cache_runtime_artifact(root.path(), &source, &digest).unwrap();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let artifact = ReleasedPrivateComponent {
            kind: PrivateComponentKind::ProviderCli,
            slot: "fixture".into(),
            dependency: "fixture".into(),
            version: "1.0.0".into(),
            command: "fixture".into(),
            artifact_url: format!("http://{}/artifact", listener.local_addr().unwrap()),
            artifact_digest: digest,
            artifact_format: ProviderArtifactFormat::Raw,
            entrypoint: None,
            probe: ProviderArtifactProbe {
                args: vec![],
                timeout_ms: 1_000,
            },
        };
        let _ = rustls::crypto::ring::default_provider().install_default();
        let staged = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            runtime_staging::stage_runtime_component(
                &reqwest::Client::new(),
                &root.path().join("staging"),
                &artifact,
                &InstallGuard::Legacy,
                Some(root.path()),
                None,
                false,
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(fs::read(staged.executable).unwrap(), bytes);
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn retention_removes_only_old_unreferenced_content_addressed_blobs() {
        use std::time::{Duration, SystemTime};
        let state = tempfile::tempdir().unwrap();
        let cache = state.path().join("artifact-cache");
        fs::DirBuilder::new().mode(0o700).create(&cache).unwrap();
        let blob = |bytes: &[u8], age: Duration| {
            let hash = format!("{:x}", Sha256::digest(bytes));
            let path = cache.join(&hash);
            fs::write(&path, bytes).unwrap();
            fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(SystemTime::now() - age)
                .unwrap();
            (hash, path)
        };
        let week = Duration::from_hours(7 * 24);
        let (referenced_hash, referenced) = blob(b"still installed", 2 * week);
        let (_, superseded) = blob(b"superseded runtime", 2 * week);
        let superseded_blocks = superseded.symlink_metadata().unwrap().blocks() * 512;
        let (_, recent) = blob(b"freshly preloaded", Duration::from_mins(1));
        let temporary = cache.join(".partial.tmp");
        fs::write(&temporary, b"in-flight write").unwrap();
        let abandoned_cache_tmp = cache.join(".cache.99999.1");
        fs::write(&abandoned_cache_tmp, b"abandoned import").unwrap();

        let plugins = state.path().join("plugins");
        let content = plugins.join("claude-code/generations/g1/content");
        fs::create_dir_all(content.join("runtime")).unwrap();
        fs::write(
            content.join("runtime/metadata.json"),
            serde_json::json!({"schema_version": 2, "commands": {"claude": {
                "executable": "runtime/x/claude",
                "artifact": "runtime/x/artifact.tar.gz",
                "artifact_digest": format!("sha256:{referenced_hash}"),
            }}})
            .to_string(),
        )
        .unwrap();

        let retired = retire_unreferenced(state.path(), &plugins, week).unwrap();
        assert_eq!(retired.files, 1);
        assert_eq!(retired.bytes, superseded_blocks);
        assert!(!superseded.exists());
        assert!(!abandoned_cache_tmp.exists());
        assert!(
            referenced.exists(),
            "a retained generation still references it"
        );
        assert!(recent.exists(), "younger than the threshold");
        assert!(temporary.exists(), "not a content-addressed blob");
        assert_eq!(
            retire_unreferenced(state.path(), &plugins, week).unwrap(),
            Retired::default()
        );
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(
            retire_unreferenced(empty.path(), &plugins, week).unwrap(),
            Retired::default()
        );
    }

    #[test]
    fn abandoned_cache_temps_are_swept_even_when_reference_scan_fails() {
        use std::time::Duration;
        let state = tempfile::tempdir().unwrap();
        let cache = state.path().join("artifact-cache");
        fs::DirBuilder::new().mode(0o700).create(&cache).unwrap();
        let abandoned = cache.join(".cache.42.9");
        fs::write(&abandoned, b"interrupted 1gb write").unwrap();
        let plugins = state.path().join("plugins");
        // Damaged generation missing runtime/ makes reference scan fail closed.
        fs::create_dir_all(
            plugins
                .join("claude-code/generations")
                .join("a".repeat(64))
                .join("content"),
        )
        .unwrap();
        assert!(retire_unreferenced(state.path(), &plugins, Duration::from_hours(1)).is_err());
        assert!(
            !abandoned.exists(),
            "abandoned temp must be swept before failing scan"
        );
    }

    #[test]
    fn retained_generation_supplies_only_exact_unchanged_runtime_bytes() {
        let root = tempfile::tempdir().unwrap();
        let generations = root.path().join("generations");
        let bytes = b"unchanged provider cli";
        let digest = format!("sha256:{:x}", Sha256::digest(bytes));
        let retained = generations.join("old/content");
        fs::create_dir_all(retained.join("runtime/provider_cli-claude")).unwrap();
        fs::write(
            retained.join("runtime/provider_cli-claude/artifact.tar.gz"),
            bytes,
        )
        .unwrap();
        let metadata = |artifact: &str| {
            serde_json::json!({"schema_version": 2, "commands": {"claude": {
                "executable": "runtime/provider_cli-claude/content/claude",
                "artifact": artifact,
                "artifact_digest": digest,
            }}})
            .to_string()
        };
        fs::write(
            retained.join("runtime/metadata.json"),
            metadata("runtime/provider_cli-claude/artifact.tar.gz"),
        )
        .unwrap();
        let current = generations.join("new/content");
        fs::create_dir_all(&current).unwrap();

        assert_eq!(read_retained(&current, &digest).unwrap(), bytes);
        let other = format!("sha256:{:x}", Sha256::digest(b"changed adapter"));
        assert!(read_retained(&current, &other).is_none());
        assert!(read_retained(&retained, &digest).is_none());

        fs::write(retained.join("runtime/metadata.json"), metadata("../../x")).unwrap();
        assert!(read_retained(&current, &digest).is_none());
        fs::write(
            retained.join("runtime/metadata.json"),
            metadata("runtime/provider_cli-claude/artifact.tar.gz"),
        )
        .unwrap();
        fs::write(
            retained.join("runtime/provider_cli-claude/artifact.tar.gz"),
            b"tampered",
        )
        .unwrap();
        assert!(read_retained(&current, &digest).is_none());
    }

    #[test]
    fn opaque_import_is_not_installation_and_cache_tampering_fails_closed() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("download");
        fs::write(&source, b"public signed artifact bytes").unwrap();
        let digest = format!(
            "sha256:{:x}",
            Sha256::digest(b"public signed artifact bytes")
        );
        assert!(read(root.path(), &digest).unwrap().is_none());
        let uppercase = format!("sha256:{}", digest[7..].to_ascii_uppercase());
        assert!(read(root.path(), &uppercase).unwrap().is_none());
        MachinePluginStore::cache_runtime_artifact(root.path(), &source, &digest).unwrap();
        assert_eq!(
            read(root.path(), &uppercase).unwrap(),
            read(root.path(), &digest).unwrap()
        );
        assert_eq!(
            read(root.path(), &digest).unwrap().unwrap(),
            b"public signed artifact bytes"
        );
        assert!(!root.path().join("plugins").exists());
        assert!(!root.path().join("plugin-operations").exists());
        let cached = cache_path(root.path(), &digest).unwrap();
        fs::set_permissions(&cached, fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(&cached, b"tampered").unwrap();
        assert!(read(root.path(), &digest).is_err());
        fs::remove_file(&cached).unwrap();
        symlink(&source, &cached).unwrap();
        assert!(read(root.path(), &digest).is_err());
    }

    #[test]
    fn unsafe_paths_and_wrong_digests_never_create_cache_state() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("download");
        fs::write(&source, b"public").unwrap();
        assert!(
            MachinePluginStore::cache_runtime_artifact(root.path(), &source, "sha256:../escape")
                .is_err()
        );
        assert!(
            MachinePluginStore::cache_runtime_artifact(
                root.path(),
                &source,
                &format!("sha256:{}", "0".repeat(64))
            )
            .is_err()
        );
        assert!(!root.path().join("artifact-cache").exists());
        let external = tempfile::tempdir().unwrap();
        symlink(external.path(), root.path().join("artifact-cache")).unwrap();
        let digest = format!("sha256:{:x}", Sha256::digest(b"public"));
        assert!(MachinePluginStore::cache_runtime_artifact(root.path(), &source, &digest).is_err());
        assert_eq!(fs::read_dir(external.path()).unwrap().count(), 0);
    }

    #[test]
    fn ensure_cached_migrates_legacy_writable_entries_to_fresh_readonly_inodes() {
        let state = tempfile::tempdir().unwrap();
        let bytes = b"legacy writable cached archive";
        let digest = format!("sha256:{:x}", Sha256::digest(bytes));
        let hash = digest.strip_prefix("sha256:").unwrap();
        let cache_dir = state.path().join("artifact-cache");
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&cache_dir)
            .unwrap();
        let legacy = cache_dir.join(hash);
        fs::write(&legacy, bytes).unwrap();
        fs::set_permissions(&legacy, fs::Permissions::from_mode(0o600)).unwrap();
        let legacy_ino = legacy.symlink_metadata().unwrap().ino();

        let alias = state.path().join("legacy-alias");
        fs::hard_link(&legacy, &alias).unwrap();

        let migrated = ensure_cached(state.path(), &digest, bytes).unwrap();
        let fresh_meta = legacy.symlink_metadata().unwrap();
        assert_ne!(fresh_meta.ino(), legacy_ino);
        assert_eq!(fresh_meta.permissions().mode() & 0o777, 0o400);
        assert_eq!(migrated.id.ino, fresh_meta.ino());
        assert_eq!(alias.symlink_metadata().unwrap().ino(), legacy_ino);
    }

    #[test]
    fn ensure_and_link_preserves_inodes_and_fails_closed_on_missing_or_malformed_metadata() {
        use std::time::{Duration, SystemTime};
        let state = tempfile::tempdir().unwrap();
        let bytes = b"shared signed tar.gz payload";
        let digest = format!("sha256:{:x}", Sha256::digest(bytes));
        let hash = digest.strip_prefix("sha256:").unwrap();

        let plugins = state.path().join("plugins");
        let generations_root = plugins.join("claude-code/generations");
        let g1_name = "a".repeat(64);
        let g2_name = "b".repeat(64);
        let gen1 = generations_root
            .join(&g1_name)
            .join("content/runtime/provider_cli-claude");
        let gen2 = generations_root
            .join(&g2_name)
            .join("content/runtime/provider_cli-claude");
        fs::create_dir_all(&gen1).unwrap();
        fs::create_dir_all(&gen2).unwrap();
        let art1 = gen1.join("artifact.tar.gz");
        let art2 = gen2.join("artifact.tar.gz");

        let cached1 = ensure_cached(state.path(), &digest, bytes).unwrap();
        link_at(&cached1, &digest, &art1).unwrap();
        let preload_src = state.path().join("preload.tar.gz");
        fs::write(&preload_src, bytes).unwrap();
        MachinePluginStore::cache_runtime_artifact(state.path(), &preload_src, &digest).unwrap();
        let cached2 = ensure_cached(state.path(), &digest, bytes).unwrap();
        link_at(&cached2, &digest, &art2).unwrap();

        let cached = state.path().join("artifact-cache").join(hash);
        let m_cache = cached.symlink_metadata().unwrap();
        assert_eq!(
            (m_cache.dev(), m_cache.ino()),
            (
                art1.symlink_metadata().unwrap().dev(),
                art1.symlink_metadata().unwrap().ino()
            )
        );
        assert_eq!(
            (m_cache.dev(), m_cache.ino()),
            (
                art2.symlink_metadata().unwrap().dev(),
                art2.symlink_metadata().unwrap().ino()
            )
        );
        assert_eq!(m_cache.nlink(), 3);
        assert_eq!(m_cache.permissions().mode() & 0o777, 0o400);

        let week = Duration::from_hours(7 * 24);
        fs::set_permissions(&cached, fs::Permissions::from_mode(0o600)).unwrap();
        fs::File::options()
            .write(true)
            .open(&cached)
            .unwrap()
            .set_modified(SystemTime::now() - 2 * week)
            .unwrap();
        fs::set_permissions(&cached, fs::Permissions::from_mode(0o400)).unwrap();
        assert!(retire_unreferenced(state.path(), &plugins, week).is_err());
        assert!(cached.exists());

        let meta_path1 = generations_root
            .join(&g1_name)
            .join("content/runtime/metadata.json");
        let meta_path2 = generations_root
            .join(&g2_name)
            .join("content/runtime/metadata.json");
        let empty_meta = serde_json::json!({"schema_version": 2, "commands": {}}).to_string();
        fs::write(&meta_path1, &empty_meta).unwrap();
        fs::write(&meta_path2, &empty_meta).unwrap();
        assert!(retire_unreferenced(state.path(), &plugins, week).is_err());

        let meta = serde_json::json!({"schema_version": 2, "commands": {"claude": {
            "executable": "runtime/provider_cli-claude/content/claude",
            "artifact": "runtime/provider_cli-claude/artifact.tar.gz",
            "artifact_digest": digest,
        }}})
        .to_string();
        fs::write(&meta_path1, &meta).unwrap();
        fs::write(&meta_path2, &meta).unwrap();

        let partial = generations_root
            .join(&g2_name)
            .join("content/.runtime.1.1.partial");
        fs::create_dir_all(&partial).unwrap();
        assert!(retire_unreferenced(state.path(), &plugins, week).is_err());
        assert_eq!(sweep_abandoned_runtime_staging(&plugins).unwrap(), 1);
        assert!(!partial.exists());

        fs::remove_dir_all(generations_root.join(&g1_name)).unwrap();
        assert_eq!(cached.symlink_metadata().unwrap().nlink(), 2);
        assert_eq!(
            retire_unreferenced(state.path(), &plugins, week).unwrap(),
            Retired::default()
        );

        fs::remove_dir_all(generations_root.join(&g2_name)).unwrap();
        assert_eq!(cached.symlink_metadata().unwrap().nlink(), 1);
        let expected_blocks = cached.symlink_metadata().unwrap().blocks() * 512;
        let retired = retire_unreferenced(state.path(), &plugins, week).unwrap();
        assert_eq!(retired.files, 1);
        assert_eq!(retired.bytes, expected_blocks);
        assert!(!cached.exists());
    }
}
