//! Shared account metadata writer for HTTP, CLI and MCP. Never accepts secrets.
use super::*;
use crate::store::provider_accounts::{Mutation, MutationResult};
use serde_json::{Value, json};

pub(super) mod lifecycle;
pub(super) mod login;
pub(super) mod native_generation;
pub(super) mod registration;
mod revocation;
pub(super) mod transitions;
pub(in crate::server) use revocation::reconcile;
pub(in crate::server) use revocation::run_recovery;

pub(super) fn validate_native_evidence(
    provider: &str,
    recipe: Option<&cowboy_provider_sdk::NativeIdentityRecipe>,
    identity: Option<&crate::provider_account::NativeIdentity>,
) -> Result<(), &'static str> {
    match (recipe, identity) {
        (None, None) => Ok(()),
        (
            Some(cowboy_provider_sdk::NativeIdentityRecipe::FreshLoginJsonV1 {
                authority,
                tenant_path,
                ..
            }),
            Some(identity),
        ) => {
            if &identity.authority != authority
                || identity.tenant.is_some() != tenant_path.is_some()
            {
                return Err("native_identity_contract_mismatch");
            }
            identity.fingerprint(provider).map(|_| ())
        }
        _ => Err("native_identity_evidence_required"),
    }
}

pub(super) fn receipt_value(receipt: &crate::store::provider_accounts::Receipt) -> Value {
    if let Some(plan) = &receipt.revocation {
        return json!({"status":if receipt.revocation_completed { "completed" } else { "revoking" },"revision":receipt.registry.revision,"account_id":receipt.account_id,
            "planned":{"logins":plan.logins.len(),"sessions":plan.sessions.len(),"grants":plan.grants.len()},
            "completion_state":if receipt.revocation_completed { "completed" } else { "unconfirmed" }});
    }
    json!({"status":"committed","revision":receipt.registry.revision,"account_id":receipt.account_id})
}

pub(super) async fn resolve_for_create(
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
            return Ok(Default::default());
        }
        None => return Err("unavailable"),
    };
    let id = registry.resolve(provider, explicit)?;
    if !id.is_default() {
        allow_named_creation(
            provider,
            state
                .service_config
                .get(&crate::config::schema::ACCOUNT_NATIVE_CLAUDE_SESSIONS),
        )?;
    }
    Ok(id)
}

pub(super) async fn read_value(state: &AppState, owner: &str) -> Result<Value, &'static str> {
    let registry = state
        .store
        .as_ref()
        .ok_or("unavailable")?
        .provider_accounts(owner)
        .await
        .map_err(|_| "unavailable")?;
    let enabled = state
        .service_config
        .get(&crate::config::schema::ACCOUNT_NATIVE_CLAUDE_SESSIONS);
    Ok(
        json!({"registry":registry,"session_admission":if enabled {"enabled"} else {"not_enabled"},
        "session_providers":if enabled {vec!["claude-code"]} else {vec![]}}),
    )
}

fn allow_named_creation(provider: &str, enabled: bool) -> Result<(), &'static str> {
    if !enabled {
        return Err("named_account_runtime_unavailable");
    }
    if provider != "claude-code" {
        return Err("named_account_provider_unavailable");
    }
    Ok(())
}

pub(super) async fn update_value(
    state: &AppState,
    owner: &str,
    operation: &str,
    input: &Mutation,
) -> Result<Value, &'static str> {
    let store = state.store.as_ref().ok_or("unavailable")?;
    // Metadata writes share one authority and receipt; no native login, usage
    // probe, credential access or worker activation happens here.
    match store
        .mutate_provider_accounts(owner, operation, input)
        .await
        .map_err(|_| "outcome_unknown")?
    {
        MutationResult::Applied(receipt) => {
            state
                .hub
                .announce_preferences(owner, "provider_accounts", receipt.registry.revision);
            Ok(receipt_value(&receipt))
        }
        MutationResult::Conflict(current) => {
            Ok(json!({"error":{"code":"revision_conflict"},"current":current}))
        }
        MutationResult::Invalid(code) => Err(code),
    }
}

pub(super) async fn read(
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
pub(super) struct Update {
    operation: String,
    dataset: String,
    #[serde(flatten)]
    mutation: Mutation,
}

pub(super) async fn update(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Json(input): Json<Update>,
) -> Response {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollout_cannot_enable_unsupported_provider_or_fall_back_to_default() {
        assert!(allow_named_creation("claude-code", true).is_ok());
        for provider in ["claude-code", "codex", "claude-deepseek", "grok", ""] {
            assert_eq!(
                allow_named_creation(provider, false),
                Err("named_account_runtime_unavailable")
            );
            if provider != "claude-code" {
                assert_eq!(
                    allow_named_creation(provider, true),
                    Err("named_account_provider_unavailable")
                );
            }
        }
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
            }),
            remote_wipes: Default::default(),
            local_wipes: Default::default(),
            login_stops: Default::default(),
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
            json!({"logins":0,"sessions":0,"grants":0})
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
        };
        let identity = crate::provider_account::NativeIdentity {
            authority: "native".into(),
            subject: "subject".into(),
            tenant: Some("tenant".into()),
        };
        assert!(validate_native_evidence("provider", None, None).is_ok());
        assert!(validate_native_evidence("provider", Some(&recipe), Some(&identity)).is_ok());
        assert!(validate_native_evidence("provider", None, Some(&identity)).is_err());
        assert!(validate_native_evidence("provider", Some(&recipe), None).is_err());
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
            assert!(validate_native_evidence("provider", Some(&recipe), Some(&invalid)).is_err());
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
