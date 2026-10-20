//! Local readiness only: no vault creation, native process or vendor request.
use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct QueryOptions {
    machine_id: String,
}

pub(in crate::server) async fn options(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(account): Path<ProviderAccountId>,
    Query(query): Query<QueryOptions>,
) -> Response {
    match options_value(&state, &auth.principal.user_id, &account, &query.machine_id).await {
        Ok(value) => ([(header::CACHE_CONTROL, "no-store")], Json(value)).into_response(),
        Err(code) => (
            if code == "account_unavailable" {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            },
            Json(json!({"error":{"code":code}})),
        )
            .into_response(),
    }
}

pub(in crate::server) async fn options_value(
    state: &AppState,
    owner: &str,
    account_id: &ProviderAccountId,
    machine_id: &str,
) -> Result<Value, &'static str> {
    let store = state.store.as_ref().ok_or("unavailable")?;
    let registry = store
        .provider_accounts(owner)
        .await
        .map_err(|_| "unavailable")?;
    // Resolve ownership before looking up any Machine or signed package.
    let account = registry
        .accounts
        .iter()
        .find(|account| &account.id == account_id)
        .ok_or("account_unavailable")?;
    let machines = store.list_machines().await.map_err(|_| "unavailable")?;
    let machine = machines
        .iter()
        .find(|machine| machine.id == machine_id && !machine.revoked);
    let installed = active_machine_plugin(&machines, machine_id, &account.provider_id).ok();
    let package = installed.as_ref().and_then(|installed| {
        state.provider_catalog.package(
            &account.provider_id,
            &installed.plugin_version,
            &installed.generation_digest,
        )
    });
    let protocol = state
        .machine_control
        .connection_protocol(machine_id)
        .map(|(_, protocol)| protocol);
    let recipient = if machine.is_some() {
        store
            .machine_encryption_public_key(machine_id)
            .await
            .map_err(|_| "unavailable")?
            .is_some()
    } else {
        false
    };
    let reason = public_login_allowed(
        &account.provider_id,
        state
            .service_config
            .get(&crate::config::schema::ACCOUNT_NATIVE_CLAUDE_LOGIN),
    )
    .err()
    .or_else(|| {
        readiness_reason(
            account.available(),
            state
                .supervisor
                .runtime_allowed(&account.provider_id, machine_id),
            machine.is_some(),
            protocol,
            installed.is_some(),
            package.is_some(),
            recipient,
        )
    });
    let methods: Vec<_> = package
        .iter()
        .flat_map(|package| &package.manifest.authentication.methods)
        .map(|method| {
            let reason = reason.or_else(|| {
                method
                    .native_identity
                    .is_none()
                    .then_some("native_identity_verification_unavailable")
            });
            json!({"id":method.id,"label":method.label,"available":reason.is_none(),"reason":reason})
        })
        .collect();
    let available = methods.iter().any(|method| method["available"] == true);
    // Metadata only: do not probe the Provider or turn discovery into a lease.
    let native_profile = match store
        .native_login_for_session(owner, account_id, machine_id)
        .await
        .map_err(|_| "unavailable")?
    {
        Some((operation, completion))
            if store
                .with_native_login_binding(owner, &operation, &completion, || Ok(()))
                .await
                .is_ok()
                && installed.as_ref().is_some_and(|installed| {
                    resolve_provider_generation_with_auth(
                        &state.provider_catalog,
                        &state.plugin_catalog,
                        std::slice::from_ref(installed),
                        &account.provider_id,
                        SchedulingAuthentication::Native(&completion.profile),
                    )
                    .is_ok()
                }) =>
        {
            Some(json!({
                "generation": completion.profile.generation,
                "provider_version": completion.profile.provider_version,
                "generation_digest": completion.profile.generation_digest,
                "method": completion.profile.method,
            }))
        }
        _ => None,
    };
    Ok(json!({
        "account_id": account_id, "provider_id": account.provider_id,
        "machine_id": machine_id, "registry_revision": registry.revision,
        "available": available,
        "reason": reason.or_else(|| (!available).then_some("native_identity_verification_unavailable")),
        "release": installed.map(|installed| json!({"provider_version":installed.plugin_version,"generation_digest":installed.generation_digest})),
        "methods":methods,
        "native_profile":native_profile,
        "session_admission":if super::super::allow_named_creation(&account.provider_id,
            state.service_config.get(&crate::config::schema::ACCOUNT_NATIVE_CLAUDE_SESSIONS)).is_ok() {"enabled"} else {"not_enabled"},
    }))
}

fn readiness_reason(
    enabled: bool,
    permitted: bool,
    enrolled: bool,
    protocol: Option<u16>,
    installed: bool,
    package: bool,
    recipient: bool,
) -> Option<&'static str> {
    if !enabled {
        Some("account_disabled")
    } else if !permitted {
        Some("machine_not_permitted")
    } else if !enrolled || protocol.is_none() {
        Some("machine_unavailable")
    } else if protocol.is_some_and(|protocol| {
        protocol < crate::machine_protocol::PROVIDER_ACCOUNT_PROTOCOL_VERSION
    }) {
        Some("machine_account_protocol_required")
    } else if !installed {
        Some("provider_not_installed")
    } else if !package {
        Some("release_unavailable")
    } else if !recipient {
        Some("machine_recipient_unavailable")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readiness_requires_each_local_admission_condition() {
        let protocol = Some(crate::machine_protocol::PROVIDER_ACCOUNT_PROTOCOL_VERSION);
        assert_eq!(
            readiness_reason(true, true, true, protocol, true, true, true),
            None
        );
        for (enabled, permitted, enrolled, protocol, installed, package, recipient, expected) in [
            (
                false,
                true,
                true,
                protocol,
                true,
                true,
                true,
                "account_disabled",
            ),
            (
                true,
                false,
                true,
                protocol,
                true,
                true,
                true,
                "machine_not_permitted",
            ),
            (
                true,
                true,
                false,
                protocol,
                true,
                true,
                true,
                "machine_unavailable",
            ),
            (
                true,
                true,
                true,
                None,
                true,
                true,
                true,
                "machine_unavailable",
            ),
            (
                true,
                true,
                true,
                Some(0),
                true,
                true,
                true,
                "machine_account_protocol_required",
            ),
            (
                true,
                true,
                true,
                protocol,
                false,
                true,
                true,
                "provider_not_installed",
            ),
            (
                true,
                true,
                true,
                protocol,
                true,
                false,
                true,
                "release_unavailable",
            ),
            (
                true,
                true,
                true,
                protocol,
                true,
                true,
                false,
                "machine_recipient_unavailable",
            ),
        ] {
            assert_eq!(
                readiness_reason(
                    enabled, permitted, enrolled, protocol, installed, package, recipient
                ),
                Some(expected)
            );
        }
    }

    #[test]
    fn discovery_query_cannot_override_account_or_owner() {
        assert!(serde_json::from_value::<QueryOptions>(json!({"machine_id":"machine"})).is_ok());
        for key in ["owner", "account_id", "provider_id", "credentials"] {
            assert!(
                serde_json::from_value::<QueryOptions>(
                    json!({"machine_id":"machine",key:"override"})
                )
                .is_err()
            );
        }
    }
}
