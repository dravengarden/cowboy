//! Owner-operated Machine browsers for named accounts: admission (Service
//! gate, account enablement), Machine selection, the lifecycle commands the
//! owner may issue, durable placements and the state read shared by HTTP and
//! MCP. The display stream itself is `viewer` and `machine_stream`.
//! See `docs/machine-browser.md`.

use super::*;
use crate::machine_control::ConnectionToken;
use crate::machine_protocol::MACHINE_BROWSER_PROTOCOL_VERSION;
use crate::machine_protocol::machine_browser::{
    Command, InstanceRef, Purpose, Reason, State as BrowserState, StateEvent, Viewport,
};
use crate::provider_account::ProviderAccountId;
use crate::store::provider_accounts::Account;
use crate::store::provider_accounts::browser_placements::Refusal;
use coordinator::Observed;
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

/// Routes the Machines' browser observations to the coordinator. They name
/// accounts, so the shared diagnostics history never consumes them.
pub(in crate::server) fn install_observer(state: &Arc<AppState>) {
    let weak = Arc::downgrade(state);
    state
        .machine_control
        .set_browser_observer(Arc::new(move |connection, event| {
            if let Some(state) = weak.upgrade() {
                state.browsers.observe(connection, event);
            }
        }));
}

/// What a browser operation needs beyond ownership of the account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::server) enum Authority {
    /// Start or drive a browser: open, attach, navigate, resize, snapshot.
    /// Needs the Service gate, an available account and its browser enabled.
    Launch,
    /// Stop a running browser. Always the owner's to do, so a closed gate
    /// or a disabled account never keeps an instance running.
    Stop,
    /// Delete the retained profile. Allowed while either gate is closed;
    /// refused only while the account is being revoked, whose own path
    /// deletes every profile.
    Reset,
}

/// The owner's named account. Resolved before any gate or Machine is
/// consulted, so another owner's account is indistinguishable from none.
async fn owned_account(
    state: &AppState,
    owner: &str,
    account_id: &ProviderAccountId,
) -> Result<Account, &'static str> {
    if account_id.is_default() {
        return Err("named_account_required");
    }
    let store = state.store.as_ref().ok_or("unavailable")?;
    let registry = store
        .provider_accounts(owner)
        .await
        .map_err(|_| "unavailable")?;
    registry
        .accounts
        .into_iter()
        .find(|account| &account.id == account_id)
        .ok_or("account_unavailable")
}

pub(in crate::server) fn gate_open(state: &AppState) -> bool {
    state
        .service_config
        .get(&crate::config::schema::ACCOUNT_MACHINE_BROWSER)
}

/// The authority check for an owned account, given the Service gate.
fn authorize_account(
    authority: Authority,
    gate_open: bool,
    account: &Account,
) -> Result<(), &'static str> {
    match authority {
        Authority::Launch => {
            if !gate_open {
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
        Authority::Stop => Ok(()),
        Authority::Reset if account.revoking => Err("account_revoking"),
        Authority::Reset => Ok(()),
    }
}

pub(in crate::server) async fn authorize(
    state: &AppState,
    owner: &str,
    account_id: &ProviderAccountId,
    authority: Authority,
) -> Result<(), &'static str> {
    let account = owned_account(state, owner, account_id).await?;
    authorize_account(authority, gate_open(state), &account)
}

/// Whether the owner may start or drive a browser for the account:
/// ownership, the Service gate, the account's availability and its browser
/// enablement.
pub(in crate::server) async fn admit(
    state: &AppState,
    owner: &str,
    account_id: &ProviderAccountId,
) -> Result<(), &'static str> {
    authorize(state, owner, account_id, Authority::Launch).await
}

/// The account's latest observation, only while the connection that
/// reported it is still the Machine's current one.
pub(in crate::server) fn current_observation(
    state: &AppState,
    account_id: &ProviderAccountId,
) -> Option<Observed> {
    state
        .browsers
        .state(account_id.as_str())
        .filter(|observed| state.machine_control.is_current(&observed.connection))
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

/// Ask a Machine for the account's browser. Its answer is authoritative,
/// including "nothing runs", and replaces the cached observation.
async fn query_machine(
    state: &AppState,
    machine_id: &str,
    account_id: &ProviderAccountId,
) -> Option<StateEvent> {
    let connection = connection_for(state, machine_id).ok()?;
    let event = state
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
        .ok()?;
    state.browsers.record_authoritative(&connection, &event);
    Some(event)
}

/// Current browser state for the owner's account: a live instance observed
/// on its Machine's current connection, else the Machine's own answer for
/// the account's placement. Readable while the gates are closed, so the
/// owner can still stop or reset what is there.
pub(in crate::server) async fn state_value(
    state: &AppState,
    owner: &str,
    account_id: &ProviderAccountId,
) -> Result<Value, &'static str> {
    let account = owned_account(state, owner, account_id).await?;
    let gate = authorize_account(Authority::Launch, gate_open(state), &account).err();
    let store = state.store.as_ref().ok_or("unavailable")?;
    let placements = store
        .browser_placements(owner, account_id)
        .await
        .map_err(|_| "unavailable")?;
    let observed = current_observation(state, account_id);
    let mut value = json!({"machine_id": Value::Null, "state": Value::Null, "placements": placements, "admission": gate});
    if let Some(observed) = &observed {
        value["machine_id"] = json!(observed.machine_id);
        value["state"] = json!(observed.event);
        if observed.live() {
            return Ok(value);
        }
    }
    let machine = observed.map(|observed| observed.machine_id).or_else(|| {
        placements
            .first()
            .map(|placement| placement.machine_id.clone())
    });
    if let Some(machine) = machine
        && let Some(event) = query_machine(state, &machine, account_id).await
    {
        value["machine_id"] = json!(machine);
        value["state"] = json!(event);
    }
    Ok(value)
}

