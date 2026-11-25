//! Shared account metadata writer for HTTP, CLI and MCP. Never accepts secrets.
use super::*;
use crate::store::provider_accounts::{Mutation, MutationResult};
use serde_json::{Value, json};

pub mod browser;
pub mod controls;
pub mod lifecycle;
pub mod login;
pub mod native_generation;
pub mod portable;
pub mod registration;
pub mod retirement;
mod revocation;
pub mod transitions;
pub mod usage;
pub use revocation::reconcile;
pub use revocation::run_recovery;

pub fn validate_native_evidence(
    provider: &str,
    account: &crate::provider_account::ProviderAccountId,
    recipe: Option<&cowboy_provider_sdk::NativeIdentityRecipe>,
    identity: Option<&crate::provider_account::NativeIdentity>,
) -> Result<(), &'static str> {
    match (recipe, identity) {
        (None, None) => Ok(()),
        (Some(recipe), Some(identity)) => {
            if identity.authority != recipe.authority()
                || identity.tenant.is_some() != recipe.has_tenant()
            {
                return Err("native_identity_contract_mismatch");
            }
            // An owner-declared subject is the account card itself; any other
            // value would let one card's login bind another card.
            if recipe.is_owner_declared()
                && (account.is_default() || identity.subject != account.as_str())
            {
                return Err("native_identity_contract_mismatch");
            }
            identity.fingerprint(provider).map(|_| ())
        }
        _ => Err("native_identity_evidence_required"),
    }
}

pub fn receipt_value(receipt: &crate::store::provider_accounts::Receipt) -> Value {
    if let Some(plan) = &receipt.revocation {
        return json!({"status":if receipt.revocation_completed { "completed" } else { "revoking" },"revision":receipt.registry.revision,"account_id":receipt.account_id,
            "planned":{"logins":plan.logins.len(),"sessions":plan.sessions.len(),"grants":plan.grants.len(),"browsers":plan.browsers.len()},
            "completion_state":if receipt.revocation_completed { "completed" } else { "unconfirmed" }});
    }
    let mut value = json!({"status":"committed","revision":receipt.registry.revision,"account_id":receipt.account_id});
    if let Some(share) = &receipt.browser_share_id {
        value["share_id"] = json!(share);
    }
    value
}

pub async fn resolve_for_create(
    state: &AppState,
    owner: &str,
    provider: &str,
    explicit: Option<&crate::provider_account::ProviderAccountId>,
) -> Result<crate::provider_account::ProviderAccountId, &'static str> {
    let registry = match state.store.as_ref() {
        Some(store) => store
            .provider_accounts(owner)
            .await
            .map_err(|_| "unavailable")?,
        None if explicit.is_none_or(crate::provider_account::ProviderAccountId::is_default) => {
            return creatable_default(provider).map(|()| Default::default());
        }
        None => return Err("unavailable"),
    };
    // A portable account serves every portable Provider of its credential
    // scope; other accounts serve exactly their own Provider.
    let scope = state.provider_catalog.portable_account_scope(provider);
    let id = registry.resolve_in_scope(provider, &scope, explicit)?;
    // Any number of sessions may run one account concurrently (design
    // "Revision 6"); the daemon's per-account bound is the only cap.
    named_only_admitted(state, provider, &id).await?;
    if id.is_default() {
        creatable_default(provider)?;
    } else {
        allow_named_creation(
            named_session_provider(state, provider),
            state
                .service_config
                .get(&crate::config::schema::ACCOUNT_NATIVE_CLAUDE_SESSIONS),
        )?;
    }
    Ok(id)
}

/// No session is created on a retired card's `default` account, whether or
/// not its root entry exists (only historical sessions keep that exception).
pub fn creatable_default(provider: &str) -> Result<(), &'static str> {
    if crate::store::provider_accounts::legacy_cards::retired_provider(provider) {
        return Err("named_account_required");
    }
    Ok(())
}

/// A named-only Provider (Antigravity) has no default account, and admits
/// sign-ins and sessions only for accounts the Service admits
/// (`accounts.antigravity_admitted`) and the owner has not blocked.
pub async fn named_only_admitted(
    state: &AppState,
    provider: &str,
    account: &crate::provider_account::ProviderAccountId,
) -> Result<(), &'static str> {
    if !crate::provider_account::named_only(provider) {
        return Ok(());
    }
    if account.is_default() {
        return Err("named_account_required");
    }
    if !controls::admitted(state, account) {
        return Err("antigravity_account_not_admitted");
    }
    if controls::blocked(state, account).await? {
        return Err("antigravity_account_blocked");
    }
    Ok(())
}

