//! Installation authority inside the existing Machine operation journal.
//! The enclosing journal owns the process lock; the store owns lifecycle
//! serialization. Reopen only reads evidence, never replays a pending effect.

use super::*;
use crate::machine_protocol::installation_revision::InstallationRevision;

const MAX_SLOTS: usize = 1024;
const MAX_SLOT_BYTES: u64 = 4096;
pub(super) const DIRECTORY: &str = "installations-v1";

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
enum Outcome {
    Stable {},
    Unknown {},
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::machine_plugins) enum Effect {
    Adopt,
    Install,
    Uninstall,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Transition {
    schema: u16,
    plugin_id: String,
    revision: InstallationRevision,
    previous_revision: Option<InstallationRevision>,
    generation_digest: Option<String>,
    effect: Effect,
    /// Binds a removal tombstone to the complete durable uninstall request.
    operation_digest: Option<String>,
    outcome: Outcome,
}

/// Process-local completion authority, deliberately not deserializable.
pub(in crate::machine_plugins) struct PendingInstallation {
    transition: Transition,
    owner: std::sync::Arc<()>,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SlotFile {
    transition: Transition,
    evidence_digest: String,
}

struct State {
    slots: BTreeMap<String, Transition>,
    present: bool,
    writer: bool,
    poisoned: bool,
}

pub(in crate::machine_plugins) struct Installations {
    root: PathBuf,
    owner: std::sync::Arc<()>,
    state: parking_lot::Mutex<State>,
}

impl Installations {
    pub(super) fn open(root: PathBuf) -> Result<Self> {
        let present = match root.symlink_metadata() {
            Ok(metadata) => {
                ensure!(metadata.is_dir(), "invalid installation journal directory");
                true
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(error.into()),
        };
        let mut slots = BTreeMap::new();
        if present {
            for entry in fs::read_dir(&root)? {
                let entry = entry?;
                let name = entry.file_name();
                let name = name
                    .to_str()
                    .context("invalid installation journal entry")?;
                if name.starts_with('.') && name.ends_with(".partial") {
                    continue;
                }
                ensure!(
                    slots.len() < MAX_SLOTS,
                    "installation journal capacity exceeded"
                );
                let file = OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                    .open(entry.path())?;
                ensure!(
                    file.metadata()?.is_file(),
                    "invalid installation journal record"
                );
                let mut bytes = Vec::new();
                file.take(MAX_SLOT_BYTES + 1).read_to_end(&mut bytes)?;
                ensure!(
                    bytes.len() as u64 <= MAX_SLOT_BYTES,
                    "installation record too large"
                );
                let record: SlotFile = serde_json::from_slice(&bytes)
                    .map_err(|_| anyhow::anyhow!("invalid installation evidence"))?;
                let transition = record.transition;
                transition.validate()?;
                ensure!(
                    name == format!("{}.json", transition.plugin_id)
                        && record.evidence_digest == digest(&serde_json::to_vec(&transition)?),
                    "installation evidence integrity failure"
                );
                ensure!(
                    slots
                        .insert(transition.plugin_id.clone(), transition)
                        .is_none(),
                    "duplicate installation slot"
                );
            }
        }
        Ok(Self {
            root,
            owner: std::sync::Arc::new(()),
            state: parking_lot::Mutex::new(State {
                slots,
                present,
                writer: false,
                poisoned: false,
            }),
        })
    }

    fn persist(&self, state: &mut State, transition: &Transition) -> Result<()> {
        let result = (|| {
            transition.validate()?;
            let record = SlotFile {
                evidence_digest: digest(&serde_json::to_vec(transition)?),
                transition: transition.clone(),
            };
            let bytes = serde_json::to_vec(&record)?;
            ensure!(
                bytes.len() as u64 <= MAX_SLOT_BYTES,
                "installation evidence too large"
            );
            atomic_write(
                &self.root.join(format!("{}.json", transition.plugin_id)),
                &bytes,
                0o600,
            )?;
            fs::File::open(&self.root)?.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            state.poisoned = true;
        } else {
            state
                .slots
                .insert(transition.plugin_id.clone(), transition.clone());
        }
        result
    }

    /// Explicit writer cutover only, called under the store lifecycle lock.
    /// Existing slots (including Unknown) are never reconstructed from links.
    pub(in crate::machine_plugins) fn enable(&self, existing: &[(String, String)]) -> Result<()> {
        let mut state = self.state.lock();
        ensure!(!state.poisoned, "installation journal unavailable");
        if !state.present {
            // Mark locally before any fallible filesystem operation: failure
            // cannot reopen the legacy untracked mutation path in this process.
            state.present = true;
            if let Err(error) = (|| -> Result<()> {
                fs::DirBuilder::new().mode(0o700).create(&self.root)?;
                fs::File::open(self.root.parent().context("installation journal parent")?)?
                    .sync_all()?;
                Ok(())
            })() {
                state.poisoned = true;
                return Err(error);
            }
        }
        for (plugin, generation) in existing {
            if state.slots.contains_key(plugin) {
                continue;
            }
            ensure!(
                state.slots.len() < MAX_SLOTS,
                "installation journal capacity exceeded"
            );
            self.persist(
                &mut state,
                &Transition {
                    schema: 1,
                    plugin_id: plugin.clone(),
                    revision: InstallationRevision::fresh()?,
                    previous_revision: None,
                    generation_digest: Some(generation.clone()),
                    effect: Effect::Adopt,
                    operation_digest: None,
                    outcome: Outcome::Stable {},
                },
            )?;
        }
        state.writer = true;
        Ok(())
    }

    pub(in crate::machine_plugins) fn tracked(&self, plugin: &str) -> bool {
        self.state.lock().slots.contains_key(plugin)
    }

    pub(in crate::machine_plugins) fn requires_cas(&self) -> bool {
        self.state.lock().present
    }

    pub(in crate::machine_plugins) fn ensure_writable(&self) -> Result<()> {
        let state = self.state.lock();
        ensure!(
            !state.poisoned && (!state.present || state.writer),
            "installation journal is reader-only or unavailable"
        );
        Ok(())
    }

    pub(super) fn admits(&self, step: &UninstallStep) -> bool {
        let state = self.state.lock();
        !state.poisoned
            && if state.present {
                state.writer && step.installation_revision.is_some()
            } else {
                step.installation_revision.is_none()
            }
    }

    pub(in crate::machine_plugins) fn ensure_unfenced(&self, plugin: &str) -> Result<()> {
        let state = self.state.lock();
        ensure!(
            !state.poisoned
                && !state
                    .slots
                    .get(plugin)
                    .is_some_and(|t| t.outcome == Outcome::Unknown {}),
            "Plugin installation requires reconciliation"
        );
        Ok(())
    }

    pub(in crate::machine_plugins) fn revision(
        &self,
        plugin: &str,
        generation: &str,
    ) -> Result<Option<InstallationRevision>> {
        let state = self.state.lock();
        ensure!(!state.poisoned, "installation journal unavailable");
        match state.slots.get(plugin) {
            Some(t)
                if t.outcome == Outcome::Stable {}
                    && t.generation_digest.as_deref() == Some(generation) =>
            {
                Ok(Some(t.revision.clone()))
            }
            Some(_) => bail!("Plugin installation requires reconciliation"),
            None if state.present => bail!("Plugin installation has not been adopted"),
            None => Ok(None),
        }
    }

    /// Must precede the first activation/credential-projection mutation. An
    /// identical artifact still receives an independently random incarnation.
    pub(in crate::machine_plugins) fn begin(
        &self,
        plugin: &str,
        current: Option<&str>,
        target: Option<&str>,
        effect: Effect,
        operation_digest: Option<String>,
    ) -> Result<Option<PendingInstallation>> {
        let mut state = self.state.lock();
        ensure!(!state.poisoned, "installation journal unavailable");
        if !state.present {
            return Ok(None);
        }
        ensure!(state.writer, "installation journal is reader-only");
        let previous = state.slots.get(plugin);
        match previous {
            Some(t) => ensure!(
                t.outcome == Outcome::Stable {} && t.generation_digest.as_deref() == current,
                "installation precondition changed"
            ),
            None => ensure!(
                current.is_none() && state.slots.len() < MAX_SLOTS,
                "installation slot unavailable"
            ),
        }
        let transition = Transition {
            schema: 1,
            plugin_id: plugin.to_owned(),
            revision: InstallationRevision::fresh()?,
            previous_revision: previous.map(|t| t.revision.clone()),
            generation_digest: target.map(str::to_owned),
            effect,
            operation_digest,
            outcome: Outcome::Unknown {},
        };
        self.persist(&mut state, &transition)?;
        Ok(Some(PendingInstallation {
            transition,
            owner: std::sync::Arc::clone(&self.owner),
        }))
    }

    pub(in crate::machine_plugins) fn finish(
        &self,
        pending: Option<PendingInstallation>,
    ) -> Result<()> {
        let Some(pending) = pending else {
            return Ok(());
        };
        ensure!(
            std::sync::Arc::ptr_eq(&self.owner, &pending.owner),
            "installation completion belongs to another process incarnation"
        );
        let mut transition = pending.transition;
        let mut state = self.state.lock();
        ensure!(
            !state.poisoned
                && state.writer
                && state.slots.get(&transition.plugin_id) == Some(&transition),
            "installation completion precondition changed"
        );
        transition.outcome = Outcome::Stable {};
        self.persist(&mut state, &transition)
    }
}

impl Transition {
    fn validate(&self) -> Result<()> {
        validate_plugin_id(&self.plugin_id)?;
        ensure!(
            self.schema == 1 && self.previous_revision.as_ref() != Some(&self.revision),
            "invalid installation evidence"
        );
        for value in [&self.generation_digest, &self.operation_digest]
            .into_iter()
            .flatten()
        {
            ensure!(
                format!("sha256:{}", digest_generation_name(value)?) == *value,
                "noncanonical installation digest"
            );
        }
        ensure!(
            match self.effect {
                Effect::Adopt =>
                    self.previous_revision.is_none()
                        && self.generation_digest.is_some()
                        && self.operation_digest.is_none()
                        && self.outcome == Outcome::Stable {},
                Effect::Install =>
                    self.generation_digest.is_some() && self.operation_digest.is_none(),
                Effect::Uninstall =>
                    self.previous_revision.is_some()
                        && self.generation_digest.is_none()
                        && self.operation_digest.is_some(),
            },
            "invalid installation transition"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install(journal: &Installations, current: Option<&str>) -> InstallationRevision {
        let release = digest(b"release");
        let pending = journal
            .begin("victoria", current, Some(&release), Effect::Install, None)
            .unwrap();
        assert!(journal.revision("victoria", &release).is_err());
        journal.finish(pending).unwrap();
        journal.revision("victoria", &release).unwrap().unwrap()
    }

    #[test]
    fn opening_and_observation_never_create_authority() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(DIRECTORY);
        let journal = Installations::open(path.clone()).unwrap();
        assert!(!journal.requires_cas());
        assert_eq!(
            journal.revision("victoria", &digest(b"release")).unwrap(),
            None
        );
        assert!(!path.exists());
    }

    #[test]
    fn reinstall_tombstone_and_reader_rollback_never_reuse_an_incarnation() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(DIRECTORY);
        let journal = Installations::open(path.clone()).unwrap();
        journal.enable(&[]).unwrap();
        let release = digest(b"release");
        let first = install(&journal, None);
        let second = install(&journal, Some(&release));
        assert_ne!(first, second);
        let removal = journal
            .begin(
                "victoria",
                Some(&release),
                None,
                Effect::Uninstall,
                Some(digest(b"approved removal")),
            )
            .unwrap();
        let tombstone = removal.as_ref().unwrap().transition.revision.clone();
        journal.finish(removal).unwrap();
        assert_ne!(second, tombstone);
        let bytes = fs::read(path.join("victoria.json")).unwrap();
        drop(journal);
        let reader = Installations::open(path.clone()).unwrap();
        assert!(reader.requires_cas());
        assert!(reader.ensure_writable().is_err());
        assert!(
            reader
                .begin("victoria", None, Some(&release), Effect::Install, None)
                .is_err()
        );
        assert_eq!(fs::read(path.join("victoria.json")).unwrap(), bytes);
        reader.enable(&[]).unwrap();
        let third = install(&reader, None);
        assert_ne!(third, first);
        assert_ne!(third, second);
        assert_ne!(third, tombstone);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(path.join("victoria.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[test]
    fn pending_reopen_adoption_and_stale_completion_cannot_clear_a_fence() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(DIRECTORY);
        let journal = Installations::open(path.clone()).unwrap();
        let release = digest(b"release");
        journal
            .enable(&[("victoria".into(), release.clone())])
            .unwrap();
        let pending = journal
            .begin(
                "victoria",
                Some(&release),
                Some(&release),
                Effect::Install,
                None,
            )
            .unwrap();
        let bytes = fs::read(path.join("victoria.json")).unwrap();
        drop(journal);
        let reopened = Installations::open(path.clone()).unwrap();
        reopened
            .enable(&[("victoria".into(), release.clone())])
            .unwrap();
        assert!(reopened.ensure_unfenced("victoria").is_err());
        assert!(reopened.ensure_unfenced("unrelated").is_ok());
        assert!(
            reopened
                .begin(
                    "victoria",
                    Some(&release),
                    Some(&release),
                    Effect::Install,
                    None
                )
                .is_err()
        );
        assert_eq!(fs::read(path.join("victoria.json")).unwrap(), bytes);
        // Even exact retained bytes cannot grant a new process completion authority.
        assert!(reopened.finish(pending).is_err());
        assert_eq!(fs::read(path.join("victoria.json")).unwrap(), bytes);
    }

    #[test]
    fn before_and_after_effect_storage_failures_do_not_acknowledge_or_clear_authority() {
        for after in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join(DIRECTORY);
            let journal = Installations::open(path.clone()).unwrap();
            journal.enable(&[]).unwrap();
            let release = digest(b"release");
            let pending = if after {
                Some(
                    journal
                        .begin("victoria", None, Some(&release), Effect::Install, None)
                        .unwrap(),
                )
            } else {
                None
            };
            let retained = root.path().join("retained");
            fs::rename(&path, &retained).unwrap();
            fs::write(&path, b"not a directory").unwrap();
            if let Some(pending) = pending {
                assert!(journal.finish(pending).is_err());
            } else {
                assert!(
                    journal
                        .begin("victoria", None, Some(&release), Effect::Install, None)
                        .is_err()
                );
            }
            assert!(journal.ensure_writable().is_err());
            assert!(journal.ensure_unfenced("victoria").is_err());
            assert!(journal.revision("victoria", &release).is_err());
            if after {
                let reopened = Installations::open(retained).unwrap();
                assert!(reopened.ensure_unfenced("victoria").is_err());
            }
        }
    }

    #[test]
    fn invalid_schema_symlinks_truncation_and_noncanonical_digests_fail_closed() {
        for corruption in 0..7 {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join(DIRECTORY);
            let journal = Installations::open(path.clone()).unwrap();
            journal.enable(&[]).unwrap();
            install(&journal, None);
            drop(journal);
            let file = path.join("victoria.json");
            let mut record: serde_json::Value =
                serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
            match corruption {
                0 => record["transition"]["schema"] = 2.into(),
                1 => record["transition"]["revision"] = digest(b"not an incarnation").into(),
                2 => record["transition"]["outcome"]["undo"] = true.into(),
                3 => {
                    record["transition"]["generation_digest"] =
                        format!("sha256:{}", "A".repeat(64)).into();
                }
                4 => {
                    fs::write(&file, b"{").unwrap();
                }
                5 => {
                    fs::write(
                        &file,
                        vec![b' '; usize::try_from(MAX_SLOT_BYTES).unwrap() + 1],
                    )
                    .unwrap();
                }
                _ => {
                    let external = root.path().join("external");
                    fs::rename(&file, &external).unwrap();
                    symlink(external, &file).unwrap();
                }
            }
            if corruption < 4 {
                fs::write(&file, serde_json::to_vec(&record).unwrap()).unwrap();
            }
            assert!(Installations::open(path).is_err());
        }
        let root = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let path = root.path().join(DIRECTORY);
        symlink(external.path(), &path).unwrap();
        assert!(Installations::open(path).is_err());
    }

    #[test]
    fn capacity_keeps_existing_slots_and_does_not_accept_unadopted_active_links() {
        let root = tempfile::tempdir().unwrap();
        let journal = Installations::open(root.path().join(DIRECTORY)).unwrap();
        journal.enable(&[]).unwrap();
        let release = digest(b"release");
        assert!(
            journal
                .begin(
                    "victoria",
                    Some(&release),
                    Some(&release),
                    Effect::Install,
                    None
                )
                .is_err()
        );
        let original = install(&journal, None);
        {
            let mut state = journal.state.lock();
            let template = state.slots["victoria"].clone();
            for n in 1..MAX_SLOTS {
                let mut slot = template.clone();
                slot.plugin_id = format!("slot-{n}");
                state.slots.insert(slot.plugin_id.clone(), slot);
            }
        }
        assert!(
            journal
                .begin("excess", None, Some(&release), Effect::Install, None)
                .is_err()
        );
        assert_eq!(
            journal.revision("victoria", &release).unwrap(),
            Some(original)
        );
        install(&journal, Some(&release));
    }
}
