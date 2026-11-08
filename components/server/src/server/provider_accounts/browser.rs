//! Owner-operated Machine browsers for named accounts: admission (Service
//! gate, account enablement), Machine selection, the lifecycle commands the
//! owner may issue, durable placements and the state read shared by HTTP and
//! MCP. The display stream itself is `viewer` and `machine_stream`.
//! See `docs/machine-browser.md`.

use super::*;
use crate::machine_control::ConnectionToken;
use crate::machine_protocol::MACHINE_BROWSER_PROTOCOL_VERSION;
use crate::machine_protocol::machine_browser::{
    Command, InstanceRef, Purpose, State as BrowserState, StateEvent, Viewport,
};
use crate::provider_account::ProviderAccountId;
use crate::store::provider_accounts::browser_placements::Placement;
use serde_json::{Value, json};

pub(in crate::server) mod coordinator;
pub(in crate::server) mod machine_stream;
pub(in crate::server) mod revocation;
pub(in crate::server) mod viewer;

/// Unit start, snapshot restoration and launcher readiness.
const OPEN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(150);
const COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
/// Graceful stop with a stop-time snapshot.
const HIBERNATE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);
const RESET_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const QUERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

pub(in crate::server) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

fn request_id(prefix: &str) -> String {
    format!(
        "{prefix}-{}",
        super::super::machine_connection::random_machine_token()
            .unwrap_or_else(|_| now_ms().to_string())
    )
}

/// Whether the owner may operate a browser for the account: ownership, the
/// Service gate, the account's availability and its browser enablement.
pub(in crate::server) async fn admit(
    state: &AppState,
    owner: &str,
    account_id: &ProviderAccountId,
) -> Result<(), &'static str> {
    if account_id.is_default() {
        return Err("named_account_required");
    }
    let store = state.store.as_ref().ok_or("unavailable")?;
    let registry = store
        .provider_accounts(owner)
        .await
        .map_err(|_| "unavailable")?;
    // Resolve ownership before consulting the gate or any Machine.
    let account = registry
        .accounts
        .iter()
        .find(|account| &account.id == account_id)
        .cloned()
        .ok_or("account_unavailable")?;
    if !state
        .service_config
        .get(&crate::config::schema::ACCOUNT_MACHINE_BROWSER)
    {
        return Err("machine_browser_not_enabled");
    }
    if !account.available() {
        return Err("account_unavailable");
    }
    if !account.browser_enabled {
        return Err("browser_not_enabled");
    }
    Ok(())
}

/// The Machine an account's browser runs on: its existing placement when it
/// is connected and capable, else a Machine holding the account's native
/// credential generation. Nothing else may host it.
async fn machine_for(
    state: &AppState,
    owner: &str,
    account_id: &ProviderAccountId,
) -> Result<(String, ConnectionToken), &'static str> {
    let store = state.store.as_ref().ok_or("unavailable")?;
    let mut candidates: Vec<String> = store
        .browser_placements(owner, account_id)
        .await
        .map_err(|_| "unavailable")?
        .into_iter()
        .map(|placement| placement.machine_id)
        .collect();
    for machine in store
        .native_generation_machines(owner, account_id)
        .await
        .map_err(|_| "unavailable")?
    {
        if !candidates.contains(&machine) {
            candidates.push(machine);
        }
    }
    let mut seen_capable = false;
    for machine in candidates {
        let Some((connection, protocol)) = state.machine_control.connection_protocol(&machine)
        else {
            continue;
        };
        if protocol < MACHINE_BROWSER_PROTOCOL_VERSION {
            continue;
        }
        match state.browsers.capability(&machine) {
            Some(capability) if capability.available => return Ok((machine, connection)),
            Some(_) => seen_capable = true,
            None => {}
        }
    }
    Err(if seen_capable {
        "browser_host_requirements"
    } else {
        "machine_unavailable"
    })
}

