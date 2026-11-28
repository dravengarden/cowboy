//! Where a managed child's Agent runtime runs, on which subscription, and
//! whether it can run now.
use std::collections::HashSet;

use super::super::super::AppState;

/// The signed managed-profile argument by which a context-only adapter
/// (Antigravity) declares that it enforces a round's output schema itself.
pub(in crate::server::managed_calls) const OUTPUT_CONTRACT_FLAG: &str =
    "--cowboy-output-contract=v1";
/// The signed managed-profile argument by which a context-only adapter
/// (Antigravity) declares that a replacement proxy can safely reopen the exact
/// `managed_session_id` bound in its trusted execution descriptor.
pub(in crate::server::managed_calls) const MANAGED_CONTINUE_FLAG: &str =
    "--cowboy-managed-continue=v1";

pub(in crate::server::managed_calls) struct ProviderReadiness {
    pub available: bool,
    pub reason: Option<&'static str>,
    pub generation: Option<super::super::super::ResolvedProviderGeneration>,
    pub structured_output: bool,
    pub managed_continue: bool,
    /// The subscription the child runs on: the owner's default account of
    /// the Provider, never a fallback to another account.
    pub account: crate::provider_account::ProviderAccountId,
}

impl ProviderReadiness {
    /// The signed presets of the generation this runtime would launch (the
    /// latest package when none is ready): what review admission ranks.
    pub(in crate::server::managed_calls) fn presets(
        &self,
        state: &AppState,
        provider: &str,
    ) -> serde_json::Value {
        let generation = self.generation.as_ref();
        super::super::super::agent_tools::presets_on(
            state,
            provider,
            generation.map(|g| (g.version.as_str(), g.digest.as_str())),
        )
    }
}

/// Where a managed child's Agent runtime runs, and whether it can. The
/// execution Machine is preferred. When host policy places the Provider on
/// other Machines, one of them (the parent's runtime first) runs the child
/// against the execution Machine's snapshot through a keeper-enforced
/// read-only environment. Execution never moves; there is no local fallback.
pub(in crate::server::managed_calls) async fn child_runtime(
    state: &AppState,
    owner: Option<&str>,
    execution_machine: &str,
    parent_runtime: Option<&str>,
    provider: &str,
    profile: cowboy_provider_sdk::ManagedRuntimeProfile,
) -> (String, ProviderReadiness) {
    child_runtime_pinned(
        state,
        owner,
        execution_machine,
        parent_runtime,
        provider,
        profile,
        None,
    )
    .await
}

