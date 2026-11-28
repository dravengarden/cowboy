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
pub const SESSION_RETIRED: &str = "This session ran on a retired legacy default account; continue it in a new agent on a named account";

/// Refusal of a Service (`default`) sign-in or credential commit.
pub const LOGIN_RETIRED: &str = "named_account_required: the legacy default account is retired; add a named account in Settings → Subscriptions";

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
pub fn open_provider_auth(data_dir: &std::path::Path) -> anyhow::Result<Arc<ProviderAuthService>> {
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
pub fn admit_startup(auth: &ProviderAuthService) -> anyhow::Result<()> {
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

/// Card-family Providers whose latest root wipe some enrolled Machine
/// (`recipients`) has not acknowledged, with those Machines. A distribution
/// the previous Controller recorded as `current` reached every Machine
/// enrolled then (it kept no per-Machine record); a Machine enrolled later
/// never held the root credential.
fn unacknowledged_wipes(
    auth: &ProviderAuthService,
    recipients: &[crate::provider_service::Recipient],
) -> Vec<String> {
    auth.statuses()
        .into_iter()
        .filter(|status| {
            legacy_cards::retired_provider(&status.provider_id)
                && status.distribution_state
                    != crate::provider_service::ServiceDistributionState::Current
        })
        .filter_map(|status| {
            let missing = auth.unacknowledged(&status.provider_id, recipients);
            (!missing.is_empty()).then(|| format!("{}: {}", status.provider_id, missing.join(", ")))
        })
        .collect()
}

/// What admission covers: the Service dataset and every card Provider's
/// root wipe generation.
fn admission(
    auth: &ProviderAuthService,
    dataset: &str,
) -> crate::provider_service::AdmittedRetirement {
    crate::provider_service::AdmittedRetirement {
        dataset: dataset.to_owned(),
        wipes: auth
            .statuses()
            .into_iter()
            .filter(|status| legacy_cards::retired_provider(&status.provider_id))
            .map(|status| (status.provider_id, status.auth_generation))
            .collect(),
    }
}

/// The second startup gate, right after migrations and before the
/// card-row cleanup.
///
/// The first admission of a Service dataset is strict and read-only: every
/// enrolled Machine applied every card Provider's root wipe (so none can
/// still hold, and later rotate, a moved root generation), and no session on
/// a retired card's `default` account is starting, running or busy (its
/// worker may hold a card credential). The previous Controller is the remedy
/// for both refusals. The admission is recorded with its dataset and wipe
/// generations.
///
/// Once this dataset was admitted, startup never refuses on session or wipe
/// state. A refusal cannot stop a Machine's worker or deliver a wipe (only a
/// running Controller delivers one), no previous Controller could act
/// instead since every release since the retirement carries this gate, and
/// refusing kept Hawk's Controller down for 85 minutes on 2026-11-28 over
/// two stale intake registrations. Rejected intake registrations, which
/// never launched, are settled. Other live sessions on retired `default`
/// accounts are logged as an error and left as they are, so a reconnecting
/// worker still settles a restored turn; every prompt to them is refused and
/// none is revived. Such a worker can only survive a rollback to a
/// Controller from before the retirement, which the component activator no
/// longer selects, or a Machine revoked before the admission and enrolled
/// again with its workers intact. Changed wipe generations (a repeated
/// sign-out) are re-admitted once every enrolled Machine acknowledged them;
/// until then the distribution reconciler keeps delivering them.
pub async fn admit_store(
    auth: &ProviderAuthService,
    store: &crate::store::Store,
    dataset: &str,
) -> anyhow::Result<()> {
    let admitted = admission(auth, dataset);
    let recorded = auth.admitted_retirement();
    if recorded
        .as_ref()
        .is_some_and(|recorded| recorded.dataset == admitted.dataset)
    {
        // Registered by the old enrolled-Machine intake path before its
        // account gate rejected them: provably never launched.
        let settled = store.settle_retired_intake_registrations().await?;
        if !settled.is_empty() {
            tracing::warn!(sessions = ?settled, "settled rejected legacy intake registrations");
        }
        let live = store.live_retired_default_sessions().await?;
        if !live.is_empty() {
            tracing::error!(
                sessions = ?live,
                "live sessions on retired legacy default accounts; every prompt to them is refused, stop their workers"
            );
        }
        if recorded.as_ref() != Some(&admitted) {
            refresh_admission(auth, store, &admitted).await;
        }
        return Ok(());
    }
    let sessions = store.live_retired_default_sessions().await?;
    let recipients = store.credential_recipients().await?;
    admit(auth, &admitted, &recipients, &sessions)
}

/// Record changed wipe generations of an admitted dataset once every
/// enrolled Machine acknowledged them. Bookkeeping only: the recorded
/// dataset already admits this Service, so a failure is logged, never
/// fatal, and the next start retries.
async fn refresh_admission(
    auth: &ProviderAuthService,
    store: &crate::store::Store,
    admitted: &crate::provider_service::AdmittedRetirement,
) {
    let recipients = match store.credential_recipients().await {
        Ok(recipients) => recipients,
        Err(error) => {
            tracing::error!(%error, "reading credential recipients to refresh the retirement admission");
            return;
        }
    };
    let missing = unacknowledged_wipes(auth, &recipients);
    if !missing.is_empty() {
        tracing::error!(
            unacknowledged = ?missing,
            "legacy default card wipes are not yet acknowledged by every enrolled Machine; they are delivered as Machines connect"
        );
    } else if let Err(error) = auth.record_admitted_retirement(admitted) {
        tracing::error!(%error, "recording the refreshed retirement admission");
    }
}

/// The strict gate of a dataset's first admission.
fn admit(
    auth: &ProviderAuthService,
    admitted: &crate::provider_service::AdmittedRetirement,
    recipients: &[crate::provider_service::Recipient],
    sessions: &[String],
) -> anyhow::Result<()> {
    refuse_live_sessions(sessions)?;
    let missing = unacknowledged_wipes(auth, recipients);
    anyhow::ensure!(
        missing.is_empty(),
        "legacy default card wipes are not acknowledged by every enrolled Machine ({}): connect those Machines to the previous Controller until the wipes are delivered, or revoke an unreachable Machine, before installing this release",
        missing.join("; ")
    );
    auth.record_admitted_retirement(admitted)
}

/// The store gate's session refusal, shared with the read-only startup
/// admission preflight (`cowboy serve --check-startup-admission`).
pub fn refuse_live_sessions(sessions: &[String]) -> anyhow::Result<()> {
    anyhow::ensure!(
        sessions.is_empty(),
        "sessions on retired legacy default accounts are still live ({}): stop them with the previous Controller before installing this release",
        sessions.join(", ")
    );
    Ok(())
}

/// Whether the `default` account of `provider` is a retired card: the
/// Provider is of a card family and its root entry is not Ready (wiped by an
/// adoption or a sign-out, or never present: a Machine's legacy launch path
/// could otherwise resume it on its own native login).
pub fn retired(state: &AppState, provider: &str) -> bool {
    retired_entry(provider, state.provider_auth.status(provider).as_ref())
}

/// The refusal of any input to `session` (a queued prompt, a direct prompt
/// or steering) when it ran on a retired card's `default` account.
pub fn session_refusal(state: &AppState, session: &str) -> Option<&'static str> {
    session_retired(state, &state.hub.session_info(session)?.meta).then_some(SESSION_RETIRED)
}

/// The managed-call failure of a child session of a retired card: it can
/// never run its prompt, so neither a launch nor a queued round waits for it.
pub fn call_refusal(state: &AppState, child: &str) -> Option<&'static str> {
    session_refusal(state, child).map(|_| "named_account_required")
}