fn connection_for(state: &AppState, machine_id: &str) -> Result<ConnectionToken, &'static str> {
    let (connection, protocol) = state
        .machine_control
        .connection_protocol(machine_id)
        .ok_or("machine_unavailable")?;
    if protocol < MACHINE_BROWSER_PROTOCOL_VERSION {
        return Err("machine_unavailable");
    }
    Ok(connection)
}

/// Stable per-account instance id: one browser per account and Machine.
fn instance_id(owner: &str, account_id: &ProviderAccountId, machine_id: &str) -> String {
    let digest = crate::admin::hex_sha256(
        format!("{owner}:{}:{machine_id}", account_id.as_str()).as_bytes(),
    );
    format!("b-{}", &digest[..24])
}

fn state_json(machine_id: &str, event: &StateEvent) -> Value {
    json!({"machine_id": machine_id, "state": event})
}

/// Current browser state for the owner's account: the latest Machine
/// observation, refreshed from the Machine when none is cached.
pub(in crate::server) async fn state_value(
    state: &AppState,
    owner: &str,
    account_id: &ProviderAccountId,
) -> Result<Value, &'static str> {
    let admitted = admit(state, owner, account_id).await;
    let store = state.store.as_ref().ok_or("unavailable")?;
    let placements = store
        .browser_placements(owner, account_id)
        .await
        .map_err(|_| "unavailable")?;
    let gate = match &admitted {
        Ok(_) => None,
        Err(code) => Some(*code),
    };
    if let Some(observed) = state.browsers.state(account_id.as_str()) {
        let mut value = state_json(&observed.machine_id, &observed.event);
        value["placements"] = json!(placements);
        value["admission"] = json!(gate);
        return Ok(value);
    }
    let mut value = json!({"machine_id": Value::Null, "state": Value::Null, "placements": placements, "admission": gate});
    if admitted.is_ok()
        && let Some(placement) = placements.first()
        && let Ok(connection) = connection_for(state, &placement.machine_id)
        && let Ok(event) = state
            .machine_control
            .browser_command(
                &connection,
                request_id("browser-query"),
                Command::Query {
                    account_id: account_id.clone(),
                },
                QUERY_TIMEOUT,
            )
            .await
    {
        value["machine_id"] = json!(placement.machine_id);
        value["state"] = json!(event);
    }
    Ok(value)
}

async fn record_placement(
    state: &AppState,
    owner: &str,
    account_id: &ProviderAccountId,
    machine_id: &str,
    event: &StateEvent,
) {
    let Some(store) = state.store.as_ref() else {
        return;
    };
    if matches!(event.state, BrowserState::Hibernated) && event.reason.is_some() {
        // A refused open leaves no placement.
        return;
    }
    if let Err(error) = store
        .upsert_browser_placement(
            owner,
            &Placement {
                account_id: account_id.clone(),
                machine_id: machine_id.to_owned(),
                instance_id: event.instance.instance_id.clone(),
                profile_revision: event.profile_revision.unwrap_or(1),
                last_open_ms: now_ms(),
            },
        )
        .await
    {
        tracing::warn!(%error, "browser placement not recorded");
    }
}

/// Open the account's browser as a tab on its Machine.
pub(in crate::server) async fn open_tab(
    state: &AppState,
    owner: &str,
    account_id: &ProviderAccountId,
    viewport: Viewport,
) -> Result<Value, &'static str> {
    admit(state, owner, account_id).await?;
    viewport.validate().map_err(|_| "invalid_viewport")?;
    let (machine_id, connection) = machine_for(state, owner, account_id).await?;
    let event = state
        .machine_control
        .browser_command(
            &connection,
            request_id("browser-open"),
            Command::Open {
                account_id: account_id.clone(),
                instance_id: instance_id(owner, account_id, &machine_id),
                purpose: Purpose::Tab,
                viewport,
            },
            OPEN_TIMEOUT,
        )
        .await
        .map_err(|error| {
            tracing::warn!(%error, "browser open failed");
            "browser_open_failed"
        })?;
    record_placement(state, owner, account_id, &machine_id, &event).await;
    Ok(state_json(&machine_id, &event))
}

