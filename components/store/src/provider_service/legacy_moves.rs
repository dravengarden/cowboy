//! Moving the legacy Service ("default") account's credential into a named
//! account vault, without signing in again.
//!
//! The copy and the wipe of the root entries happen atomically under the
//! root vault's write lock: no compare-and-swap refresh or commit of the
//! root lineage can land between them, so no rotation promoted at the
//! Service is lost. A rotation a Machine made of the moved root generation
//! before the wipe reached it is still promoted, into the account: each move
//! is recorded beside the vault and a refresh of exactly that generation is
//! redirected to the account's compare-and-swap (`legacy_move`).
//!
//! Adoption retired with the legacy cards (`docs/legacy-card-retirement.md`).
//! The journals it wrote and their fence and rotation route remain: a Machine
//! that was offline during an adoption may still propose a rotation of a
//! moved generation. The move itself stays only to produce journals in
//! tests; `recover_legacy_moves` finishes a journal an interrupted adoption
//! left behind.
use super::*;

pub(super) const LEGACY_MOVES: &str = "legacy-moves";

/// Where one Provider's root credential moved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyMove {
    pub account_id: ProviderAccountId,
    pub root_generation: u64,
    pub account_generation: u64,
    /// The root entries are wiped. Until then the record is a journal that
    /// fences every root commit and refresh of the scope, and only the same
    /// account may finish it.
    #[serde(default)]
    pub completed: bool,
}

/// The account entries the move committed (empty when the account already
/// held a Ready credential) and the root wipes to distribute.
#[cfg(test)]
#[derive(Debug, Default)]
pub struct LegacyMoveOutcome {
    pub account: Vec<ProviderAuthenticationStatus>,
    pub wiped: Vec<ProviderAuthenticationStatus>,
}

impl ProviderAuthService {
    fn move_path(&self, provider_id: &str) -> Result<PathBuf> {
        ensure!(
            !provider_id.is_empty()
                && provider_id.len() <= 64
                && provider_id
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
            "invalid Provider id"
        );
        Ok(self.root.join(LEGACY_MOVES).join(format!("{provider_id}.json")))
    }