/// Providers whose named accounts have a credential path: a signed native
/// login, or a Service-held portable key at account scope.
pub fn session_providers(state: &AppState) -> BTreeSet<String> {
    let mut providers = state.provider_catalog.native_account_providers();
    providers.extend(state.provider_catalog.portable_account_providers());
    providers
}

pub fn named_session_provider(state: &AppState, provider: &str) -> bool {
    session_providers(state).contains(provider)
}

/// Public key state of each available portable account, never the key.
fn portable_credentials(
    state: &AppState,
    registry: &crate::store::provider_accounts::Registry,
) -> Vec<Value> {
    registry
        .accounts
        .iter()
        .filter_map(|account| {
            let status = portable::scope(state, &account.id, &account.provider_id)?
                .status(&account.provider_id)?;
            Some(json!({"account_id":account.id,"provider_id":account.provider_id,
                "authentication_state":status.authentication_state,"auth_generation":status.auth_generation,
                "distribution_state":status.distribution_state,"updated_at_ms":status.updated_at_ms}))
        })
        .collect()
}

pub async fn read_value(state: &Arc<AppState>, owner: &str) -> Result<Value, &'static str> {
    let store = state.store.as_ref().ok_or("unavailable")?;
    let registry = store
        .provider_accounts(owner)
        .await
        .map_err(|_| "unavailable")?;
    // Read-only compatibility projection: a Web client from before the
    // retirement compares this revision too and would discard every later
    // snapshot without it. Nothing writes the `subscriptions` preference now.
    let display = store
        .retired_subscriptions_preferences(owner)
        .await
        .map_err(|_| "unavailable")?;
    let enabled = state
        .service_config
        .get(&crate::config::schema::ACCOUNT_NATIVE_CLAUDE_SESSIONS);
    let revoked = store
        .completed_provider_account_revocations(owner)
        .await
        .map_err(|_| "unavailable")?;
    let (shares, families) = browser::shares::list_value(state, owner).await?;
    Ok(
        json!({"registry":registry,"display_preferences":display,"revoked_accounts":revoked,"session_admission":if enabled {"enabled"} else {"not_enabled"},
        "session_providers":if enabled {session_providers(state).into_iter().collect()} else {Vec::<String>::new()},
        // Accounts of these Providers take an API key instead of a sign-in;
        // each inner list is one credential scope an account serves whole.
        "portable_providers":state.provider_catalog.portable_account_providers(),
        // Subscriptions sign in once on any Machine; the account vault serves every Machine.
        "subscription_providers":state.provider_catalog.subscription_account_providers(),
        "portable_scopes":portable_scopes(state),"portable_credentials":portable_credentials(state, &registry),
        // The Service gate of the optional Machine browser (`accounts.machine_browser`).
        "machine_browser":if browser::gate_open(state) {"enabled"} else {"not_enabled"},
        // Owner-defined shared browser profiles and the family of each
        // Provider (a share holds at most one member per family).
        "browser_shares":shares,"browser_share_families":families,
        // Named-only (Antigravity) accounts: admission, durable state and
        // reason, the active turn, Machine fences, uncertain effects and the
        // kill switch's logout. Their Providers have no default account.
        "named_only_providers":crate::provider_account::NAMED_ONLY_PROVIDERS,
        // The retired legacy default cards: these Providers have no Service
        // (`default`) account either (`docs/legacy-card-retirement.md`).
        "retired_default_providers":crate::store::provider_accounts::legacy_cards::retired_providers(),
        "account_controls":controls::read_value(state, &registry)}),
    )
}

fn portable_scopes(state: &AppState) -> BTreeSet<Vec<String>> {
    let catalog = &state.provider_catalog;
    catalog
        .portable_account_providers()
        .into_iter()
        .chain(catalog.subscription_account_providers())
        .collect::<BTreeSet<_>>()
        .iter()
        .map(|provider| state.provider_catalog.portable_account_scope(provider))
        .collect()
}

/// `supported` means the Provider has a named credential path (native login
/// or portable key); named accounts of other Providers have none and never
/// fall back to the default.
fn allow_named_creation(supported: bool, enabled: bool) -> Result<(), &'static str> {
    if !enabled {
        return Err("named_account_runtime_unavailable");
    }
    if !supported {
        return Err("named_account_provider_unavailable");
    }
    Ok(())
}