/// Open the account's browser bound to one of the owner's native logins.
/// The Machine is the login's Machine; the URL comes from the Machine's own
/// login challenge, never from the Controller.
pub(in crate::server) async fn open_sign_in(
    state: &Arc<AppState>,
    owner: &str,
    account_id: &ProviderAccountId,
    request: &str,
    viewport: Viewport,
) -> Result<Value, &'static str> {
    admit(state, owner, account_id).await?;
    viewport.validate().map_err(|_| "invalid_viewport")?;
    let store = state.store.as_ref().ok_or("unavailable")?;
    // The owner must own a login operation naming this account and request.
    let operation = super::login::continuation::resolve(store, owner, account_id, request).await?;
    let receipt = store
        .provider_login_receipt(owner, &operation)
        .await
        .map_err(|_| "unavailable")?
        .ok_or("login_operation_not_found")?;
    let machine_id = receipt.input.machine_id.clone();
    let (connection, protocol) = state
        .machine_control
        .connection_protocol(&machine_id)
        .ok_or("machine_unavailable")?;
    if protocol < MACHINE_BROWSER_PROTOCOL_VERSION {
        return Err("machine_unavailable");
    }
    let live = state
        .provider_auth_executors
        .lock()
        .get(request)
        .filter(|executor| {
            executor.owner_user_id == owner
                && executor.account_id == *account_id
                && executor.machine_id == machine_id
                && executor.connection.same(&connection)
                && executor.active(&state.machine_control, now_ms())
        })
        .is_some();
    if !live {
        return Err("login_inactive");
    }
    if !state
        .browsers
        .capability(&machine_id)
        .is_some_and(|capability| capability.available)
    {
        return Err("machine_unavailable");
    }
    let event = state
        .machine_control
        .browser_command(
            &connection,
            request_id("browser-open"),
            Command::Open {
                account_id: account_id.clone(),
                instance_id: instance_id(owner, account_id, &machine_id),
                purpose: Purpose::SignIn {
                    login_request_id: request.to_owned(),
                },
                viewport,
            },
            OPEN_TIMEOUT,
        )
        .await
        .map_err(|error| {
            tracing::warn!(%error, "browser sign-in open failed");
            "browser_open_failed"
        })?;
    record_placement(state, owner, account_id, &machine_id, &event).await;
    Ok(state_json(&machine_id, &event))
}

/// Forward one instance-scoped command to the Machine the instance runs on.
pub(in crate::server) async fn instance_command(
    state: &AppState,
    owner: &str,
    account_id: &ProviderAccountId,
    instance: &InstanceRef,
    command: Command,
    timeout: std::time::Duration,
) -> Result<Value, &'static str> {
    admit(state, owner, account_id).await?;
    if instance.account_id != *account_id {
        return Err("invalid_argument");
    }
    let observed = state
        .browsers
        .state(account_id.as_str())
        .ok_or("browser_not_running")?;
    let connection = connection_for(state, &observed.machine_id)?;
    let event = state
        .machine_control
        .browser_command(&connection, request_id("browser"), command, timeout)
        .await
        .map_err(|error| {
            tracing::warn!(%error, "browser command failed");
            "browser_command_failed"
        })?;
    Ok(state_json(&observed.machine_id, &event))
}

pub(in crate::server) async fn reset(
    state: &AppState,
    owner: &str,
    account_id: &ProviderAccountId,
    expected_profile_revision: u64,
) -> Result<Value, &'static str> {
    admit(state, owner, account_id).await?;
    let machine_id = match state.browsers.state(account_id.as_str()) {
        Some(observed) => observed.machine_id,
        None => {
            let store = state.store.as_ref().ok_or("unavailable")?;
            store
                .browser_placements(owner, account_id)
                .await
                .map_err(|_| "unavailable")?
                .into_iter()
                .next()
                .map(|placement| placement.machine_id)
                .ok_or("browser_not_placed")?
        }
    };
    let connection = connection_for(state, &machine_id)?;
    let event = state
        .machine_control
        .browser_command(
            &connection,
            request_id("browser-reset"),
            Command::Reset {
                account_id: account_id.clone(),
                expected_profile_revision,
            },
            RESET_TIMEOUT,
        )
        .await
        .map_err(|error| {
            tracing::warn!(%error, "browser reset failed");
            "browser_command_failed"
        })?;
    if let (Some(store), Some(revision)) = (state.store.as_ref(), event.profile_revision)
        && event.reason != Some(crate::machine_protocol::machine_browser::Reason::Superseded)
    {
        let _ = store
            .upsert_browser_placement(
                owner,
                &Placement {
                    account_id: account_id.clone(),
                    machine_id: machine_id.clone(),
                    instance_id: instance_id(owner, account_id, &machine_id),
                    profile_revision: revision,
                    last_open_ms: now_ms(),
                },
            )
            .await;
    }
    Ok(state_json(&machine_id, &event))
}