/// Re-learn the browsers a reconnected Machine holds: every durable
/// placement on it is queried, so the cache reflects this connection only.
pub(in crate::server) async fn refresh_placements(
    state: Arc<AppState>,
    connection: ConnectionToken,
) {
    let Some(store) = state.store.as_ref() else {
        return;
    };
    let machine_id = connection.machine_id().to_owned();
    if connection_for(&state, &machine_id).is_err() {
        return;
    }
    let placements = match store.browser_placements_on_machine(&machine_id).await {
        Ok(placements) => placements,
        Err(error) => {
            tracing::warn!(%error, machine = %machine_id, "browser placements not refreshed");
            return;
        }
    };
    for (_, account_id) in placements {
        if !state.machine_control.is_current(&connection) {
            return;
        }
        let _ = query_machine(&state, &machine_id, &account_id).await;
    }
}

/// Dispatch an open behind its durable fence. The placement is committed
/// under the owner lock first, so a revocation either refuses this open or
/// captures its Machine; the account's lifecycle lock keeps a revocation's
/// command from overtaking the open on the wire. The placement stays
/// whatever the outcome: an unanswered open may have started a browser.
/// A disable that raced the open is honoured once the open answers.
async fn dispatch_open(
    state: &AppState,
    owner: &str,
    account_id: &ProviderAccountId,
    (machine_id, connection): (&str, &ConnectionToken),
    purpose: Purpose,
    viewport: Viewport,
) -> Result<StateEvent, &'static str> {
    let store = state.store.as_ref().ok_or("unavailable")?;
    let instance = instance_id(owner, account_id, machine_id);
    let lifecycle = state.browsers.lifecycle_lock(account_id.as_str());
    let dispatch = lifecycle.lock().await;
    store
        .admit_browser_placement(owner, account_id, machine_id, &instance, now_ms())
        .await
        .map_err(|error| {
            tracing::warn!(%error, "browser placement not committed; open refused");
            "unavailable"
        })?
        .map_err(Refusal::code)?;
    let event = state
        .machine_control
        .browser_command(
            connection,
            request_id("browser-open"),
            Command::Open {
                account_id: account_id.clone(),
                instance_id: instance,
                purpose,
                viewport,
            },
            OPEN_TIMEOUT,
        )
        .await
        .map_err(|error| {
            tracing::warn!(%error, "browser open failed");
            "browser_open_failed"
        })?;
    drop(dispatch);
    if let Some(revision) = event.profile_revision
        && let Err(error) = store
            .record_browser_profile_revision(owner, account_id, machine_id, revision)
            .await
    {
        tracing::warn!(%error, "browser profile revision not recorded");
    }
    if let Err(code) = admit(state, owner, account_id).await {
        if matches!(
            event.state,
            BrowserState::Starting | BrowserState::Detached | BrowserState::Attached
        ) {
            state.browsers.cancel_viewer(account_id.as_str());
            let _ = state
                .machine_control
                .browser_command(
                    connection,
                    request_id("browser-hibernate"),
                    Command::Hibernate {
                        instance: event.instance.clone(),
                    },
                    HIBERNATE_TIMEOUT,
                )
                .await;
        }
        return Err(code);
    }
    Ok(event)
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
    let event = dispatch_open(
        state,
        owner,
        account_id,
        (&machine_id, &connection),
        Purpose::Tab,
        viewport,
    )
    .await?;
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
    let event = dispatch_open(
        state,
        owner,
        account_id,
        (&machine_id, &connection),
        Purpose::SignIn {
            login_request_id: request.to_owned(),
        },
        viewport,
    )
    .await?;
    Ok(state_json(&machine_id, &event))
}

/// Forward one instance-scoped command to the Machine the instance runs on.
pub(in crate::server) async fn instance_command(
    state: &AppState,
    owner: &str,
    (account_id, authority): (&ProviderAccountId, Authority),
    instance: &InstanceRef,
    command: Command,
    timeout: std::time::Duration,
) -> Result<Value, &'static str> {
    authorize(state, owner, account_id, authority).await?;
    if instance.account_id != *account_id {
        return Err("invalid_argument");
    }
    let observed = current_observation(state, account_id).ok_or("browser_not_running")?;
    let event = state
        .machine_control
        .browser_command(
            &observed.connection,
            request_id("browser"),
            command,
            timeout,
        )
        .await
        .map_err(|error| {
            tracing::warn!(%error, "browser command failed");
            "browser_command_failed"
        })?;
    Ok(state_json(&observed.machine_id, &event))
}

