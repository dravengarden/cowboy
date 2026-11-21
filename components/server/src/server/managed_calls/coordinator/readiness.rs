//! Where a managed child's Agent runtime runs, on which subscription, and
//! whether it can run now.
use std::collections::HashSet;

use super::super::super::AppState;

pub(in crate::server::managed_calls) struct ProviderReadiness {
    pub available: bool,
    pub reason: Option<&'static str>,
    pub generation: Option<super::super::super::ResolvedProviderGeneration>,
    pub structured_output: bool,
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
                account: Default::default(),
            },
        );
    }
    let local = provider_readiness(state, owner, execution_machine, provider, profile).await;
    // A context-only adapter starts only with a keeper-backed execution
    // descriptor, which only a split child supplies: never run it locally,
    // and never let a missing local login hide a remote one.
    let split_only = super::super::super::agent_tools::context_only(provider);
    if !split_only
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
        if readiness.available {
            return (runtime, readiness);
        }
        remote_reason.get_or_insert(readiness.reason.unwrap_or("provider_unavailable"));
    }
    let readiness = match remote_reason {
        Some(reason) => ProviderReadiness {
            available: false,
            reason: Some(reason),
            generation: None,
            structured_output: false,
            account: Default::default(),
        },
        None if split_only => ProviderReadiness {
            available: false,
            reason: Some("runtime_policy"),
            generation: None,
            structured_output: false,
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
                state, owner, provider, None,
            )
            .await
            {
                Ok(account) => account,
                Err(_) => return unavailable("authentication_required"),
            }
        }
        None => Default::default(),
    };
    let resolved = match owner {
        Some(owner) if !account.is_default() => {
            super::super::super::provider_accounts::registration::generation(
                state, &inventory, provider, owner, &account, machine,
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
    let supports = state
        .provider_catalog
        .package(provider, &generation.version, &generation.digest)
        .is_some_and(|package| {
            package
                .manifest
                .runtime
                .launch_arguments(Some(profile))
                .is_ok()
        });
    if !supports {
        return ProviderReadiness {
            available: false,
            reason: Some("unsupported_capability"),
            generation: Some(generation),
            structured_output: false,
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
        structured_output: profile == cowboy_provider_sdk::ManagedRuntimeProfile::ReadOnlyV1,
    }
}