/// Hibernate whatever runs for the account, without the admission checks:
/// used when the owner disables the browser or revokes the account, and by
/// memory reclaim.
pub(in crate::server) async fn hibernate_running(
    state: &AppState,
    account_id: &ProviderAccountId,
) -> Result<Option<StateEvent>, String> {
    let Some(observed) = state.browsers.state(account_id.as_str()) else {
        return Ok(None);
    };
    if !observed.running() {
        return Ok(None);
    }
    let connection = connection_for(state, &observed.machine_id).map_err(str::to_owned)?;
    state
        .machine_control
        .browser_command(
            &connection,
            request_id("browser-hibernate"),
            Command::Hibernate {
                instance: observed.instance(),
            },
            HIBERNATE_TIMEOUT,
        )
        .await
        .map(Some)
}

fn error_status(code: &str) -> StatusCode {
    match code {
        "unavailable" => StatusCode::SERVICE_UNAVAILABLE,
        "account_unavailable" | "login_operation_not_found" | "browser_not_placed" => {
            StatusCode::NOT_FOUND
        }
        "invalid_argument"
        | "invalid_viewport"
        | "invalid_login_reference"
        | "named_account_required" => StatusCode::BAD_REQUEST,
        _ => StatusCode::CONFLICT,
    }
}

fn respond(result: Result<Value, &'static str>) -> Response {
    match result {
        Ok(value) => Json(value).into_response(),
        Err(code) => (error_status(code), Json(json!({"error":{"code":code}}))).into_response(),
    }
}

fn dataset_ok(state: &AppState, auth: &AuthenticatedProductRequest, dataset: Option<&str>) -> bool {
    dataset.is_none_or(|dataset| sync_dataset::matches(&state.service_id, &auth.principal, dataset))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::server) struct OpenBody {
    dataset: Option<String>,
    width: u32,
    height: u32,
    scale: u8,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::server) struct InstanceBody {
    dataset: Option<String>,
    instance: InstanceRef,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::server) struct NavigateBody {
    dataset: Option<String>,
    instance: InstanceRef,
    url: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::server) struct ResizeBody {
    dataset: Option<String>,
    instance: InstanceRef,
    attachment_id: String,
    width: u32,
    height: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::server) struct ResetBody {
    dataset: Option<String>,
    expected_profile_revision: u64,
}

pub(in crate::server) async fn http_state(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(account): Path<ProviderAccountId>,
) -> Response {
    respond(state_value(&state, &auth.principal.user_id, &account).await)
}

pub(in crate::server) async fn http_open(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(account): Path<ProviderAccountId>,
    Json(body): Json<OpenBody>,
) -> Response {
    if !dataset_ok(&state, &auth, body.dataset.as_deref()) {
        return respond(Err("dataset_changed"));
    }
    respond(
        open_tab(
            &state,
            &auth.principal.user_id,
            &account,
            Viewport {
                width: body.width,
                height: body.height,
                scale: body.scale,
            },
        )
        .await,
    )
}

pub(in crate::server) async fn http_open_sign_in(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path((account, request)): Path<(ProviderAccountId, String)>,
    Json(body): Json<OpenBody>,
) -> Response {
    if !dataset_ok(&state, &auth, body.dataset.as_deref()) {
        return respond(Err("dataset_changed"));
    }
    respond(
        open_sign_in(
            &state,
            &auth.principal.user_id,
            &account,
            &request,
            Viewport {
                width: body.width,
                height: body.height,
                scale: body.scale,
            },
        )
        .await,
    )
}

