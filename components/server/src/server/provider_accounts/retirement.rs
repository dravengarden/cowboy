//! The retired legacy default cards (`docs/legacy-card-retirement.md`): the
//! Service `default` account of every card-family Provider. Nothing signs one
//! in, starts or resumes a session on one, or collects its usage; historical
//! sessions stay readable and continue in a new agent on a named account.
use super::*;
use crate::provider_service::{
    ProviderAuthService, ProviderAuthenticationStatus, ServiceAuthenticationState,
};
use crate::store::provider_accounts::legacy_cards;

/// The visible wait of a prompt to a retired `default` session.
pub(in crate::server) const SESSION_RETIRED: &str = "This session ran on a retired legacy default account; continue it in a new agent on a named account";

/// Refusal of a Service (`default`) sign-in or credential commit.
pub(in crate::server) const LOGIN_RETIRED: &str = "named_account_required: the legacy default account is retired; add a named account in Settings → Subscriptions";

/// Card-family Providers whose root vault entry still holds a credential.
fn live(statuses: &[ProviderAuthenticationStatus]) -> Vec<String> {
    statuses
        .iter()
        .filter(|status| {
            legacy_cards::retired_provider(&status.provider_id)
                && status.authentication_state != ServiceAuthenticationState::SignedOut
        })
        .map(|status| status.provider_id.clone())
        .collect()
}

/// Open the Service root vault behind the startup gate.
pub(in crate::server) fn open_provider_auth(
    data_dir: &std::path::Path,
) -> anyhow::Result<Arc<ProviderAuthService>> {
    let auth = Arc::new(ProviderAuthService::open(data_dir)?);
    admit_startup(&auth)?;
    Ok(auth)
}

/// Startup gate: refuse to run while a legacy card still holds a Service
/// credential. The deleted code can neither adopt nor serve it; the previous
/// Controller still can, and the component activator restores it.
/// An adoption interrupted after its root wipe left its move journal
/// unfinished; it is finished here, before anything can propose a rotation.
/// From then on no root write reaches a card-family Provider.
pub(in crate::server) fn admit_startup(auth: &ProviderAuthService) -> anyhow::Result<()> {
    let live = live(&auth.statuses());
    anyhow::ensure!(
        live.is_empty(),
        "legacy default cards still hold Service credentials ({}): adopt them into named accounts or sign them out with the previous Controller before installing this release",
        live.join(", ")
    );
    let recovered = auth.recover_legacy_moves()?;
    if !recovered.is_empty() {
        tracing::warn!(providers = ?recovered, "finished interrupted legacy credential moves");
    }
    auth.retire_root_providers(legacy_cards::retired_provider);
    Ok(())
}

/// Whether the `default` account of `provider` is a retired card: the
/// Provider is of a card family and its root entry exists but is not Ready
/// (wiped by an adoption or a sign-out). A Provider that never had a root
/// entry keeps its ordinary "not authenticated" failure.
pub(in crate::server) fn retired(state: &AppState, provider: &str) -> bool {
    retired_entry(provider, state.provider_auth.status(provider).as_ref())
}

/// The refusal of any input to `session` (a queued prompt, a direct prompt
/// or steering) when it ran on a retired card's `default` account.
pub(in crate::server) fn session_refusal(state: &AppState, session: &str) -> Option<&'static str> {
    session_retired(state, &state.hub.session_info(session)?.meta).then_some(SESSION_RETIRED)
}

/// Whether `meta` is a session of a retired card's `default` account. Its
/// queue is parked for good: never retried, woken or counted as a waiter.
pub(in crate::server) fn session_retired(state: &AppState, meta: &crate::core::SessionMeta) -> bool {
    retired_session(
        &meta.provider,
        &meta.provider_account_id,
        state.provider_auth.status(&meta.provider).as_ref(),
    )
}

fn retired_session(
    provider: &str,
    account: &crate::provider_account::ProviderAccountId,
    root: Option<&ProviderAuthenticationStatus>,
) -> bool {
    account.is_default() && retired_entry(provider, root)
}