/// `child_runtime`, preferring a runtime whose generation is `pinned` (the
/// digest admission recorded), so a claim finds the admitted generation
/// while it is still ready anywhere.
pub(in crate::server::managed_calls) async fn child_runtime_pinned(
    state: &AppState,
    owner: Option<&str>,
    execution_machine: &str,
    parent_runtime: Option<&str>,
    provider: &str,
    profile: cowboy_provider_sdk::ManagedRuntimeProfile,
    pinned: Option<&str>,
) -> (String, ProviderReadiness) {
    if profile == cowboy_provider_sdk::ManagedRuntimeProfile::ImageV1
        && state
            .machine_control
            .connection_protocol(execution_machine)
            .is_none()
    {
        return (
            execution_machine.into(),
            ProviderReadiness {
                available: false,
                reason: Some("unsupported_capability"),
                generation: None,
                structured_output: false,
                managed_continue: false,
                account: Default::default(),
            },
        );
    }
    let local = provider_readiness(state, owner, execution_machine, provider, profile).await;
    // A context-only adapter starts only with a keeper-backed execution
    // descriptor, which only a split child supplies: never run it locally,
    // and never let a missing local login hide a remote one.
    let split_only = super::super::super::agent_tools::context_only(provider);
    let on_pin = |readiness: &ProviderReadiness| {
        pinned.is_none_or(|digest| {
            readiness
                .generation
                .as_ref()
                .is_some_and(|generation| generation.digest == digest)
        })
    };
    // A local runtime on another generation than the pinned one is only a
    // fallback: a permitted remote may still run the pinned generation.
    let local_fallback = !split_only && local.available && !on_pin(&local);
    if !local_fallback
        && !split_only
        && (local.available
            || !matches!(
                local.reason,
                Some("runtime_policy" | "provider_unavailable" | "unsupported_capability")
            ))
    {
        return (execution_machine.to_owned(), local);
    }
    let mut candidates: Vec<String> = parent_runtime.into_iter().map(str::to_owned).collect();
    if let Some(store) = &state.store
        && let Ok(machines) = store.list_machines().await
    {
        let mut registered: Vec<String> = machines
            .into_iter()
            .filter(|machine| !machine.revoked)
            .map(|machine| machine.id)
            .collect();
        registered.sort();
        candidates.extend(registered);
    }
    let mut seen = HashSet::new();
    candidates.retain(|machine| machine != execution_machine && seen.insert(machine.clone()));
    let mut remote_reason = None;
    // The pinned generation first; else, as a context-only adapter's output
    // contract varies by generation, a runtime that enforces it over the
    // first one that merely runs.
    let mut fallback = None;
    for runtime in candidates {
        if !state.supervisor.runtime_allowed(provider, &runtime)
            || !state
                .project_placement
                .snapshot()
                .allows(&runtime, execution_machine)
        {
            continue;
        }
        let readiness =
            runtime_readiness(state, owner, execution_machine, &runtime, provider, profile).await;
        let preferred = if pinned.is_some() {
            on_pin(&readiness)
        } else {
            !split_only || readiness.structured_output
        };
        if readiness.available && preferred {
            return (runtime, readiness);
        }
        if readiness.available {
            fallback.get_or_insert((runtime, readiness));
            continue;
        }
        remote_reason.get_or_insert(readiness.reason.unwrap_or("provider_unavailable"));
    }
    if local_fallback {
        return (execution_machine.to_owned(), local);
    }
    if let Some(found) = fallback {
        return found;
    }
    let readiness = match remote_reason {
        Some(reason) => ProviderReadiness {
            available: false,
            reason: Some(reason),
            generation: None,
            structured_output: false,
            managed_continue: false,
            account: Default::default(),
        },
        None if split_only => ProviderReadiness {
            available: false,
            reason: Some("runtime_policy"),
            generation: None,
            structured_output: false,
            managed_continue: false,
            account: Default::default(),
        },
        None => local,
    };
    (execution_machine.to_owned(), readiness)
}

/// Whether a child of `provider` can run with its runtime on `runtime` and
/// its execution on `execution_machine`.
pub(in crate::server::managed_calls) async fn runtime_readiness(
    state: &AppState,
    owner: Option<&str>,
    execution_machine: &str,
    runtime: &str,
    provider: &str,
    profile: cowboy_provider_sdk::ManagedRuntimeProfile,
) -> ProviderReadiness {
    let readiness = provider_readiness(state, owner, runtime, provider, profile).await;
    if runtime == execution_machine || !readiness.available {
        return readiness;
    }
    let checked = match &readiness.generation {
        Some(generation) => remote_ready(state, execution_machine, runtime, generation).await,
        None => Err("provider_unavailable"),
    };
    match checked {
        Ok(()) => readiness,
        Err(reason) => ProviderReadiness {
            available: false,
            reason: Some(reason),
            generation: None,
            structured_output: false,
            managed_continue: false,
            account: Default::default(),
        },
    }
}

/// A split child needs both Machines to understand remote managed bindings
/// and the exact runtime Provider to accept the target's executor.
async fn remote_ready(
    state: &AppState,
    execution_machine: &str,
    runtime_machine: &str,
    generation: &super::super::super::ResolvedProviderGeneration,
) -> Result<(), &'static str> {
    for machine in [execution_machine, runtime_machine] {
        match state.machine_control.connection_protocol(machine) {
            Some(_) => {}
            None => return Err("machine_unavailable"),
        }
    }
    let executor = super::super::super::execution::sessions::executor(state, execution_machine)
        .await
        .map_err(|_| "machine_unavailable")?;
    if !super::super::super::execution::sessions::accepts(&generation.behavior, &executor) {
        return Err("unsupported_capability");
    }
    Ok(())
}