/// Delete the account's retained profile on its Machine. The owner may do
/// this while the gates are closed; only the Machine's confirmation
/// (`owner_request`) is taken as "nothing runs and nothing is retained".
pub(in crate::server) async fn reset(
    state: &AppState,
    owner: &str,
    account_id: &ProviderAccountId,
    expected_profile_revision: u64,
) -> Result<Value, &'static str> {
    authorize(state, owner, account_id, Authority::Reset).await?;
    let store = state.store.as_ref().ok_or("unavailable")?;
    let machine_id = match current_observation(state, account_id) {
        Some(observed) => observed.machine_id,
        None => store
            .browser_placements(owner, account_id)
            .await
            .map_err(|_| "unavailable")?
            .into_iter()
            .next()
            .map(|placement| placement.machine_id)
            .ok_or("browser_not_placed")?,
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
    if reset_confirmed(&event) {
        state.browsers.record_authoritative(&connection, &event);
    }
    if let Some(revision) = event.profile_revision
        && let Err(error) = store
            .record_browser_profile_revision(owner, account_id, &machine_id, revision)
            .await
    {
        tracing::warn!(%error, "browser profile revision not recorded");
    }
    Ok(state_json(&machine_id, &event))
}

/// A reset the Machine carried out: hibernated for the owner's request.
/// Busy, superseded and failed resets left whatever was there.
fn reset_confirmed(event: &StateEvent) -> bool {
    event.state == BrowserState::Hibernated && event.reason == Some(Reason::OwnerRequest)
}

/// Hibernate whatever runs for the account, without the admission checks:
/// used when the owner disables the browser or revokes the account, and by
/// memory reclaim.
pub(in crate::server) async fn hibernate_running(
    state: &AppState,
    account_id: &ProviderAccountId,
) -> Result<Option<StateEvent>, String> {
    let Some(observed) = current_observation(state, account_id) else {
        return Ok(None);
    };
    if !observed.running() {
        return Ok(None);
    }
    state
        .machine_control
        .browser_command(
            &observed.connection,
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
            (&account, Authority::Launch),
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
            (&account, Authority::Launch),
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
            (&account, Authority::Launch),
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
            (&account, Authority::Stop),
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

    fn account(enabled: bool, revoking: bool, browser_enabled: bool) -> Account {
        Account {
            id: uuid::Uuid::new_v4().to_string().try_into().unwrap(),
            provider_id: "claude-code".into(),
            label: "Test".into(),
            enabled,
            revoking,
            identity_fingerprint: None,
            admission_revision: None,
            browser_enabled,
        }
    }

    #[test]
    fn cleanup_stays_with_the_owner_while_launch_admission_is_closed() {
        let admitted = account(true, false, true);
        assert_eq!(
            authorize_account(Authority::Launch, true, &admitted),
            Ok(())
        );
        let closed = [
            (false, admitted.clone(), "machine_browser_not_enabled"),
            (true, account(true, false, false), "browser_not_enabled"),
            (true, account(false, false, true), "account_unavailable"),
        ];
        for (gate, account, code) in closed {
            assert_eq!(
                authorize_account(Authority::Launch, gate, &account),
                Err(code)
            );
            // Stopping and deleting what is retained stay possible.
            assert_eq!(authorize_account(Authority::Stop, gate, &account), Ok(()));
            assert_eq!(authorize_account(Authority::Reset, gate, &account), Ok(()));
        }
        // A revocation deletes every profile itself; stopping stays allowed.
        let revoking = account(false, true, true);
        assert_eq!(
            authorize_account(Authority::Reset, true, &revoking),
            Err("account_revoking")
        );
        assert_eq!(authorize_account(Authority::Stop, false, &revoking), Ok(()));
    }

    #[test]
    fn only_a_carried_out_reset_is_confirmed() {
        let event = |reason| StateEvent {
            request_id: Some("r".into()),
            instance: InstanceRef {
                account_id: uuid::Uuid::new_v4().to_string().try_into().unwrap(),
                instance_id: String::new(),
                incarnation: 0,
            },
            state: BrowserState::Hibernated,
            reason,
            geometry: None,
            scale: None,
            deadline_ms: None,
            attachment_id: None,
            snapshot_generation: None,
            snapshot_at_ms: None,
            profile_revision: Some(3),
            unsaved_state_lost: false,
            observed_at_ms: 1,
        };
        assert!(reset_confirmed(&event(Some(Reason::OwnerRequest))));
        for reason in [
            None,
            Some(Reason::Superseded),
            Some(Reason::Busy),
            Some(Reason::Failed),
        ] {
            assert!(!reset_confirmed(&event(reason)));
        }
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
