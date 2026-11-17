//! Named portable accounts: a Service-held secret committed at account scope
//! (never a Machine grant) and sealed to every enrolled Machine on demand.
//! Subscription Providers commit their sign-in here the same way; native-only
//! Providers (Claude) stay fenced out by `portable_credentials_allowed`.

use super::*;

impl ProviderAuthService {
    pub(super) fn require_portable_packages(&self, packages: &[ProviderPackage]) -> Result<()> {
        for package in packages {
            crate::provider_account::portable_credentials_allowed(
                &self.account_id,
                &package.manifest.id,
            )
            .map_err(anyhow::Error::msg)?;
        }
        Ok(())
    }

    /// Commit a named account's credential (an entered key, a subscription
    /// sign-in or a migrated native profile) for every member of its scope,
    /// as a generation above `floor`: the account's native login generations,
    /// so no vault generation ever names a native profile on any Machine.
    pub fn commit_account(
        &self,
        packages: &[ProviderPackage],
        bundle: &PortableCredentialBundle,
        requested_provider_id: &str,
        floor: u64,
    ) -> Result<Vec<ProviderAuthenticationStatus>> {
        ensure!(
            !self.account_id.is_default() && self.recipient_key.is_none(),
            "account credentials require a named account vault"
        );
        self.commit_scope(packages, bundle, None, requested_provider_id, (None, floor))
    }

    /// Lookup-only: an untrusted observation or admission recheck must never
    /// create a named account vault. Only the root vault resolves accounts.
    #[must_use]
    pub fn existing_account(&self, account: &ProviderAccountId) -> Option<Arc<Self>> {
        if !self.account_id.is_default() || account.is_default() {
            return None;
        }
        self.accounts.lock().get(account).cloned()
    }