/// Whether the exact installed Provider on the target can run this request.
pub(in crate::server::managed_calls) async fn provider_readiness(
    state: &AppState,
    owner: Option<&str>,
    machine: &str,
    provider: &str,
    profile: cowboy_provider_sdk::ManagedRuntimeProfile,
) -> ProviderReadiness {
    let unavailable = |reason| ProviderReadiness {
        available: false,
        reason: Some(reason),
        generation: None,
        structured_output: false,
        managed_continue: false,
        account: Default::default(),
    };
    let Some(store) = state.store.as_ref() else {
        return unavailable("provider_unavailable");
    };
    let Ok(machines) = store.list_machines().await else {
        return unavailable("provider_unavailable");
    };
    let Some(record) = machines
        .iter()
        .find(|record| record.id == machine && !record.revoked)
    else {
        return unavailable("provider_unavailable");
    };
    if !state.runtime_router.connected(machine) {
        return unavailable("machine_unavailable");
    }
    if profile == cowboy_provider_sdk::ManagedRuntimeProfile::ImageV1
        && state.machine_control.connection_protocol(machine).is_none()
    {
        return unavailable("unsupported_capability");
    }
    // Host policy may pin a Provider's runtime to other Machines. A managed
    // child runs on the parent's execution Machine, never elsewhere.
    if !state.supervisor.runtime_allowed(provider, machine) {
        return unavailable("runtime_policy");
    }
    let inventory = super::super::super::execution::sessions::providers(record);
    // A child runs on its owner's default subscription of the Provider, as a
    // session created without an explicit account would. A named default
    // needs its native login on this Machine; it never falls back.
    let account = match owner {
        Some(owner) => {
            match super::super::super::provider_accounts::resolve_for_create(
                &state.accounts,
                owner,
                provider,
                None,
            )
            .await
            {
                Ok(account) => account,
                // A retired card default needs a named selection, not a sign-in.
                Err("named_account_required") => return unavailable("named_account_required"),
                Err(_) => return unavailable("authentication_required"),
            }
        }
        None => match super::super::super::provider_accounts::creatable_default(provider) {
            Ok(()) => Default::default(),
            Err(reason) => return unavailable(reason),
        },
    };
    let resolved = match owner {
        Some(owner) if !account.is_default() => {
            super::super::super::provider_accounts::registration::generation(
                &state.accounts,
                &inventory,
                provider,
                owner,
                &account,
                machine,
            )
            .await
        }
        _ => super::super::super::resolve_provider_generation(
            &state.provider_catalog,
            &state.plugin_catalog,
            &inventory,
            provider,
            state.provider_auth.status(provider).as_ref(),
        ),
    };
    let generation = match resolved {
        Ok(generation) => generation,
        Err(detail) => {
            return unavailable(
                if !account.is_default() || detail.contains("auth") || detail.contains("sign") {
                    "authentication_required"
                } else {
                    "provider_unavailable"
                },
            );
        }
    };
    let package = state
        .provider_catalog
        .package(provider, &generation.version, &generation.digest);
    let arguments = package.as_ref().and_then(|package| {
        package
            .manifest
            .runtime
            .launch_arguments(Some(profile))
            .ok()
    });
    let supports = arguments.is_some();
    let output_contract = enforces_output(provider, arguments);
    let managed_continue = supports_managed_continue(
        provider,
        arguments,
        state.machine_control.supports_managed_resume(machine),
    );
    if !supports {
        return ProviderReadiness {
            available: false,
            reason: Some("unsupported_capability"),
            generation: Some(generation),
            structured_output: false,
            managed_continue: false,
            account,
        };
    }
    ProviderReadiness {
        available: true,
        reason: None,
        generation: Some(generation),
        account,
        // The managed read-only profile contract includes the native turn
        // constraint; an adapter declaring it must apply the schema.
        structured_output: profile == cowboy_provider_sdk::ManagedRuntimeProfile::ReadOnlyV1
            && output_contract,
        managed_continue,
    }
}

/// Whether `provider`'s managed launch `arguments` enforce a round's output
/// schema. A context-only adapter has no native schema constraint: it
/// enforces one only when its signed managed profile declares the contract.
pub(in crate::server::managed_calls) fn enforces_output(
    provider: &str,
    arguments: Option<&[cowboy_provider_sdk::RuntimeValue]>,
) -> bool {
    !super::super::super::agent_tools::context_only(provider)
        || arguments.is_some_and(|arguments| {
            arguments.iter().any(|argument| {
                matches!(argument, cowboy_provider_sdk::RuntimeValue::Literal(flag)
                    if flag == OUTPUT_CONTRACT_FLAG)
            })
        })
}