/// Whether `meta` is a session of a retired card's `default` account. Its
/// queue is parked for good: never retried, woken or counted as a waiter.
pub fn session_retired(state: &AppState, meta: &crate::core::SessionMeta) -> bool {
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
            .is_none_or(|status| status.authentication_state != ServiceAuthenticationState::Ready)
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
        // No vault entry at all is no way back either.
        assert!(retired_entry("codex", None));
        assert!(!retired_entry(
            "antigravity",
            Some(&status("antigravity", SignedOut))
        ));
    }

    fn signed_out_grok(root: &std::path::Path) -> ProviderAuthService {
        use base64::Engine as _;
        let auth = ProviderAuthService::open(root).unwrap();
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
        auth.logout("grok").unwrap();
        auth
    }

    fn recipients() -> [crate::provider_service::Recipient; 2] {
        [("hawk".into(), "k1".into()), ("ovh".into(), "k2".into())]
    }

    #[test]
    fn the_delivery_gate_refuses_an_unacknowledged_machine_or_a_live_session() {
        let root = tempfile::tempdir().unwrap();
        let auth = signed_out_grok(root.path());
        let wipe = auth.status("grok").unwrap().auth_generation;
        let admitted = admission(&auth, "dataset");
        auth.record_delivery("grok", ("hawk", "k1"), wipe).unwrap();
        let refusal = admit(&auth, &admitted, &recipients(), &[])
            .unwrap_err()
            .to_string();
        assert!(refusal.contains("grok: ovh"), "{refusal}");
        assert!(auth.admitted_retirement().is_none());
        // A live session on a retired default is refused too, by name.
        let refusal = admit(&auth, &admitted, &recipients()[..1], &["sess-1".into()])
            .unwrap_err()
            .to_string();
        assert!(refusal.contains("sess-1"), "{refusal}");
        // Revoking the unreachable Machine (it leaves the recipients) passes.
        admit(&auth, &admitted, &recipients()[..1], &[]).unwrap();
        assert_eq!(auth.admitted_retirement().as_ref(), Some(&admitted));
        // Once admitted, `admit_store` fences live sessions instead
        // (`retirement_fences_every_live_retired_default_session`).
    }

    #[tokio::test]
    async fn an_admitted_dataset_starts_despite_live_retired_sessions() {
        let root = tempfile::tempdir().unwrap();
        let auth = signed_out_grok(root.path());
        let store = crate::store::Store::connect("sqlite::memory:", root.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let mut live = crate::store::fixtures::session("sess-live");
        live.provider = "grok".into();
        store.insert_session(&live).await.unwrap();
        // Not admitted yet: the previous Controller is the remedy.
        let refusal = admit_store(&auth, &store, "dataset").await.unwrap_err();
        assert!(format!("{refusal:#}").contains("sess-live"), "{refusal:#}");
        assert!(auth.admitted_retirement().is_none());
        // Admitted (no enrolled Machine to acknowledge the wipe).
        admit(&auth, &admission(&auth, "dataset"), &[], &[]).unwrap();
        // The stale live row no longer keeps the Service down. It is left as
        // it is, so a reconnecting worker still settles a restored turn.
        admit_store(&auth, &store, "dataset").await.unwrap();
        let live = store.live_retired_default_sessions().await.unwrap();
        assert_eq!(live, ["sess-live"]);
        // Another dataset is not covered by that admission.
        assert!(admit_store(&auth, &store, "copied").await.is_err());
        // A changed wipe generation of the admitted dataset starts too, and
        // is re-admitted once acknowledged (no enrolled Machine here).
        auth.logout("grok").unwrap();
        let before = auth.admitted_retirement().unwrap();
        admit_store(&auth, &store, "dataset").await.unwrap();
        let after = auth.admitted_retirement().unwrap();
        assert_ne!(after, before);
        assert_eq!(after, admission(&auth, "dataset"));
    }

    #[test]
    fn the_delivery_gate_passes_when_every_machine_acknowledged_the_wipe() {
        let root = tempfile::tempdir().unwrap();
        let auth = signed_out_grok(root.path());
        let wipe = auth.status("grok").unwrap().auth_generation;
        for (machine, enrollment) in recipients() {
            auth.record_delivery("grok", (&machine, &enrollment), wipe)
                .unwrap();
        }
        admit(&auth, &admission(&auth, "dataset"), &recipients(), &[]).unwrap();
        // A re-enrolled Machine id does not inherit its acknowledgement.
        let reenrolled = [("ovh".to_owned(), "k3".to_owned())];
        assert!(admit(&auth, &admission(&auth, "dataset"), &reenrolled, &[]).is_err());
        // The previous Controller's aggregate `current` also counts.
        let other = tempfile::tempdir().unwrap();
        let auth = signed_out_grok(other.path());
        auth.mark_distribution(
            "grok",
            wipe,
            crate::provider_service::ServiceDistributionState::Current,
        )
        .unwrap();
        admit(&auth, &admission(&auth, "dataset"), &recipients(), &[]).unwrap();
    }

    #[test]
    fn an_admission_covers_only_its_dataset_and_wipe_generations() {
        let root = tempfile::tempdir().unwrap();
        let auth = signed_out_grok(root.path());
        let admitted = admission(&auth, "dataset");
        assert_eq!(
            admitted.wipes["grok"],
            auth.status("grok").unwrap().auth_generation
        );
        auth.record_admitted_retirement(&admitted).unwrap();
        assert_eq!(
            auth.admitted_retirement().as_ref(),
            Some(&admission(&auth, "dataset"))
        );
        // Another dataset, or a newer wipe (a rollback signed in and out
        // again), is not admitted.
        assert_ne!(
            auth.admitted_retirement().as_ref(),
            Some(&admission(&auth, "other"))
        );
        auth.logout("grok").unwrap();
        assert_ne!(
            auth.admitted_retirement().as_ref(),
            Some(&admission(&auth, "dataset"))
        );
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
        assert!(!retired_session(
            "codex",
            &default,
            Some(&status("codex", Ready))
        ));
        assert!(retired_session("codex", &default, None));
        // A Provider without a card keeps its default.
        assert!(!retired_session("portable-fixture", &default, None));
    }

    #[test]
    fn no_session_is_created_on_a_card_default_even_without_a_root_entry() {
        for provider in ["codex", "claude-code", "grok", "codex-deepseek"] {
            assert_eq!(
                super::super::creatable_default(provider),
                Err("named_account_required")
            );
        }
        assert_eq!(super::super::creatable_default("portable-fixture"), Ok(()));
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