    pub(super) fn read_move(&self, provider_id: &str) -> Option<LegacyMove> {
        if !self.account_id.is_default() || self.recipient_key.is_some() {
            return None;
        }
        let bytes = fs::read(self.move_path(provider_id).ok()?).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// The completed move of `provider_id`'s root credential, if any: the
    /// target of a rotation of its moved root generation.
    pub fn legacy_move(&self, provider_id: &str) -> Option<LegacyMove> {
        self.read_move(provider_id).filter(|moved| moved.completed)
    }

    /// Called under the vault write lock by every root commit and refresh:
    /// an unfinished move fences the root lineage until it is reconciled.
    pub(super) fn ensure_no_pending_move(&self, packages: &[ProviderPackage]) -> Result<()> {
        for package in packages {
            ensure!(
                self.read_move(&package.manifest.id)
                    .is_none_or(|moved| moved.completed),
                "legacy_adoption_in_progress"
            );
        }
        Ok(())
    }

    pub(super) fn write_move(&self, providers: &[String], moved: &LegacyMove) -> Result<()> {
        crate::provider_account::prepare_private_directory(&self.root.join(LEGACY_MOVES))?;
        let bytes = serde_json::to_vec(moved)?;
        for provider_id in providers {
            atomic_write(&self.move_path(provider_id)?, &bytes, 0o600)?;
        }
        Ok(())
    }

    /// Move the root vault's Ready credential of `packages`' scope into the
    /// named `account` vault (unless that already holds a Ready credential:
    /// a replay never overwrites it) and wipe every root entry of the scope.
    #[cfg(test)]
    pub fn move_root_scope_into(
        &self,
        account: &Self,
        packages: &[ProviderPackage],
        requested_provider_id: &str,
    ) -> Result<LegacyMoveOutcome> {
        ensure!(
            self.account_id.is_default() && self.recipient_key.is_none(),
            "legacy moves start at the Service account vault"
        );
        ensure!(
            !account.account_id.is_default() && account.recipient_key.is_none(),
            "legacy moves target a named account vault"
        );
        let mut providers = self.providers.write();
        let journal = self.read_move(requested_provider_id);
        match &journal {
            // Finished for this account: a replay changes nothing.
            Some(moved) if moved.completed && moved.account_id == account.account_id => {
                return Ok(LegacyMoveOutcome::default());
            }
            Some(moved) if !moved.completed && moved.account_id != account.account_id => {
                anyhow::bail!("legacy_adoption_in_progress");
            }
            _ => {}
        }
        let pending = journal.filter(|moved| !moved.completed);
        let generation = maximum_auth_generation(packages, &providers);
        let source = packages
            .iter()
            .filter_map(|package| providers.get(&package.manifest.id))
            .find(|stored| {
                generation > 0
                    && stored.auth_generation == generation
                    && stored.action == ProviderAuthAction::Apply
                    && stored.authentication_state == ServiceAuthenticationState::Ready
                    && stored.promotion_commitment.is_none()
            })
            .cloned();
        let schema = source.as_ref().map_or_else(
            || packages[0].manifest.authentication.portable_schema.clone(),
            |source| source.portable_schema.clone(),
        );
        let live: Vec<String> = providers
            .values()
            .filter(|stored| {
                stored.portable_schema == schema && stored.action == ProviderAuthAction::Apply
            })
            .map(|stored| stored.provider_id.clone())
            .collect();
        let mut outcome = LegacyMoveOutcome::default();
        let mut moved = if let Some(pending) = pending {
            // The journal fenced the root since the copy: its entries still
            // hold exactly the moved generation, so only the wipe remains.
            ensure!(
                account.status(requested_provider_id).is_some(),
                "the journaled account copy is missing"
            );
            pending
        } else {
            // A target that already holds a credential is no proof of a
            // transfer: never wipe a root lineage it did not copy.
            ensure!(
                account.status(requested_provider_id).is_none(),
                "account_already_holds_a_credential"
            );
            let source = source
                .as_ref()
                .context("the Service account holds no ready credential to move")?;
            let bundle: PortableCredentialBundle =
                serde_json::from_slice(&self.open_vault(source)?)
                    .context("decoding the Service account credential")?;
            let mut moved = LegacyMove {
                account_id: account.account_id.clone(),
                root_generation: source.auth_generation,
                account_generation: 0,
                completed: false,
            };
            // Journal first: a crash after the copy leaves the root fenced.
            self.write_move(&live, &moved)?;
            outcome.account =
                account.commit_scope(packages, &bundle, None, requested_provider_id, (None, 0))?;
            moved.account_generation = outcome
                .account
                .iter()
                .map(|status| status.auth_generation)
                .max()
                .context("the account commit produced no status")?;
            self.write_move(&live, &moved)?;
            moved
        };
        if !live.is_empty() {
            outcome.wiped = self.logout_locked(&mut providers, &live)?;
        }
        moved.completed = true;
        let journaled: Vec<String> = live
            .iter()
            .cloned()
            .chain(std::iter::once(requested_provider_id.to_owned()))
            .collect();
        self.write_move(&journaled, &moved)?;
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codex() -> ProviderPackage {
        let source: cowboy_provider_sdk::StandardProviderSource =
            serde_json::from_str(include_str!("../../../../plugins/codex/provider.json")).unwrap();
        cowboy_provider_sdk::build_package(source.compile().unwrap()).unwrap()
    }

    fn bundle(token: &str) -> PortableCredentialBundle {
        PortableCredentialBundle {
            portable_schema: "codex-auth-v1".into(),
            method_id: "chatgpt".into(),
            values: BTreeMap::from([(
                "auth_json".into(),
                base64::engine::general_purpose::STANDARD.encode(token),
            )]),
        }
    }

    #[test]
    fn a_legacy_credential_moves_once_and_its_root_lineage_is_wiped() {
        let root = tempfile::tempdir().unwrap();
        let service = ProviderAuthService::open(root.path()).unwrap();
        let package = codex();
        let packages = std::slice::from_ref(&package);
        service
            .commit(&package, &bundle("{\"t\":1}"), None, Some(0))
            .unwrap();
        service
            .compare_and_swap_refresh(packages, &bundle("{\"t\":2}"), "codex", 1)
            .unwrap();
        let id: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let account = service.account(&id).unwrap();
        let moved = service
            .move_root_scope_into(&account, packages, "codex")
            .unwrap();
        assert_eq!(moved.account[0].auth_generation, 1);
        assert_eq!(moved.wiped[0].auth_generation, 3);
        assert_eq!(
            service.status("codex").unwrap().authentication_state,
            ServiceAuthenticationState::SignedOut
        );
        // The account holds the newest root rotation, not the first sign-in.
        let machine = PublicKey::from(&StaticSecret::from([9_u8; 32]));
        let key = base64::engine::general_purpose::STANDARD.encode(machine.as_bytes());
        assert_eq!(
            account
                .seal_for_machine("codex", &key)
                .unwrap()
                .auth_generation,
            1
        );
        assert_eq!(
            service.legacy_move("codex"),
            Some(LegacyMove {
                account_id: id.clone(),
                root_generation: 2,
                account_generation: 1,
                completed: true,
            })
        );
        // The fenced root lineage refuses a rotation of the moved generation;
        // the account takes it instead.
        assert!(matches!(
            service
                .compare_and_swap_refresh(packages, &bundle("{\"t\":3}"), "codex", 2)
                .unwrap(),
            ProviderAuthRefreshResult::Stale { .. }
        ));
        assert!(matches!(
            account
                .compare_and_swap_refresh(packages, &bundle("{\"t\":3}"), "codex", 1)
                .unwrap(),
            ProviderAuthRefreshResult::Updated(_)
        ));
        // A replay never overwrites the account with the wiped root.
        let replay = service
            .move_root_scope_into(&account, packages, "codex")
            .unwrap();
        assert!(replay.account.is_empty() && replay.wiped.is_empty());
        assert_eq!(account.status("codex").unwrap().auth_generation, 2);
        // After the move, the Service account signs in again: another
        // replay (of any operation) never touches the new root lineage.
        service
            .commit(&package, &bundle("{\"t\":9}"), None, None)
            .unwrap();
        let replay = service
            .move_root_scope_into(&account, packages, "codex")
            .unwrap();
        assert!(replay.wiped.is_empty());
        assert_eq!(
            service.status("codex").unwrap().authentication_state,
            ServiceAuthenticationState::Ready
        );
        // A move needs a ready Service credential.
        let empty = tempfile::tempdir().unwrap();
        let empty = ProviderAuthService::open(empty.path()).unwrap();
        let other: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        assert!(
            empty
                .move_root_scope_into(&empty.account(&other).unwrap(), packages, "codex")
                .is_err()
        );
    }

    #[test]
    fn an_unfinished_move_fences_the_root_until_its_account_finishes_it() {
        let root = tempfile::tempdir().unwrap();
        let service = ProviderAuthService::open(root.path()).unwrap();
        let package = codex();
        let packages = std::slice::from_ref(&package);
        service
            .commit(&package, &bundle("{\"t\":1}"), None, Some(0))
            .unwrap();
        let id: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let account = service.account(&id).unwrap();
        // Simulate a crash after the journaled copy, before the wipe.
        let pending = LegacyMove {
            account_id: id.clone(),
            root_generation: 1,
            account_generation: 0,
            completed: false,
        };
        service.write_move(&["codex".to_owned()], &pending).unwrap();
        account
            .commit_scope(packages, &bundle("{\"t\":1}"), None, "codex", (None, 0))
            .unwrap();
        // No root rotation or sign-in can land while the move is unfinished.
        let fenced = service
            .compare_and_swap_refresh(packages, &bundle("{\"t\":2}"), "codex", 1)
            .unwrap_err();
        assert_eq!(fenced.to_string(), "legacy_adoption_in_progress");
        assert!(
            service
                .commit(&package, &bundle("{\"t\":3}"), None, None)
                .is_err()
        );
        assert!(service.legacy_move("codex").is_none());
        // Another account cannot take over the unfinished move.
        let other: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let other = service.account(&other).unwrap();
        assert!(
            service
                .move_root_scope_into(&other, packages, "codex")
                .is_err()
        );
        // The same account finishes it: the root is wiped, nothing recopied.
        let finished = service
            .move_root_scope_into(&account, packages, "codex")
            .unwrap();
        assert!(finished.account.is_empty());
        assert_eq!(finished.wiped.len(), 1);
        assert_eq!(account.status("codex").unwrap().auth_generation, 1);
        assert!(service.legacy_move("codex").unwrap().completed);
        service
            .commit(&package, &bundle("{\"t\":4}"), None, None)
            .unwrap();
    }
}