fn retired_entry(provider: &str, root: Option<&ProviderAuthenticationStatus>) -> bool {
    legacy_cards::retired_provider(provider)
        && root
            .is_some_and(|status| status.authentication_state != ServiceAuthenticationState::Ready)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(provider: &str, state: ServiceAuthenticationState) -> ProviderAuthenticationStatus {
        ProviderAuthenticationStatus {
            provider_id: provider.to_owned(),
            account_id: crate::provider_account::ProviderAccountId::default(),
            recipient_key: None,
            auth_generation: 3,
            authentication_state: state,
            distribution_state: crate::provider_service::ServiceDistributionState::Current,
            auth_contract_fingerprint: "sha256:fixture".to_owned(),
            authentication_scope: provider.to_owned(),
            portable_schema: "fixture".to_owned(),
            projection_schema: "fixture".to_owned(),
            account_label: None,
            updated_at_ms: 1,
        }
    }

    #[test]
    fn only_a_card_family_credential_that_is_not_signed_out_blocks_startup() {
        use ServiceAuthenticationState::{Expired, Ready, SignedOut};
        assert!(live(&[]).is_empty());
        assert!(
            live(&[
                status("codex", SignedOut),
                status("claude-code", SignedOut),
                status("antigravity", Ready),
                status("future-provider", Ready),
            ])
            .is_empty()
        );
        assert_eq!(
            live(&[status("codex", Ready), status("claude-code", Expired)]),
            ["codex", "claude-code"]
        );
    }

    #[test]
    fn a_default_session_is_retired_only_where_its_card_entry_was_wiped() {
        use ServiceAuthenticationState::{Ready, SignedOut};
        assert!(retired_entry("codex", Some(&status("codex", SignedOut))));
        assert!(!retired_entry("codex", Some(&status("codex", Ready))));
        // Never had a card credential: the ordinary failure stays.
        assert!(!retired_entry("codex", None));
        assert!(!retired_entry(
            "antigravity",
            Some(&status("antigravity", SignedOut))
        ));
    }

    #[test]
    fn only_a_default_session_of_a_wiped_card_is_parked_for_good() {
        use ServiceAuthenticationState::{Ready, SignedOut};
        let default = crate::provider_account::ProviderAccountId::default();
        let named: crate::provider_account::ProviderAccountId =
            uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let wiped = status("codex", SignedOut);
        assert!(retired_session("codex", &default, Some(&wiped)));
        assert!(!retired_session("codex", &named, Some(&wiped)));
        assert!(!retired_session("codex", &default, Some(&status("codex", Ready))));
        assert!(!retired_session("codex", &default, None));
    }

    #[test]
    fn startup_refuses_a_live_card_credential_until_it_is_signed_out() {
        use base64::Engine as _;
        let root = tempfile::tempdir().unwrap();
        admit_startup(&ProviderAuthService::open(root.path()).unwrap()).unwrap();
        let auth = ProviderAuthService::open(root.path()).unwrap();
        let source: cowboy_provider_sdk::StandardProviderSource =
            serde_json::from_str(include_str!("../../../../../plugins/grok/provider.json"))
                .unwrap();
        let package = cowboy_provider_sdk::build_package(source.compile().unwrap()).unwrap();
        let bundle = crate::machine_protocol::PortableCredentialBundle {
            portable_schema: package.manifest.authentication.portable_schema.clone(),
            method_id: "xai-account".into(),
            values: [(
                "auth_json".to_owned(),
                base64::engine::general_purpose::STANDARD.encode(b"{}"),
            )]
            .into_iter()
            .collect(),
        };
        auth.commit(&package, &bundle, None, Some(0)).unwrap();
        let refusal = admit_startup(&auth).unwrap_err().to_string();
        assert!(refusal.contains("(grok)"), "{refusal}");
        auth.logout("grok").unwrap();
        admit_startup(&auth).unwrap();
        // Admitted: no root write can sign a card in again.
        assert_eq!(
            auth.commit(&package, &bundle, None, None)
                .unwrap_err()
                .to_string(),
            crate::provider_service::ROOT_RETIRED
        );
    }
}