pub async fn update_value(
    state: &Arc<AppState>,
    owner: &str,
    operation: &str,
    input: &Mutation,
) -> Result<Value, &'static str> {
    let store = state.store.as_ref().ok_or("unavailable")?;
    // `set_default` to `default` of a retired card's Provider clears the
    // named selection (so its last named default can be disabled); creation
    // then requires an explicit named account (`creatable_default`).
    // Metadata writes share one authority and receipt; no native login, usage
    // probe, credential access or worker activation happens here. Share
    // changes use the same writer, with families from the static table. Membership
    // changes are fenced against share resets, and a replayed receipt
    // (the operation already committed) reruns no browser effect.
    let fence = state.browsers.share_lock(owner);
    let _fence = fence.lock().await;
    let replay = store
        .provider_account_receipt(owner, operation)
        .await
        .map_err(|_| "unavailable")?
        .is_some();
    // A no-op share change (the member already in exactly that share)
    // commits its receipt but has no browser effect.
    let mut deferred_share_leave = false;
    let (result, noop) = if input.change.is_browser_share() {
        browser::shares::update(state, owner, operation, input).await?
    } else if let crate::store::provider_accounts::Change::BeginRevocation { id } = &input.change {
        deferred_share_leave =
            browser::shares::member_has_pending_navigation(state, owner, id).await?;
        (
            if deferred_share_leave {
                store
                    .mutate_provider_accounts_deferring_share_leave(owner, operation, input)
                    .await
            } else {
                store
                    .mutate_provider_accounts(owner, operation, input)
                    .await
            }
            .map_err(|_| "outcome_unknown")?,
            false,
        )
    } else {
        (
            store
                .mutate_provider_accounts(owner, operation, input)
                .await
                .map_err(|_| "outcome_unknown")?,
            false,
        )
    };
    match result {
        MutationResult::Applied(receipt) => {
            state
                .hub
                .announce_preferences(owner, "provider_accounts", receipt.registry.revision);
            // Revocation ends the member's viewers at once; its own
            // reconciliation revokes every captured placement of its own key.
            // Disabling the account or its browser also hibernates its own
            // instance (the snapshot is kept). Neither touches a share's
            // browser; only deleting the share revokes it.
            browser::shares::run_effects(
                state,
                owner,
                &input.change,
                browser::shares::fresh_effects(owner, &input.change, replay || noop),
            );
            if deferred_share_leave {
                state.browsers.navigation_settled_notify().notify_one();
            }
            Ok(receipt_value(&receipt))
        }
        MutationResult::Conflict(current) => {
            Ok(json!({"error":{"code":"revision_conflict"},"current":current}))
        }
        MutationResult::Invalid(code) => Err(code),
    }
}

