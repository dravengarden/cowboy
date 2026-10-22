//! Directory defaults are creation inputs, never live session policy.
use super::*;
use crate::session_folders::defaults::{DefaultKey, ResolvedDefaults};

pub(super) async fn create(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Json(mut body): Json<serde_json::Value>,
) -> Response {
    if !auth.principal.can_reorder() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(object) = body.as_object_mut() else {
        return (StatusCode::BAD_REQUEST, "Expected an object").into_response();
    };
    let id = format!("f-{}", uuid::Uuid::new_v4());
    object.insert("id".into(), serde_json::Value::String(id.clone()));
    match state.hub.sync_apply_as(
        &folder_actor(&state, &auth.principal),
        "folders",
        format!("create:{id}"),
        "create",
        &body,
    ) {
        Ok(()) => (StatusCode::CREATED, Json(serde_json::json!({"id":id}))).into_response(),
        Err(error) => (StatusCode::BAD_REQUEST, error).into_response(),
    }
}

pub(super) async fn list(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
) -> Response {
    let actor = folder_actor(&state, &auth.principal);
    Json(crate::session_folders::project_folders_value(
        state.hub.sync_value("folders"),
        &std::collections::HashSet::new(),
        |owner| actor.sees_all || owner == actor.user_id.as_deref(),
    ))
    .into_response()
}

pub(super) async fn update(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(id): Path<String>,
    Json(mut body): Json<serde_json::Value>,
) -> Response {
    if !auth.principal.can_reorder() {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(object) = body.as_object_mut() else {
        return (StatusCode::BAD_REQUEST, "Expected an object").into_response();
    };
    object.insert("id".into(), serde_json::Value::String(id.clone()));
    match state.hub.sync_apply_as(
        &folder_actor(&state, &auth.principal),
        "folders",
        format!("defaults:{}", uuid::Uuid::new_v4()),
        "defaults",
        &body,
    ) {
        Ok(()) => get_defaults(State(state), Extension(auth), Path(id)).await,
        Err(error) => (StatusCode::BAD_REQUEST, error).into_response(),
    }
}

pub(super) async fn get_defaults(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(id): Path<String>,
) -> Response {
    match state
        .hub
        .folder_defaults(&folder_actor(&state, &auth.principal), &id)
    {
        Ok(defaults) => Json(defaults).into_response(),
        Err(error) => (StatusCode::NOT_FOUND, error).into_response(),
    }
}

pub(super) fn resolve(
    state: &AppState,
    principal: &ProductPrincipal,
    folder: Option<&str>,
) -> Result<ResolvedDefaults, String> {
    folder.filter(|id| !id.is_empty()).map_or_else(
        || Ok(ResolvedDefaults::default()),
        |id| {
            state
                .hub
                .folder_defaults(&folder_actor(state, principal), id)
        },
    )
}

pub(super) fn value(defaults: &ResolvedDefaults, key: DefaultKey) -> Option<String> {
    defaults.values.get(&key).cloned().flatten()
}

/// Service-side counterpart of New Session's automatic local/connected project
/// choice. Explicit identities are filters, never hints that may be discarded.
pub(super) async fn default_workspace(
    state: &AppState,
    machine: Option<&str>,
    project: Option<&str>,
) -> Result<(String, String), String> {
    let machines = state
        .store
        .as_ref()
        .ok_or("Machine registry unavailable")?
        .list_machines()
        .await
        .map_err(|_| "Machine registry unavailable")?;
    let policy = state.project_placement.snapshot();
    select_workspace(
        &machines,
        &policy,
        |id| state.runtime_router.connected(id),
        machine,
        project,
    )
}

fn select_workspace(
    machines: &[crate::store::MachineRecord],
    policy: &crate::project_placement::Configuration,
    connected: impl Fn(&str) -> bool,
    machine: Option<&str>,
    project: Option<&str>,
) -> Result<(String, String), String> {
    let mut candidates = Vec::new();
    for entry in machines {
        if entry.revoked
            || machine.is_some_and(|id| id != entry.id)
            || !policy.policy(&entry.id).hosts_projects
        {
            continue;
        }
        let roots: Vec<crate::machine_protocol::MachineWorkspace> = serde_json::from_value(
            entry
                .inventory
                .get("workspaces")
                .cloned()
                .unwrap_or_else(|| serde_json::json!([])),
        )
        .map_err(|_| "Invalid Machine project inventory")?;
        let selected = if project.is_some() {
            workspace(&roots, project).ok()
        } else {
            roots
                .iter()
                .find(|root| root.display_name == "columbus")
                .or_else(|| roots.first())
        };
        if let Some(root) = selected {
            let connected = connected(&entry.id);
            let rank = if connected && entry.connection_mode == "local" {
                0
            } else if connected {
                1
            } else {
                2
            };
            candidates.push((
                rank,
                root.display_name != "columbus",
                entry.id.clone(),
                root.id.clone(),
            ));
        }
    }
    candidates.sort();
    candidates
        .into_iter()
        .next()
        .map(|(_, _, machine, project)| (machine, project))
        .ok_or_else(|| {
            "Directory project is unavailable; choose a registered project and Machine".into()
        })
}

pub(super) fn workspace<'a>(
    workspaces: &'a [crate::machine_protocol::MachineWorkspace],
    requested: Option<&str>,
) -> Result<&'a crate::machine_protocol::MachineWorkspace, String> {
    if let Ok(workspace) = resolve_machine_workspace(workspaces, requested) {
        return Ok(workspace);
    }
    // Legacy project bindings contain labels. Never interpret them as paths.
    let mut matches = workspaces
        .iter()
        .filter(|workspace| Some(workspace.display_name.as_str()) == requested);
    let candidate = matches
        .next()
        .ok_or("Directory project is unavailable on the selected Machine")?;
    if matches.next().is_some() {
        return Err("Directory project name is ambiguous; select its registered project id".into());
    }
    Ok(candidate)
}

