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
use super::*;

const MOVES: &str = "legacy-moves";

/// Where one Provider's root credential moved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyMove {
    pub account_id: ProviderAccountId,
    pub root_generation: u64,
    pub account_generation: u64,
}

/// The account entries the move committed (empty when the account already
/// held a Ready credential) and the root wipes to distribute.
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
        Ok(self.root.join(MOVES).join(format!("{provider_id}.json")))
    }

    /// The recorded move of `provider_id`'s root credential, if any.
    pub fn legacy_move(&self, provider_id: &str) -> Option<LegacyMove> {
        if !self.account_id.is_default() || self.recipient_key.is_some() {
            return None;
        }
        let bytes = fs::read(self.move_path(provider_id).ok()?).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Move the root vault's Ready credential of `packages`' scope into the
    /// named `account` vault (unless that already holds a Ready credential:
    /// a replay never overwrites it) and wipe every root entry of the scope.
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
        let ready = account
            .status(requested_provider_id)
            .filter(|status| status.authentication_state == ServiceAuthenticationState::Ready);
        let mut outcome = LegacyMoveOutcome::default();
        let account_generation = if let Some(ready) = ready {
            ready.auth_generation
        } else {
            let source = source
                .as_ref()
                .context("the Service account holds no ready credential to move")?;
            let bundle: PortableCredentialBundle =
                serde_json::from_slice(&self.open_vault(source)?)
                    .context("decoding the Service account credential")?;
            outcome.account =
                account.commit_scope(packages, &bundle, None, requested_provider_id, (None, 0))?;
            outcome
                .account
                .iter()
                .map(|status| status.auth_generation)
                .max()
                .context("the account commit produced no status")?
        };
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
        if let Some(source) = &source {
            crate::provider_account::prepare_private_directory(&self.root.join(MOVES))?;
            let moved = serde_json::to_vec(&LegacyMove {
                account_id: account.account_id.clone(),
                root_generation: source.auth_generation,
                account_generation,
            })?;
            for provider_id in &live {
                atomic_write(&self.move_path(provider_id)?, &moved, 0o600)?;
            }
        }
        if !live.is_empty() {
            outcome.wiped = self.logout_locked(&mut providers, &live)?;
        }
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codex() -> ProviderPackage {
        let source: cowboy_provider_sdk::StandardProviderSource =
            serde_json::from_str(include_str!("../../plugins/codex/provider.json")).unwrap();
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
                account_generation: 1
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
        // Nothing to move from an empty Service account.
        let other: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        assert!(
            service
                .move_root_scope_into(&service.account(&other).unwrap(), packages, "codex")
                .is_err()
        );
    }
}