/// Whether `provider` can continue a hibernated managed child on `machine`.
/// A context-only adapter needs both its signed continuation flag and a
/// Machine whose worker binds `managed_session_id` in the sandbox descriptor.
pub(in crate::server::managed_calls) fn supports_managed_continue(
    provider: &str,
    arguments: Option<&[cowboy_provider_sdk::RuntimeValue]>,
    machine_managed_resume: bool,
) -> bool {
    !super::super::super::agent_tools::context_only(provider)
        || (machine_managed_resume
            && arguments.is_some_and(|arguments| {
                arguments.iter().any(|argument| {
                    matches!(argument, cowboy_provider_sdk::RuntimeValue::Literal(flag)
                        if flag == MANAGED_CONTINUE_FLAG)
                })
            }))
}

/// Whether `child` has a resumable native session ID when its Provider
/// requires trusted native-session binding for continuation.
pub(in crate::server::managed_calls) fn child_has_resumable_native_id(
    state: &AppState,
    child: &crate::core::SessionMeta,
) -> bool {
    !super::super::super::agent_tools::context_only(&child.provider)
        || state
            .hub
            .agent_session_id_for_resume(&child.id)
            .as_deref()
            .is_some_and(|id| !id.trim().is_empty())
}

pub(in crate::server::managed_calls) fn child_continuable_under_fence(
    state: &AppState,
    child_session_id: &str,
) -> bool {
    !state
        .managed_calls
        .retiring
        .lock()
        .contains(child_session_id)
        && state
            .hub
            .session_info(child_session_id)
            .is_some_and(|child| {
                !child.meta.closing && child_has_resumable_native_id(state, &child.meta)
            })
}

/// Verify an existing child before quiescing it, returning the exact
/// resumable native ID that must still match at worker launch when continuing
/// a context-only child.
pub(in crate::server::managed_calls) fn expected_resume_for_reconfirm(
    state: &AppState,
    record: &crate::managed_calls::lifecycle::Record,
    request: &crate::managed_calls::Request,
) -> Result<Option<String>, &'static str> {
    if !matches!(
        request.conversation,
        crate::managed_calls::Conversation::Continue { .. }
    ) || !super::super::super::agent_tools::context_only(&record.provider)
    {
        return Ok(None);
    }
    state
        .hub
        .agent_session_id_for_resume(&record.child_session_id)
        .filter(|id| !id.trim().is_empty())
        .map(Some)
        .ok_or("conversation_unavailable")
}

pub(in crate::server::managed_calls) fn check_existing_child_before_quiesce(
    state: &AppState,
    child: &crate::core::SessionMeta,
    request: &crate::managed_calls::Request,
    profile: cowboy_provider_sdk::ManagedRuntimeProfile,
) -> Result<Option<String>, &'static str> {
    if !request.output.is_text() && !pinned_child_enforces_output(state, child, profile) {
        return Err("unsupported_capability");
    }
    if matches!(
        request.conversation,
        crate::managed_calls::Conversation::Continue { .. }
    ) {
        let expected_resume = if super::super::super::agent_tools::context_only(&child.provider) {
            let Some(native_id) = state
                .hub
                .agent_session_id_for_resume(&child.id)
                .filter(|id| !id.trim().is_empty())
            else {
                return Err("conversation_unavailable");
            };
            Some(native_id)
        } else {
            None
        };
        if !pinned_child_supports_continue(state, child, profile) {
            return Err("unsupported_capability");
        }
        return Ok(expected_resume);
    }
    Ok(None)
}

/// Whether an existing child's pinned signed generation enforces Cowboy's
/// structured output contract for `profile`.
pub(in crate::server::managed_calls) fn pinned_child_enforces_output(
    state: &AppState,
    child: &crate::core::SessionMeta,
    profile: cowboy_provider_sdk::ManagedRuntimeProfile,
) -> bool {
    let package = state.provider_catalog.package(
        &child.provider,
        &child.provider_version,
        &child.provider_generation_digest,
    );
    let arguments = package
        .as_ref()
        .and_then(|pkg| pkg.manifest.runtime.launch_arguments(Some(profile)).ok());
    enforces_output(&child.provider, arguments)
}

/// Whether an existing child's pinned signed generation AND pinned runtime
/// Machine support managed continuation for `profile`.
pub(in crate::server::managed_calls) fn pinned_child_supports_continue(
    state: &AppState,
    child: &crate::core::SessionMeta,
    profile: cowboy_provider_sdk::ManagedRuntimeProfile,
) -> bool {
    let package = state.provider_catalog.package(
        &child.provider,
        &child.provider_version,
        &child.provider_generation_digest,
    );
    let arguments = package
        .as_ref()
        .and_then(|pkg| pkg.manifest.runtime.launch_arguments(Some(profile)).ok());
    supports_managed_continue(
        &child.provider,
        arguments,
        state
            .machine_control
            .supports_managed_resume(&child.machine_id),
    )
}

