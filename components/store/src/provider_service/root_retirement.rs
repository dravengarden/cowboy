//! The Service root vault after the legacy default cards retired
//! (`docs/legacy-card-retirement.md`).
//!
//! Once the Controller enables the policy, no root write may give a retired
//! Provider a credential again: a commit, a refresh promotion or a shared
//! scope adoption names its write destinations, and the retired members of
//! them are dropped (or the write is refused when the requested Provider is
//! itself retired). Root wipes and named account vaults are untouched, so a
//! rotation of a moved root generation still lands in its account.
use super::*;
use std::borrow::Cow;

/// Refusal of a root write whose requested Provider is retired.
pub const ROOT_RETIRED: &str = "named_account_required";

impl ProviderAuthService {
    /// Refuse every later root write to a Provider `retired` accepts. Set once
    /// at Controller startup; without it the vault keeps the old behavior.
    pub fn retire_root_providers(&self, retired: fn(&str) -> bool) {
        let _ = self.retired_root.set(retired);
    }

    fn root_retired(&self, provider_id: &str) -> bool {
        self.account_id.is_default()
            && self.recipient_key.is_none()
            && self
                .retired_root
                .get()
                .is_some_and(|retired| retired(provider_id))
    }

    /// The write destinations of a root write of `packages` requested for
    /// `requested_provider_id`: every member but the retired ones.
    pub(super) fn root_write_scope<'a>(
        &self,
        packages: &'a [ProviderPackage],
        requested_provider_id: Option<&str>,
    ) -> Result<Cow<'a, [ProviderPackage]>> {
        if requested_provider_id.is_some_and(|provider| self.root_retired(provider)) {
            anyhow::bail!(ROOT_RETIRED);
        }
        if !packages
            .iter()
            .any(|package| self.root_retired(&package.manifest.id))
        {
            return Ok(Cow::Borrowed(packages));
        }
        Ok(Cow::Owned(
            packages
                .iter()
                .filter(|package| !self.root_retired(&package.manifest.id))
                .cloned()
                .collect(),
        ))
    }

    /// A Machine's root refresh of a retired Provider is stale: the Service
    /// generation it restores is the wipe. (A rotation of a moved generation
    /// was routed to its account before reaching the root vault.)
    pub(super) fn retired_refresh(
        &self,
        packages: &[ProviderPackage],
        requested_provider_id: &str,
    ) -> Option<ProviderAuthRefreshResult> {
        if !self.root_retired(requested_provider_id) {
            return None;
        }
        let providers = self.providers.read();
        Some(ProviderAuthRefreshResult::Stale {
            current_generation: maximum_auth_generation(packages, &providers),
        })
    }

    /// Finish every legacy move whose root wipe already landed: a Controller
    /// that stopped between the wipe and the journal's completion left it
    /// unfinished, and no adoption writer remains to finish it. Without the
    /// completion a Machine's rotation of the moved generation could not be
    /// promoted into its account. Refuses a journal whose root entry still
    /// holds a credential (the previous Controller must finish that move).
    /// Returns the Providers it completed. Idempotent.
    pub fn recover_legacy_moves(&self) -> Result<Vec<String>> {
        if !self.account_id.is_default() || self.recipient_key.is_some() {
            return Ok(Vec::new());
        }
        let directory = self.root.join(LEGACY_MOVES);
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error).context("reading the legacy move journals"),
        };
        let mut pending = BTreeMap::<ProviderAccountId, (LegacyMove, Vec<String>)>::new();
        for entry in entries {
            let name = entry?.file_name();
            let Some(provider_id) = name.to_str().and_then(|name| name.strip_suffix(".json"))
            else {
                continue;
            };
            let Some(moved) = self.read_move(provider_id) else {
                anyhow::bail!("legacy move journal of {provider_id} is unreadable");
            };
            if moved.completed {
                continue;
            }
            pending
                .entry(moved.account_id.clone())
                .or_insert_with(|| (moved.clone(), Vec::new()))
                .1
                .push(provider_id.to_owned());
        }
        let providers = self.providers.write();
        let mut completed = Vec::new();
        for (account_id, (mut moved, journaled)) in pending {
            let account = self.account(&account_id)?;
            for provider_id in &journaled {
                ensure!(
                    providers.get(provider_id).is_none_or(|stored| {
                        stored.authentication_state == ServiceAuthenticationState::SignedOut
                            && stored.action != ProviderAuthAction::Apply
                    }),
                    "unfinished legacy move of {provider_id}: its Service credential was not wiped; finish the adoption with the previous Controller"
                );
            }
            // Revoking the account deleted its copy: no rotation can land
            // there, so the journal stays unfinished and routes nothing.
            if moved.account_generation == 0
                || !journaled
                    .iter()
                    .any(|provider_id| account.status(provider_id).is_some())
            {
                tracing::warn!(account = account_id.as_str(), providers = ?journaled,
                    "unfinished legacy move has no account copy; left unrouted");
                continue;
            }
            moved.completed = true;
            self.write_move(&journaled, &moved)?;
            completed.extend(journaled);
        }
        Ok(completed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(source: &str, id: &str) -> ProviderPackage {
        let source: cowboy_provider_sdk::StandardProviderSource =
            serde_json::from_str(source).unwrap();
        let mut package = cowboy_provider_sdk::build_package(source.compile().unwrap()).unwrap();
        package.manifest.id = id.to_owned();
        package
    }

    /// Two Providers of one shared scope; `codex-deepseek` plays the card.
    fn scope() -> [ProviderPackage; 2] {
        [
            package(
                include_str!("../../../../plugins/claude-deepseek/provider.json"),
                "claude-deepseek",
            ),
            package(
                include_str!("../../../../plugins/codex-deepseek/provider.json"),
                "codex-deepseek",
            ),
        ]
    }

    fn retired(provider_id: &str) -> bool {
        provider_id == "codex-deepseek"
    }

    fn bundle() -> PortableCredentialBundle {
        PortableCredentialBundle {
            portable_schema: "deepseek-api-key-v1".into(),
            method_id: "api-key".into(),
            values: BTreeMap::from([(
                "api_key".into(),
                base64::engine::general_purpose::STANDARD.encode(b"deepseek-secret"),
            )]),
        }
    }

    #[test]
    fn no_root_write_reaches_a_retired_provider_once_retired() {
        let root = tempfile::tempdir().unwrap();
        let service = ProviderAuthService::open(root.path()).unwrap();
        let [claude, codex] = scope();
        let both = [claude.clone(), codex.clone()];
        service.retire_root_providers(retired);
        // The retired Provider itself is refused, also through its scope.
        let refused = service
            .commit_shared(&both, &bundle(), None, "codex-deepseek", None)
            .unwrap_err();
        assert_eq!(refused.to_string(), ROOT_RETIRED);
        assert!(service.commit(&codex, &bundle(), None, None).is_err());
        // A shared-scope sign-in of another member skips it.
        let committed = service
            .commit_shared(&both, &bundle(), None, "claude-deepseek", None)
            .unwrap();
        assert_eq!(committed.len(), 1);
        assert_eq!(committed[0].provider_id, "claude-deepseek");
        assert!(service.status("codex-deepseek").is_none());
        // Scope adoption never extends the credential to it.
        assert!(service.adopt_shared_scope(&both).unwrap().is_empty());
        assert!(service.status("codex-deepseek").is_none());
        // A Machine refresh of its root lineage is stale: the wipe is restored.
        assert!(matches!(
            service
                .compare_and_swap_refresh(&both, &bundle(), "codex-deepseek", 0)
                .unwrap(),
            ProviderAuthRefreshResult::Stale { .. }
        ));
        // Root wipes and named account vaults are untouched.
        let id: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let account = service.account(&id).unwrap();
        assert_eq!(
            account
                .commit_shared(&both, &bundle(), None, "codex-deepseek", None)
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            service.logout("claude-deepseek").unwrap().authentication_state,
            ServiceAuthenticationState::SignedOut
        );
    }

    #[test]
    fn a_move_interrupted_after_its_root_wipe_is_finished_once() {
        let root = tempfile::tempdir().unwrap();
        let service = ProviderAuthService::open(root.path()).unwrap();
        let [claude, codex] = scope();
        let both = [claude, codex];
        assert!(service.recover_legacy_moves().unwrap().is_empty());
        service
            .commit_shared(&both, &bundle(), None, "claude-deepseek", None)
            .unwrap();
        let id: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let account = service.account(&id).unwrap();
        let providers = ["claude-deepseek".to_owned(), "codex-deepseek".to_owned()];
        // The journal after the account copy, before the root wipe.
        let pending = LegacyMove {
            account_id: id.clone(),
            root_generation: 1,
            account_generation: 1,
            completed: false,
        };
        service.write_move(&providers, &pending).unwrap();
        account
            .commit_shared(&both, &bundle(), None, "claude-deepseek", None)
            .unwrap();
        // The root still holds the credential: only its adoption may finish.
        let refused = service.recover_legacy_moves().unwrap_err().to_string();
        assert!(refused.contains("was not wiped"), "{refused}");
        assert!(service.legacy_move("claude-deepseek").is_none());
        // Stopped between the wipe and the completion.
        service.logout_shared(&providers).unwrap();
        let mut recovered = service.recover_legacy_moves().unwrap();
        recovered.sort();
        assert_eq!(recovered, providers);
        for provider in &providers {
            assert_eq!(
                service.legacy_move(provider),
                Some(LegacyMove {
                    completed: true,
                    ..pending.clone()
                })
            );
        }
        assert!(service.recover_legacy_moves().unwrap().is_empty());
    }

    #[test]
    fn a_move_without_an_account_copy_is_left_unrouted() {
        let root = tempfile::tempdir().unwrap();
        let service = ProviderAuthService::open(root.path()).unwrap();
        let id: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let pending = LegacyMove {
            account_id: id,
            root_generation: 1,
            account_generation: 1,
            completed: false,
        };
        service
            .write_move(&["codex-deepseek".to_owned()], &pending)
            .unwrap();
        assert!(service.recover_legacy_moves().unwrap().is_empty());
        assert!(service.legacy_move("codex-deepseek").is_none());
    }
}