pub(super) fn preset_values(
    state: &AppState,
    provider: &str,
    generation: &ResolvedProviderGeneration,
    preset: Option<&str>,
) -> Result<BTreeMap<String, String>, String> {
    let Some(preset) = preset.filter(|value| !value.is_empty()) else {
        return Ok(BTreeMap::new());
    };
    let (owner, id) = preset
        .split_once('/')
        .ok_or("preset must be provider/preset")?;
    if owner != provider {
        return Err("Directory preset belongs to another agent; choose a compatible preset or use agent defaults".into());
    }
    let package = state
        .provider_catalog
        .package(provider, &generation.version, &generation.digest)
        .ok_or("Provider package unavailable")?;
    package
        .manifest
        .configuration
        .presets
        .iter()
        .find(|preset| preset.id == id)
        .map(|preset| preset.values.clone())
        .ok_or_else(|| "Directory preset is unavailable in the installed agent version".into())
}

pub(super) fn apply_created(
    state: &AppState,
    principal: &ProductPrincipal,
    session: &str,
    folder: Option<&str>,
    configuration: BTreeMap<String, String>,
) -> Result<(), String> {
    for (key, value) in configuration {
        state
            .hub
            .set_config_preference(session, key, serde_json::Value::String(value))?;
    }
    if let Some(folder) = folder {
        let actor = folder_actor(state, principal);
        let mutation = format!("created-folder:{session}");
        let result = state.hub.sync_apply_as(&actor, "folders", mutation.clone(), "place",
            &serde_json::json!({"session_ids":[session], "folder": if folder.is_empty() { None } else { Some(folder) }}));
        // A removed directory must not turn a successfully created session into
        // an error inviting duplicate retries. Its session remains at Global.
        if result.is_err() && state.hub.folder_defaults(&actor, folder).is_err() {
            state.hub.sync_apply_as(
                &actor,
                "folders",
                mutation,
                "place",
                &serde_json::json!({"session_ids":[session],"folder":null}),
            )?;
        } else {
            result?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{select_workspace, workspace};
    use crate::machine_protocol::MachineWorkspace;

    #[test]
    fn directory_routes_use_product_authentication_and_operator_mutation_grants() {
        use super::super::{Method, RouteAuth, classify_route};
        for path in ["/api/folders", "/api/folders/f-test/defaults"] {
            assert_eq!(classify_route(&Method::GET, path), RouteAuth::Product);
            assert_eq!(classify_route(&Method::HEAD, path), RouteAuth::Product);
            assert_eq!(
                classify_route(&Method::POST, path),
                RouteAuth::ProductOperator
            );
            assert_eq!(
                classify_route(&Method::PATCH, path),
                RouteAuth::ProductOperator
            );
        }
    }

    #[test]
    fn automatic_placement_respects_explicit_bindings_and_revocation() {
        let machine = |id: &str, mode: &str| crate::store::MachineRecord {
            id: id.into(),
            display_name: id.into(),
            connection_mode: mode.into(),
            platform: "linux".into(),
            architecture: "x86_64".into(),
            status: "online".into(),
            inventory: serde_json::json!({"workspaces":[{"id":"project-id","display_name":"suger","canonical_path":"/trusted/suger"}]}),
            last_seen_at_ms: None,
            revoked: false,
            fingerprint: None,
        };
        let mut machines = vec![machine("remote", "remote"), machine("local-host", "local")];
        let policy = crate::project_placement::Configuration {
            schema: 1,
            revision: "test".into(),
            default_runtime_machine_id: None,
            machines: Default::default(),
        };
        assert_eq!(
            select_workspace(&machines, &policy, |_| true, None, Some("suger")).unwrap(),
            ("local-host".into(), "project-id".into())
        );
        assert_eq!(
            select_workspace(
                &machines,
                &policy,
                |_| true,
                Some("remote"),
                Some("project-id")
            )
            .unwrap()
            .0,
            "remote"
        );
        assert!(
            select_workspace(&machines, &policy, |_| true, Some("missing"), Some("suger")).is_err()
        );
        assert!(
            select_workspace(&machines, &policy, |_| true, None, Some("missing-project")).is_err()
        );
        machines[1].revoked = true;
        assert!(
            select_workspace(
                &machines,
                &policy,
                |_| true,
                Some("local-host"),
                Some("suger")
            )
            .is_err()
        );
        assert_eq!(
            select_workspace(&machines, &policy, |_| true, None, None)
                .unwrap()
                .0,
            "remote"
        );
    }

    #[test]
    fn legacy_projects_only_resolve_unambiguous_registered_roots() {
        let projects = vec![
            MachineWorkspace {
                id: "one".into(),
                display_name: "suger".into(),
                canonical_path: "/trusted/one".into(),
            },
            MachineWorkspace {
                id: "two".into(),
                display_name: "other".into(),
                canonical_path: "/trusted/two".into(),
            },
        ];
        assert_eq!(workspace(&projects, Some("one")).unwrap().id, "one");
        assert_eq!(workspace(&projects, Some("suger")).unwrap().id, "one");
        assert!(workspace(&projects, Some("/trusted/one")).is_err());
        assert!(workspace(&projects, None).is_err());
        let mut ambiguous = projects.clone();
        ambiguous[1].display_name = "suger".into();
        assert!(workspace(&ambiguous, Some("suger")).is_err());
        assert_eq!(workspace(&ambiguous, Some("one")).unwrap().id, "one");
    }
}