/// Readiness of an existing parent-owned child's exact pinned runtime Machine,
/// account, and signed Provider generation (never probing another Machine or a
/// newer fresh-child generation).
pub(in crate::server::managed_calls) async fn pinned_child_readiness(
    state: &AppState,
    execution_machine: &str,
    child: &crate::core::SessionMeta,
    profile: cowboy_provider_sdk::ManagedRuntimeProfile,
) -> Result<
    (
        Option<(String, String)>,
        crate::provider_account::ProviderAccountId,
        bool,
        bool,
    ),
    &'static str,
> {
    let Some(store) = state.store.as_ref() else {
        return Err("provider_unavailable");
    };
    let Ok(machines) = store.list_machines().await else {
        return Err("provider_unavailable");
    };
    if !machines
        .iter()
        .any(|record| record.id == child.machine_id && !record.revoked)
    {
        return Err("provider_unavailable");
    }
    if !state.runtime_router.connected(&child.machine_id) {
        return Err("machine_unavailable");
    }
    if !state
        .supervisor
        .runtime_allowed(&child.provider, &child.machine_id)
    {
        return Err("runtime_policy");
    }
    if child.machine_id != execution_machine
        && (state
            .machine_control
            .connection_protocol(execution_machine)
            .is_none()
            || state
                .machine_control
                .connection_protocol(&child.machine_id)
                .is_none())
    {
        return Err("machine_unavailable");
    }
    let package = state.provider_catalog.package(
        &child.provider,
        &child.provider_version,
        &child.provider_generation_digest,
    );
    let arguments = package
        .as_ref()
        .and_then(|pkg| pkg.manifest.runtime.launch_arguments(Some(profile)).ok());
    if super::super::super::agent_tools::context_only(&child.provider) && arguments.is_none() {
        return Err("unsupported_capability");
    }
    if package.is_some() && arguments.is_none() {
        return Err("unsupported_capability");
    }
    let structured_output = profile == cowboy_provider_sdk::ManagedRuntimeProfile::ReadOnlyV1
        && enforces_output(&child.provider, arguments);
    let managed_continue = supports_managed_continue(
        &child.provider,
        arguments,
        state
            .machine_control
            .supports_managed_resume(&child.machine_id),
    );
    let generation = (!child.provider_version.is_empty()
        || !child.provider_generation_digest.is_empty())
    .then(|| {
        (
            child.provider_version.clone(),
            child.provider_generation_digest.clone(),
        )
    });
    Ok((
        generation,
        child.provider_account_id.clone(),
        structured_output,
        managed_continue,
    ))
}

#[cfg(test)]
mod output_tests {
    use cowboy_provider_sdk::RuntimeValue;

    use super::*;

    #[test]
    fn only_a_declared_output_contract_makes_antigravity_structured() {
        let literal = |text: &str| RuntimeValue::Literal(text.to_owned());
        let old = [
            literal("acp"),
            literal("--cowboy-managed-profile=read-only-v1"),
        ];
        let new = [
            literal("acp"),
            literal("--cowboy-managed-profile=read-only-v1"),
            literal(OUTPUT_CONTRACT_FLAG),
        ];
        assert!(!enforces_output("antigravity", Some(&old)));
        assert!(!enforces_output("antigravity", None));
        assert!(enforces_output("antigravity", Some(&new)));
        // Codex and Claude constrain the turn natively.
        assert!(enforces_output("codex", Some(&old)));
        assert!(enforces_output("claude-code", None));
        let with_continue = [
            literal("acp"),
            literal("--cowboy-managed-profile=read-only-v1"),
            literal(OUTPUT_CONTRACT_FLAG),
            literal(MANAGED_CONTINUE_FLAG),
        ];
        assert!(!supports_managed_continue("antigravity", Some(&new), true));
        assert!(!supports_managed_continue(
            "antigravity",
            Some(&with_continue),
            false
        ));
        assert!(supports_managed_continue(
            "antigravity",
            Some(&with_continue),
            true
        ));
        assert!(supports_managed_continue("codex", Some(&old), false));
    }
}