pub(in crate::server) async fn http_navigate(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(account): Path<ProviderAccountId>,
    Json(body): Json<NavigateBody>,
) -> Response {
    if !dataset_ok(&state, &auth, body.dataset.as_deref()) {
        return respond(Err("dataset_changed"));
    }
    if crate::machine_protocol::machine_browser::validate_url(&body.url).is_err() {
        return respond(Err("invalid_argument"));
    }
    respond(
        instance_command(
            &state,
            &auth.principal.user_id,
            &account,
            &body.instance,
            Command::Navigate {
                instance: body.instance.clone(),
                url: body.url,
            },
            COMMAND_TIMEOUT,
        )
        .await,
    )
}

pub(in crate::server) async fn http_resize(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(account): Path<ProviderAccountId>,
    Json(body): Json<ResizeBody>,
) -> Response {
    if !dataset_ok(&state, &auth, body.dataset.as_deref()) {
        return respond(Err("dataset_changed"));
    }
    respond(
        instance_command(
            &state,
            &auth.principal.user_id,
            &account,
            &body.instance,
            Command::Resize {
                instance: body.instance.clone(),
                attachment_id: body.attachment_id,
                width: body.width,
                height: body.height,
            },
            COMMAND_TIMEOUT,
        )
        .await,
    )
}

pub(in crate::server) async fn http_snapshot(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(account): Path<ProviderAccountId>,
    Json(body): Json<InstanceBody>,
) -> Response {
    if !dataset_ok(&state, &auth, body.dataset.as_deref()) {
        return respond(Err("dataset_changed"));
    }
    respond(
        instance_command(
            &state,
            &auth.principal.user_id,
            &account,
            &body.instance,
            Command::Snapshot {
                instance: body.instance.clone(),
            },
            HIBERNATE_TIMEOUT,
        )
        .await,
    )
}

pub(in crate::server) async fn http_hibernate(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(account): Path<ProviderAccountId>,
    Json(body): Json<InstanceBody>,
) -> Response {
    if !dataset_ok(&state, &auth, body.dataset.as_deref()) {
        return respond(Err("dataset_changed"));
    }
    respond(
        instance_command(
            &state,
            &auth.principal.user_id,
            &account,
            &body.instance,
            Command::Hibernate {
                instance: body.instance.clone(),
            },
            HIBERNATE_TIMEOUT,
        )
        .await,
    )
}

pub(in crate::server) async fn http_reset(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(account): Path<ProviderAccountId>,
    Json(body): Json<ResetBody>,
) -> Response {
    if !dataset_ok(&state, &auth, body.dataset.as_deref()) {
        return respond(Err("dataset_changed"));
    }
    respond(
        reset(
            &state,
            &auth.principal.user_id,
            &account,
            body.expected_profile_revision,
        )
        .await,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_ids_are_stable_per_owner_account_and_machine() {
        let account: ProviderAccountId = uuid::Uuid::new_v4().to_string().try_into().unwrap();
        let first = instance_id("owner", &account, "ovh");
        assert_eq!(first, instance_id("owner", &account, "ovh"));
        assert_ne!(first, instance_id("other", &account, "ovh"));
        assert_ne!(first, instance_id("owner", &account, "hawk"));
        assert!(first.starts_with("b-") && first.len() == 26);
        assert!(
            first
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        );
    }

    #[test]
    fn error_codes_map_to_their_http_status() {
        assert_eq!(error_status("unavailable"), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(error_status("account_unavailable"), StatusCode::NOT_FOUND);
        assert_eq!(error_status("invalid_viewport"), StatusCode::BAD_REQUEST);
        assert_eq!(
            error_status("machine_browser_not_enabled"),
            StatusCode::CONFLICT
        );
        assert_eq!(error_status("browser_attached"), StatusCode::CONFLICT);
    }
}