    /// Durable account-scope entries of every opened named account. These
    /// are portable credentials (or their wipes); Machine-bound native grants
    /// live in nested scopes and are excluded.
    #[must_use]
    pub fn portable_account_statuses(&self) -> Vec<ProviderAuthenticationStatus> {
        if !self.account_id.is_default() {
            return Vec::new();
        }
        let accounts: Vec<_> = self.accounts.lock().values().cloned().collect();
        accounts
            .iter()
            .flat_map(|account| account.replica_statuses())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deepseek_packages() -> Vec<ProviderPackage> {
        [
            include_str!("../../plugins/claude-deepseek/provider.json"),
            include_str!("../../plugins/codex-deepseek/provider.json"),
            include_str!("../../plugins/deepseek-harness/provider.json"),
        ]
        .into_iter()
        .map(|source| {
            let source: cowboy_provider_sdk::StandardProviderSource =
                serde_json::from_str(source).unwrap();
            cowboy_provider_sdk::build_package(source.compile().unwrap()).unwrap()
        })
        .collect()
    }

    fn key_bundle(packages: &[ProviderPackage], key: &str) -> PortableCredentialBundle {
        PortableCredentialBundle {
            portable_schema: packages[0].manifest.authentication.portable_schema.clone(),
            method_id: "api-key".into(),
            values: BTreeMap::from([(
                "api_key".into(),
                base64::engine::general_purpose::STANDARD.encode(key),
            )]),
        }
    }

    #[test]
    fn a_portable_account_key_lives_at_account_scope_and_seals_to_any_machine() {
        let root = tempfile::tempdir().unwrap();
        let service = ProviderAuthService::open(root.path()).unwrap();
        let packages = deepseek_packages();
        let id: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        assert!(service.existing_account(&id).is_none());
        let account = service.account(&id).unwrap();
        let statuses = account
            .commit_shared(
                &packages,
                &key_bundle(&packages, "sk-named"),
                None,
                "codex-deepseek",
                Some(0),
            )
            .unwrap();
        assert_eq!(statuses.len(), 3);
        assert!(statuses.iter().all(|status| status.account_id == id
            && status.recipient_key.is_none()
            && status.auth_generation == 1));
        // The default account is untouched; nothing falls back to it.
        assert!(service.status("codex-deepseek").is_none());
        assert_eq!(service.portable_account_statuses().len(), 3);
        assert!(Arc::ptr_eq(
            &service.existing_account(&id).unwrap(),
            &account
        ));
        for seed in [7_u8, 9] {
            let machine = PublicKey::from(&StaticSecret::from([seed; 32]));
            let key = base64::engine::general_purpose::STANDARD.encode(machine.as_bytes());
            let envelope = account.seal_for_machine("claude-deepseek", &key).unwrap();
            assert_eq!(envelope.account_id, id);
            assert_eq!(envelope.envelope_schema, 2);
            assert_eq!(envelope.action, ProviderAuthAction::Apply);
            assert_eq!(envelope.auth_generation, 1);
        }
        // Registration holds the exact Ready account generation.
        assert!(
            account
                .with_scheduling_generation("deepseek-harness", true, Some(1), || ())
                .is_ok()
        );
        assert!(
            account
                .with_scheduling_generation("deepseek-harness", true, Some(2), || ())
                .is_err()
        );
        drop(account);
        drop(service);
        let reopened = ProviderAuthService::open(root.path()).unwrap();
        assert_eq!(reopened.portable_account_statuses().len(), 3);
    }

    #[test]
    fn a_later_scope_member_adopts_a_named_account_key_but_never_a_grant() {
        let root = tempfile::tempdir().unwrap();
        let service = ProviderAuthService::open(root.path()).unwrap();
        let packages = deepseek_packages();
        let id: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let account = service.account(&id).unwrap();
        account
            .commit_shared(
                &packages[..2],
                &key_bundle(&packages, "sk-named"),
                None,
                "claude-deepseek",
                Some(0),
            )
            .unwrap();
        let adopted = account.adopt_shared_scope(&packages).unwrap();
        assert_eq!(adopted.len(), 1);
        assert_eq!(adopted[0].provider_id, "deepseek-harness");
        assert_eq!(adopted[0].account_id, id);
        assert_eq!(adopted[0].auth_generation, 1);
        let machine = PublicKey::from(&StaticSecret::from([5_u8; 32]));
        let grant = account
            .machine_grant(&base64::engine::general_purpose::STANDARD.encode(machine.as_bytes()))
            .unwrap();
        assert!(grant.adopt_shared_scope(&packages).unwrap().is_empty());
    }

    #[test]
    fn native_login_providers_cannot_hold_a_named_portable_key() {
        let root = tempfile::tempdir().unwrap();
        let service = ProviderAuthService::open(root.path()).unwrap();
        let source: cowboy_provider_sdk::StandardProviderSource =
            serde_json::from_str(include_str!("../../plugins/claude-code/provider.json")).unwrap();
        let package = cowboy_provider_sdk::build_package(source.compile().unwrap()).unwrap();
        let id: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        assert!(
            service
                .account(&id)
                .unwrap()
                .require_portable_packages(std::slice::from_ref(&package))
                .is_err()
        );
    }

    #[test]
    fn a_subscription_sign_in_is_one_account_lineage_above_native_generations() {
        let root = tempfile::tempdir().unwrap();
        let service = ProviderAuthService::open(root.path()).unwrap();
        let source: cowboy_provider_sdk::StandardProviderSource =
            serde_json::from_str(include_str!("../../plugins/codex/provider.json")).unwrap();
        let package = cowboy_provider_sdk::build_package(source.compile().unwrap()).unwrap();
        let bundle = |token: &str| PortableCredentialBundle {
            portable_schema: "codex-auth-v1".into(),
            method_id: "chatgpt".into(),
            values: BTreeMap::from([(
                "auth_json".into(),
                base64::engine::general_purpose::STANDARD.encode(token),
            )]),
        };
        let id: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let account = service.account(&id).unwrap();
        let packages = std::slice::from_ref(&package);
        // The account signed in natively up to generation 12 somewhere.
        let first = account
            .commit_account(packages, &bundle("{\"t\":1}"), "codex", 12)
            .unwrap();
        assert_eq!(first[0].auth_generation, 13);
        assert!(first[0].recipient_key.is_none());
        assert!(service.status("codex").is_none());
        // Every Machine receives the same account-scope credential.
        for seed in [3_u8, 4] {
            let machine = PublicKey::from(&StaticSecret::from([seed; 32]));
            let key = base64::engine::general_purpose::STANDARD.encode(machine.as_bytes());
            let envelope = account.seal_for_machine("codex", &key).unwrap();
            assert_eq!(
                (envelope.account_id.clone(), envelope.auth_generation),
                (id.clone(), 13)
            );
        }
        // Two Machines refresh generation 13: only one wins, the other is stale.
        let winner = account
            .compare_and_swap_refresh(packages, &bundle("{\"t\":2}"), "codex", 13)
            .unwrap();
        assert!(
            matches!(winner, ProviderAuthRefreshResult::Updated(ref s) if s[0].auth_generation == 14)
        );
        let loser = account
            .compare_and_swap_refresh(packages, &bundle("{\"t\":3}"), "codex", 13)
            .unwrap();
        assert_eq!(
            loser,
            ProviderAuthRefreshResult::Stale {
                current_generation: 14
            }
        );
        // A later sign-in stays above the floor and the lineage.
        let again = account
            .commit_account(packages, &bundle("{\"t\":4}"), "codex", 12)
            .unwrap();
        assert_eq!(again[0].auth_generation, 15);
        assert!(service.account(&ProviderAccountId::default()).is_err());
        assert!(
            ProviderAuthService::open(root.path()).err().is_some(),
            "the vault stays owned by one instance"
        );
    }
}