pub async fn read(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
) -> Response {
    match read_value(&state, &auth.principal.user_id).await {
        Ok(mut value) => {
            value["dataset"] =
                serde_json::to_value(sync_dataset::descriptor(&state.service_id, &auth.principal))
                    .unwrap();
            ([(header::CACHE_CONTROL, "no-store")], Json(value)).into_response()
        }
        Err(code) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error":{"code":code}})),
        )
            .into_response(),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Update {
    operation: String,
    dataset: String,
    #[serde(flatten)]
    mutation: Mutation,
}

/// A pending change only a Web release from before the legacy card
/// retirement sends, typed exactly as that release's writer accepted it (a
/// card browser switch or a card share membership), so its request hash is
/// the one that writer recorded.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LegacyUpdate {
    operation: String,
    dataset: String,
    expected_revision: i64,
    change: LegacyChange,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum LegacyChange {
    SetLegacyBrowserEnabled {
        card: String,
        enabled: bool,
    },
    SetBrowserShare {
        member: LegacyCardMember,
        share_id: Option<String>,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LegacyCardMember {
    card: String,
}

impl LegacyUpdate {
    /// The request hash the previous writer recorded: its `Mutation`.
    fn request_hash(&self) -> String {
        #[derive(Serialize)]
        struct Mutation<'a> {
            expected_revision: i64,
            change: &'a LegacyChange,
        }
        let bytes = serde_json::to_vec(&Mutation {
            expected_revision: self.expected_revision,
            change: &self.change,
        })
        .expect("a legacy mutation serializes");
        crate::admin::hex_sha256(&bytes)
    }
}

/// A change only a Web release from before the legacy card retirement sends:
/// a card browser switch or a card share membership.
fn retired_change(input: &Value) -> bool {
    let change = &input["change"];
    change["type"] == "set_legacy_browser_enabled"
        || (change["type"] == "set_browser_share" && change["member"].get("card").is_some())
}

pub async fn update(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Json(input): Json<Value>,
) -> Response {
    let input = match serde_json::from_value::<Update>(input.clone()) {
        Ok(update) => update,
        Err(error) if !retired_change(&input) => {
            return (StatusCode::UNPROCESSABLE_ENTITY, error.to_string()).into_response();
        }
        Err(_) => return retired_update(&state, &auth.principal, input).await,
    };
    if !sync_dataset::matches(&state.service_id, &auth.principal, &input.dataset) {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error":{"code":"dataset_changed"}})),
        )
            .into_response();
    }
    match update_value(
        &state,
        &auth.principal.user_id,
        &input.operation,
        &input.mutation,
    )
    .await
    {
        Ok(value) => (
            if value.get("error").is_some() {
                StatusCode::CONFLICT
            } else {
                StatusCode::OK
            },
            Json(value),
        )
            .into_response(),
        Err(code) => (
            if matches!(code, "unavailable" | "outcome_unknown") {
                StatusCode::SERVICE_UNAVAILABLE
            } else {
                StatusCode::UNPROCESSABLE_ENTITY
            },
            Json(json!({"error":{"code":code}})),
        )
            .into_response(),
    }
}

/// An older Web client's pending retired change: the receipt of exactly
/// that operation and request if the previous Controller committed it, else
/// a definitive JSON rejection the client recognizes, so it releases its
/// journal. Its dataset must be the caller's.
async fn retired_update(state: &AppState, principal: &ProductPrincipal, input: Value) -> Response {
    let reject = |status: StatusCode, code: &str| {
        (status, Json(json!({"error":{"code":code}}))).into_response()
    };
    let Ok(legacy) = serde_json::from_value::<LegacyUpdate>(input) else {
        return reject(StatusCode::UNPROCESSABLE_ENTITY, "invalid_argument");
    };
    if !sync_dataset::matches(&state.service_id, principal, &legacy.dataset) {
        return reject(StatusCode::CONFLICT, "dataset_changed");
    }
    let Some(store) = state.store.as_ref() else {
        return reject(StatusCode::SERVICE_UNAVAILABLE, "unavailable");
    };
    match store
        .provider_account_operation(&principal.user_id, &legacy.operation)
        .await
    {
        Ok(Some((hash, receipt))) if hash == legacy.request_hash() => {
            Json(receipt_value(&receipt)).into_response()
        }
        // Never committed, or the id names another request.
        Ok(_) => reject(StatusCode::UNPROCESSABLE_ENTITY, "invalid_argument"),
        Err(_) => reject(StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_legacy_change_hashes_as_the_previous_writer_recorded_it() {
        // The previous writer hashed its Mutation {expected_revision, change}.
        let legacy: LegacyUpdate = serde_json::from_value(json!({"operation":"op","dataset":"d",
            "expected_revision":3,"change":{"type":"set_browser_share","member":{"card":"openai"},"share_id":null}}))
            .unwrap();
        let recorded = br#"{"expected_revision":3,"change":{"type":"set_browser_share","member":{"card":"openai"},"share_id":null}}"#;
        assert_eq!(legacy.request_hash(), crate::admin::hex_sha256(recorded));
        let legacy: LegacyUpdate = serde_json::from_value(json!({"operation":"op","dataset":"d",
            "expected_revision":0,"change":{"type":"set_legacy_browser_enabled","card":"anthropic","enabled":true}}))
            .unwrap();
        let recorded = br#"{"expected_revision":0,"change":{"type":"set_legacy_browser_enabled","card":"anthropic","enabled":true}}"#;
        assert_eq!(legacy.request_hash(), crate::admin::hex_sha256(recorded));
        // Anything else is no legacy request.
        assert!(serde_json::from_value::<LegacyUpdate>(json!({"operation":"op","dataset":"d",
            "expected_revision":0,"change":{"type":"set_legacy_browser_enabled","card":"anthropic","enabled":true,"extra":1}})).is_err());
    }

    #[test]
    fn only_the_retired_card_changes_are_answered_for_older_clients() {
        assert!(retired_change(
            &json!({"change":{"type":"set_legacy_browser_enabled","card":"anthropic","enabled":true}})
        ));
        assert!(retired_change(
            &json!({"change":{"type":"set_browser_share","member":{"card":"openai"},"share_id":null}})
        ));
        assert!(!retired_change(
            &json!({"change":{"type":"set_browser_share","member":{"account_id":"a"},"share_id":null}})
        ));
        assert!(!retired_change(
            &json!({"change":{"type":"rename","id":"a","label":"x"}})
        ));
    }

    #[test]
    fn rollout_cannot_enable_unsupported_provider_or_fall_back_to_default() {
        assert!(allow_named_creation(true, true).is_ok());
        for native in [true, false] {
            assert_eq!(
                allow_named_creation(native, false),
                Err("named_account_runtime_unavailable")
            );
        }
        assert_eq!(
            allow_named_creation(false, true),
            Err("named_account_provider_unavailable")
        );
    }

    #[test]
    fn revocation_receipt_projects_persisted_completion_only() {
        use crate::store::provider_accounts::{Receipt, Registry, revocation::Plan};
        let mut receipt = Receipt {
            registry: Registry::default(),
            account_id: uuid::Uuid::new_v4().to_string().try_into().unwrap(),
            revocation_completed: false,
            revocation: Some(Plan {
                sessions: vec![],
                logins: vec![],
                grants: vec![],
                browsers: vec![],
                portable_credentials: false,
            }),
            remote_wipes: Default::default(),
            local_wipes: Default::default(),
            login_stops: Default::default(),
            browser_revocations: Default::default(),
            browser_share_id: None,
            ..Default::default()
        };
        // An empty plan or a disabled account alone does not imply completion.
        assert_eq!(receipt_value(&receipt)["status"], "revoking");
        assert_eq!(receipt_value(&receipt)["completion_state"], "unconfirmed");
        assert!(receipt_value(&receipt).get("completion").is_none());
        receipt.revocation_completed = true;
        let restored: Receipt =
            serde_json::from_slice(&serde_json::to_vec(&receipt).unwrap()).unwrap();
        assert_eq!(receipt_value(&restored)["status"], "completed");
        assert_eq!(receipt_value(&restored)["completion_state"], "completed");
        assert_eq!(
            receipt_value(&restored)["planned"],
            json!({"logins":0,"sessions":0,"grants":0,"browsers":0})
        );
        let mut legacy = serde_json::to_value(&receipt).unwrap();
        legacy
            .as_object_mut()
            .unwrap()
            .remove("revocation_completed");
        assert_eq!(
            receipt_value(&serde_json::from_value(legacy).unwrap())["status"],
            "revoking"
        );
    }

    #[test]
    fn native_evidence_must_match_the_signed_recipe() {
        let recipe = cowboy_provider_sdk::NativeIdentityRecipe::FreshLoginJsonV1 {
            authority: "native".into(),
            relative_path: "profile.json".into(),
            subject_path: vec!["id".into()],
            tenant_path: Some(vec!["tenant".into()]),
            subject_claim: None,
        };
        let identity = crate::provider_account::NativeIdentity {
            authority: "native".into(),
            subject: "subject".into(),
            tenant: Some("tenant".into()),
        };
        assert!(
            validate_native_evidence(
                "provider",
                &crate::provider_account::ProviderAccountId::default(),
                None,
                None
            )
            .is_ok()
        );
        assert!(
            validate_native_evidence(
                "provider",
                &crate::provider_account::ProviderAccountId::default(),
                Some(&recipe),
                Some(&identity)
            )
            .is_ok()
        );
        assert!(
            validate_native_evidence(
                "provider",
                &crate::provider_account::ProviderAccountId::default(),
                None,
                Some(&identity)
            )
            .is_err()
        );
        assert!(
            validate_native_evidence(
                "provider",
                &crate::provider_account::ProviderAccountId::default(),
                Some(&recipe),
                None
            )
            .is_err()
        );
        for invalid in [
            crate::provider_account::NativeIdentity {
                authority: "other".into(),
                ..identity.clone()
            },
            crate::provider_account::NativeIdentity {
                tenant: None,
                ..identity.clone()
            },
            crate::provider_account::NativeIdentity {
                subject: "".into(),
                ..identity.clone()
            },
        ] {
            assert!(
                validate_native_evidence(
                    "provider",
                    &crate::provider_account::ProviderAccountId::default(),
                    Some(&recipe),
                    Some(&invalid)
                )
                .is_err()
            );
        }
    }

    #[test]
    fn http_account_edits_never_accept_authority_or_credentials() {
        let value = json!({"operation":"one","dataset":"observed","expected_revision":0,
            "change":{"type":"create","provider_id":"claude-code","label":"Personal Max"}});
        assert!(serde_json::from_value::<Update>(value.clone()).is_ok());
        for field in ["owner", "machine_id", "token", "password"] {
            let mut extra = value.clone();
            extra[field] = json!("injected");
            assert!(serde_json::from_value::<Update>(extra).is_err());
        }
        let mut absent_dataset = value;
        absent_dataset.as_object_mut().unwrap().remove("dataset");
        assert!(serde_json::from_value::<Update>(absent_dataset).is_err());
    }
}
