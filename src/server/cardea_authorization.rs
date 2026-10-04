//! SDK admission reuses the exact native installer, its checkpoints and journals.
use super::*;
use crate::machine_protocol::{
    DesiredPlugin,
    plugin_install::{InstallTarget, InstallTargetObservation, InstallTargetQuery},
};
use crate::operation_budget::{OperationBudget, TimeSample};
use crate::plugin_operation::{Actor, installation::InstallPhase};
use ::cardea_authorization::{
    Application, Cardea, DeviceCredential, Eligibility, Error, ExecutionContext,
    Journal as JournalContract, Recovery, Requester, Transport as TransportContract, adapter,
};
use axum::http::header::CONTENT_TYPE;
use cardea_core::authorization::{self as wire, Catalog, Command, Plan, Receipt, Status};
use tokio::runtime::Handle;

const ACTION: &str = "plugin.install";
type AuthResult<T> = ::cardea_authorization::Result<T>;
fn unavailable<T>(_: T) -> Error {
    Error::Unavailable
}
fn transport_error(error: anyhow::Error) -> Error {
    error
        .downcast_ref::<Error>()
        .copied()
        .unwrap_or(Error::Unavailable)
}
fn clock() -> AuthResult<u64> {
    u64::try_from(auth_now_ms() / 1000).map_err(|_| Error::Expired)
}
fn enabled() -> bool {
    std::env::var("COWBOY_CARDEA_OPERATIONS_ENABLED").is_ok_and(|v| v == "true")
}
fn revision(
    hub: &Hub,
    user: &crate::store::ProductUser,
    provider: &crate::oidc::OidcProvider,
) -> AuthResult<String> {
    Ok(version(
        &serde_json::json!({"policy":permission_policy(hub),"user_id":user.id,"account":provider.account(),"provider":provider.cardea_device_configuration().map_err(unavailable)?}),
    ))
}
fn version(value: &serde_json::Value) -> String {
    crate::admin::hex_sha256(wire::mutation_digest(value).as_bytes())
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    machine: String,
    plugin: String,
    version: String,
    digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    target: Option<InstallTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    envelope_digest: Option<String>,
}
fn decode_input(p: &Plan) -> AuthResult<Input> {
    serde_json::from_value(p.input.clone()).map_err(|_| Error::InvalidContract)
}
async fn operator(state: &AppState, subject: &str) -> AuthResult<crate::store::ProductUser> {
    if !state.product_auth_enabled {
        return Err(Error::PermissionDenied);
    }
    let provider = state
        .product_authentication
        .provider("cardea")
        .ok_or(Error::AuthenticationRequired)?;
    if !provider.cardea_subject_matches(subject) {
        return Err(Error::PermissionDenied);
    }
    let user = state
        .store
        .as_ref()
        .ok_or(Error::Unavailable)?
        .user_by_username(provider.account())
        .await
        .map_err(unavailable)?
        .ok_or(Error::PermissionDenied)?;
    if user.disabled_at_ms.is_some()
        || !product_principal(&state.hub, &user)
            .role
            .at_least(crate::admin::AdminRole::Operator)
    {
        return Err(Error::PermissionDenied);
    }
    Ok(user)
}
async fn requester_operator(
    state: &AppState,
    r: &Requester,
) -> AuthResult<crate::store::ProductUser> {
    let user = operator(state, r.subject_id()).await?;
    let device = state
        .product_authentication
        .provider("cardea")
        .ok_or(Error::AuthenticationRequired)?
        .cardea_product_device_id(r.grant_id())
        .map_err(unavailable)?;
    if !state
        .store
        .as_ref()
        .ok_or(Error::Unavailable)?
        .cardea_device_allowed(&user.id, &device)
        .await
        .map_err(unavailable)?
    {
        return Err(Error::PermissionDenied);
    }
    Ok(user)
}

/// Only an SDK-verified execution context can mint this process-local authority.
/// It cannot authorize uninstall, recovery, telemetry or a different install.
pub(super) struct Grant {
    input: Input,
    operation: String,
    service_scope: String,
    actor: Actor,
    policy: String,
    subject: String,
    account: String,
    device: String,
    provider_config: serde_json::Value,
    hub: Hub,
    permissions: crate::core::ProductPermissionObservation,
    expiry_ms: i64,
    budget: OperationBudget,
    start: OperationBudget,
    freeze: Arc<crate::machine_convergence::ConvergenceFreeze>,
}
impl Grant {
    fn capture(
        auth: ProductRequestAuth<'_>,
        freeze: Arc<crate::machine_convergence::ConvergenceFreeze>,
        ctx: &ExecutionContext,
        user: &crate::store::ProductUser,
    ) -> AuthResult<Self> {
        let p = ctx.plan();
        let a = ctx.admission();
        let provider = auth
            .product_authentication
            .provider("cardea")
            .ok_or(Error::AuthenticationRequired)?;
        if p.action != ACTION
            || p.resources.len() != 1
            || p.policy_revision != revision(auth.hub, user, provider)?
            || a.approver_id != p.subject_id
            || freeze.is_frozen()
        {
            return Err(Error::PermissionDenied);
        }
        let input = decode_input(p)?;
        if input.target.is_none() || input.envelope_digest.is_none() {
            return Err(Error::InvalidContract);
        }
        let expiry_ms = i64::try_from(
            ctx.requester()
                .expires_at()
                .min(p.execute_until)
                .saturating_mul(1000),
        )
        .map_err(|_| Error::Expired)?;
        let start_ms =
            i64::try_from(a.start_until.saturating_mul(1000)).map_err(|_| Error::Expired)?;
        let received = TimeSample::now();
        let permissions = auth
            .hub
            .observe_product_permissions(&user.username)
            .ok_or(Error::Unavailable)?;
        if !permissions
            .role()
            .at_least(crate::admin::AdminRole::Operator)
        {
            return Err(Error::PermissionDenied);
        }
        let grant = Self {
            input,
            operation: p.operation_id.clone(),
            service_scope: p.resources[0].scope.clone(),
            actor: Actor::Product {
                user_id: user.id.clone(),
            },
            policy: p.policy_revision.clone(),
            subject: p.subject_id.clone(),
            account: user.username.clone(),
            device: provider
                .cardea_product_device_id(ctx.requester().grant_id())
                .map_err(unavailable)?,
            hub: auth.hub.clone(),
            permissions,
            provider_config: provider
                .cardea_device_configuration()
                .map_err(unavailable)?,
            expiry_ms,
            budget: OperationBudget::new(expiry_ms, std::time::Duration::from_mins(5), received),
            start: OperationBudget::new(start_ms, std::time::Duration::from_secs(30), received),
            freeze,
        };
        if !grant.dispatch_current() {
            return Err(Error::Expired);
        }
        Ok(grant)
    }
    pub(super) fn actor(&self) -> Actor {
        self.actor.clone()
    }
    pub(super) fn matches_service(&self, service: &str) -> bool {
        self.service_scope == version(&serde_json::json!(service))
    }
    pub(super) fn expires_at_ms(&self) -> i64 {
        self.expiry_ms
    }
    pub(super) fn dispatch_current(&self) -> bool {
        !self.start.expired() && self.continuation_current()
    }
    #[cfg(test)]
    pub(super) fn expire_start_for_test(&self) {
        self.start.expire_for_test();
    }
    pub(super) fn continuation_current(&self) -> bool {
        let Actor::Product { user_id } = &self.actor else {
            return false;
        };
        !self.budget.expired()
            && !self.freeze.is_frozen()
            && self.permissions.current(&self.hub, &self.account)
            && self.policy
                == version(&serde_json::json!({
                    "policy":permission_policy(&self.hub), "user_id":user_id,
                    "account":self.account, "provider":self.provider_config,
                }))
    }
    pub(super) fn matches(
        &self,
        machine: &str,
        desired: &DesiredPlugin,
        operation: &str,
        target: &InstallTarget,
    ) -> bool {
        self.dispatch_current()
            && operation == self.operation
            && machine == self.input.machine
            && desired.release.plugin_id == self.input.plugin
            && desired.release.plugin_version == self.input.version
            && desired.release.artifact_digest == self.input.digest
            && self.input.target.as_ref() == Some(target)
            && serde_json::to_vec(desired).is_ok_and(|b| {
                self.input.envelope_digest.as_deref()
                    == Some(&format!("sha256:{}", crate::admin::hex_sha256(&b)))
            })
    }
    pub(super) async fn current(&self, auth: ProductRequestAuth<'_>) -> Option<Actor> {
        if self.budget.expired() || self.freeze.is_frozen() {
            return None;
        }
        if !auth.product_auth_enabled {
            return None;
        }
        let provider = auth.product_authentication.provider("cardea")?;
        if !provider.cardea_subject_matches(&self.subject)
            || provider.account() != self.account
            || provider.cardea_device_configuration().ok()? != self.provider_config
        {
            return None;
        }
        let Actor::Product { user_id } = &self.actor else {
            return None;
        };
        let user = auth.store?.user_by_id(user_id).await.ok()??;
        if !auth
            .store?
            .cardea_device_allowed(user_id, &self.device)
            .await
            .ok()?
        {
            return None;
        }
        (user.disabled_at_ms.is_none()
            && self.permissions.current(auth.hub, &user.username)
            && user.username == self.account
            && product_principal(auth.hub, &user)
                .role
                .at_least(crate::admin::AdminRole::Operator)
            && revision(auth.hub, &user, provider).ok()? == self.policy
            && !self.budget.expired()
            && !self.freeze.is_frozen())
        .then(|| self.actor())
    }
}

#[derive(Clone)]
struct Io {
    state: Arc<AppState>,
    runtime: Handle,
}
impl Io {
    fn store(&self) -> AuthResult<&Store> {
        self.state.store.as_ref().ok_or(Error::Unavailable)
    }
    fn provider(&self) -> AuthResult<&crate::oidc::OidcProvider> {
        self.state
            .product_authentication
            .provider("cardea")
            .map(Arc::as_ref)
            .ok_or(Error::AuthenticationRequired)
    }
    fn completed(&self, p: &Plan, claim: &str) -> AuthResult<Option<Receipt>> {
        let Some(op) = self
            .runtime
            .block_on(self.store()?.plugin_install_operation(&p.operation_id))
            .map_err(unavailable)?
        else {
            return Ok(None);
        };
        let input = decode_input(p)?;
        let user = self
            .runtime
            .block_on(operator(&self.state, &p.subject_id))?;
        let intent = &op.intent;
        if intent.service_id != self.state.service_id
            || intent.actor != (Actor::Product { user_id: user.id })
            || intent.machine_id != input.machine
            || intent.plugin_id != input.plugin
            || intent.plugin_version != input.version
            || intent.generation_digest != input.digest
            || intent.machine_target != input.target
            || input.envelope_digest.as_deref() != Some(intent.envelope_digest.as_str())
        {
            return Err(Error::Conflict);
        }
        if !matches!(
            op.phase,
            InstallPhase::Completed | InstallPhase::AuthenticationPending
        ) {
            return Ok(None);
        }
        let summary = if op.phase == InstallPhase::AuthenticationPending {
            "Plugin installed; authentication reconciliation remains pending"
        } else {
            "Plugin installation completed"
        };
        Ok(Some(Receipt {
            operation_id: p.operation_id.clone(),
            plan_digest: p.digest(),
            claim_id: claim.into(),
            outcome: Status::Succeeded,
            finished_at: u64::try_from(op.updated_at_ms / 1000)
                .map_err(|_| Error::InvalidContract)?,
            summary: summary.into(),
        }))
    }
}
struct Transport {
    io: Io,
    issuer: String,
    client: String,
}
impl TransportContract for Transport {
    fn issuer(&self) -> &str {
        &self.issuer
    }
    fn application_id(&self) -> &str {
        &self.client
    }
    fn command(&mut self, command: &Command) -> AuthResult<serde_json::Value> {
        self.io
            .runtime
            .block_on(self.io.provider()?.cardea_authorization_post(
                false,
                &serde_json::to_value(command).map_err(|_| Error::InvalidContract)?,
            ))
            .map_err(transport_error)
    }
    fn exchange_identity(&mut self, c: &DeviceCredential) -> AuthResult<serde_json::Value> {
        self.io
            .runtime
            .block_on(self.io.provider()?.cardea_authorization_post(
                true,
                &serde_json::json!({"access_token":c.access_token,"assertion":c.assertion}),
            ))
            .map_err(transport_error)
    }
}
impl Application for Io {
    fn evaluate(&self, p: &Plan, r: &Requester, _: u64) -> AuthResult<Eligibility> {
        let user = self.runtime.block_on(requester_operator(&self.state, r))?;
        if p.action != ACTION
            || self.state.convergence_freeze.is_frozen()
            || p.resources.len() != 1
            || p.resources[0].scope != version(&serde_json::json!(self.state.service_id))
        {
            return Ok(Eligibility::Denied);
        }
        Ok(Eligibility::Eligible {
            policy_revision: revision(&self.state.hub, &user, self.provider()?)?,
        })
    }
    fn approver_eligible(&self, _: &Plan, subject: &str, _: u64) -> AuthResult<bool> {
        Ok(self
            .runtime
            .block_on(operator(&self.state, subject))
            .is_ok())
    }
    fn commit(&mut self, ctx: &ExecutionContext, _: u64) -> AuthResult<String> {
        let user = self
            .runtime
            .block_on(requester_operator(&self.state, ctx.requester()))?;
        let grant = Grant::capture(
            ProductRequestAuth::from(self.state.as_ref()),
            self.state.convergence_freeze.clone(),
            ctx,
            &user,
        )?;
        let input = decode_input(ctx.plan())?;
        if super::service_managed_refusal(&self.state, &input.machine).is_some() {
            return Err(Error::PermissionDenied);
        }
        let approval =
            operator_approval::OperatorApproval::capture_cardea(&self.state.service_id, grant);
        // Existing installer owns target observation, signed release checks,
        // session leases, Machine CAS, dispatch and durable recovery evidence.
        let _response = self.runtime.block_on(plugin_install::confirmed_install(
            self.state.clone(),
            input.machine,
            input.plugin,
            PluginInstallRequest {
                operation_id: ctx.plan().operation_id.clone(),
                version: input.version,
                digest: input.digest,
            },
            approval,
        ));
        self.completed(ctx.plan(), &ctx.admission().claim_id)?
            .map(|r| r.summary)
            .ok_or(Error::OutcomeUnknown)
    }
}
struct Plans(Io);
impl adapter::Plans for Plans {
    fn load(&self, id: &str) -> AuthResult<Plan> {
        self.0
            .runtime
            .block_on(self.0.store()?.cardea_operation(id))
            .map_err(unavailable)?
            .ok_or(Error::Conflict)?
            .plan()
            .map_err(unavailable)
    }
    fn prepare(
        &mut self,
        r: &Requester,
        action: &str,
        input: serde_json::Value,
        key: &str,
        catalog: &Catalog,
        at: u64,
    ) -> AuthResult<Plan> {
        let io = &self.0;
        let user = self.0.runtime.block_on(requester_operator(&io.state, r))?;
        let policy = revision(&io.state.hub, &user, io.provider()?)?;
        if action != ACTION
            || !io.state.public_origins.contains(&catalog.adapter_origin)
            || io.state.convergence_freeze.is_frozen()
        {
            return Err(Error::PermissionDenied);
        }
        let binding = wire::mutation_digest(
            &serde_json::json!({"application":r.application_id(),"subject":r.subject_id(),"grant":r.grant_id(),"key":r.binding_key(),"action":action,"input":input,"catalog":catalog.digest()}),
        );
        if let Some(old) = io
            .runtime
            .block_on(io.store()?.cardea_request(key))
            .map_err(unavailable)?
        {
            if old.request_digest != binding {
                return Err(Error::Conflict);
            }
            return old.plan().map_err(unavailable);
        }
        let mut selected: Input =
            serde_json::from_value(input).map_err(|_| Error::InvalidContract)?;
        if selected.target.is_some()
            || selected.envelope_digest.is_some()
            || super::service_managed_refusal(&io.state, &selected.machine).is_some()
        {
            return Err(Error::InvalidContract);
        }
        let release = io
            .state
            .plugin_catalog
            .resolve_verified_exact(&selected.plugin, &selected.version, &selected.digest)
            .map_err(|_| Error::InvalidContract)?;
        if io
            .runtime
            .block_on(plugin_install_compatibility(
                &io.state,
                &selected.machine,
                release.desired(),
            ))
            .map_err(unavailable)?
            .is_some()
        {
            return Err(Error::InvalidContract);
        }
        if release.desired().release.plugin_kind == cowboy_plugin_sdk::PluginKind::AgentProvider
            && io
                .runtime
                .block_on(agent_plugin_install_compatibility(
                    &io.state,
                    &selected.machine,
                    release.desired(),
                ))
                .map_err(unavailable)?
                .is_some()
        {
            return Err(Error::InvalidContract);
        }
        let connection = io
            .state
            .machine_control
            .operation_connection(&selected.machine)
            .map_err(unavailable)?;
        let query = InstallTargetQuery {
            schema: 1,
            service_id: io.state.service_id.clone(),
            machine_id: selected.machine.clone(),
            plugin_id: selected.plugin.clone(),
        };
        let observation = io
            .runtime
            .block_on(
                io.state
                    .machine_control
                    .plugin_installation_target(&connection, &query),
            )
            .map_err(unavailable)?;
        let InstallTargetObservation::Observed {
            admission_enabled: true,
            target,
            ..
        } = observation
        else {
            return Err(Error::PermissionDenied);
        };
        selected.target = Some(target);
        selected.envelope_digest = Some(format!(
            "sha256:{}",
            crate::admin::hex_sha256(
                &serde_json::to_vec(release.desired()).map_err(|_| Error::InvalidContract)?
            )
        ));
        use rand::RngCore as _;
        let mut random = [0_u8; 32];
        rand::rngs::OsRng
            .try_fill_bytes(&mut random)
            .map_err(unavailable)?;
        let id = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random);
        let p = Plan {
            schema: wire::OPERATION_SCHEMA.into(),
            operation_id: id,
            application_id: r.application_id().into(),
            action: action.into(),
            catalog_digest: catalog.digest(),
            policy_revision: policy,
            subject_id: r.subject_id().into(),
            grant_id: r.grant_id().into(),
            binding_key: r.binding_key().into(),
            resources: vec![wire::Resource {
                kind: "machine-plugin".into(),
                id: version(&serde_json::json!([selected.machine, selected.plugin])),
                scope: version(&serde_json::json!(io.state.service_id)),
                expected_revision: version(
                    &serde_json::to_value(&selected.target).map_err(|_| Error::InvalidContract)?,
                ),
            }],
            review: cardea_core::ApprovalDisplay {
                title: "Install Cowboy Plugin".into(),
                summary: format!(
                    "Install {} {} on {}; preserve sessions bound to existing generations.",
                    selected.plugin, selected.version, selected.machine
                ),
                facts: vec![
                    cardea_core::DisplayFact {
                        label: "Artifact digest".into(),
                        value: selected.digest.clone(),
                    },
                    cardea_core::DisplayFact {
                        label: "Provider authentication".into(),
                        value: "Agent Providers synchronize current Service authentication on the selected Machine.".into(),
                    },
                ],
            },
            input: serde_json::to_value(selected).map_err(|_| Error::InvalidContract)?,
            created_at: at,
            review_until: at + 540,
            execute_until: at + 600,
        };
        if !p.validate(catalog, clock()?) {
            return Err(Error::InvalidContract);
        }
        // A concurrent same-key insert may win. Read and compare the winner;
        // the losing transaction rolls back its capacity reservation.
        let saved = io
            .runtime
            .block_on(io.store()?.prepare_cardea_operation(key, &binding, &p));
        let old = io
            .runtime
            .block_on(io.store()?.cardea_request(key))
            .map_err(unavailable)?
            .ok_or_else(|| saved.err().map(unavailable).unwrap_or(Error::Unavailable))?;
        if old.request_digest != binding {
            return Err(Error::Conflict);
        }
        old.plan().map_err(unavailable)
    }
}
struct Journal(Io);
impl JournalContract for Journal {
    fn prepare_claim(&mut self, p: &Plan, key: &str) -> AuthResult<Recovery> {
        let saved = self
            .0
            .runtime
            .block_on(self.0.store()?.claim_cardea_operation(&p.operation_id, key))
            .map_err(|_| Error::Conflict)?;
        if saved.plan().map_err(unavailable)? != *p {
            return Err(Error::Conflict);
        }
        if let Some(document) = saved.receipt {
            return Ok(Recovery::Completed(
                wire::decode(document.as_bytes()).map_err(|_| Error::InvalidContract)?,
            ));
        }
        if saved.state == "prepared" {
            return Ok(Recovery::Ready);
        }
        if let Some(receipt) = self.0.completed(p, key)? {
            self.0
                .runtime
                .block_on(self.0.store()?.record_cardea_operation(&receipt))
                .map_err(unavailable)?;
            return Ok(Recovery::Completed(receipt));
        }
        Ok(Recovery::Uncertain)
    }
    fn start_effect(&mut self, p: &Plan, a: &wire::Admission, _: u64) -> AuthResult<bool> {
        self.0
            .runtime
            .block_on(
                self.0
                    .store()?
                    .start_cardea_operation(&p.operation_id, &a.claim_id),
            )
            .map_err(unavailable)
    }
    fn record(&mut self, r: &Receipt) -> AuthResult<()> {
        // Preserve a pending installation for later receipt reconciliation.
        // Unknown is never a completed journal or authority to dispatch again.
        if r.outcome == Status::OutcomeUnknown {
            return Err(Error::OutcomeUnknown);
        }
        self.0
            .runtime
            .block_on(self.0.store()?.record_cardea_operation(r))
            .map_err(unavailable)
    }
}
pub(super) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/cardea/v1/capabilities", get(capabilities))
        .route(
            "/cardea/v1/{operation}",
            post(handle).layer(DefaultBodyLimit::max(wire::MAX_DOCUMENT_BYTES)),
        )
        .layer(middleware::from_fn(no_store))
}
async fn no_store(request: axum::extract::Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response
}
#[derive(Default)]
struct Window {
    minute: u64,
    count: u32,
}
impl Window {
    fn admit(&mut self, at: u64) -> bool {
        let minute = at / 60;
        if minute < self.minute {
            return false;
        }
        if minute > self.minute {
            self.minute = minute;
            self.count = 0;
        }
        if self.count >= 120 {
            return false;
        }
        self.count += 1;
        true
    }
}
async fn capabilities(State(state): State<Arc<AppState>>) -> Response {
    if !enabled() || !state.product_auth_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(provider) = state.product_authentication.provider("cardea") else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    match provider.cardea_device_configuration() {Ok(config)=>Json(serde_json::json!({"schema":"dravengarden.cowboy.cardea-capabilities/v1","application_id":config["client_id"],"protocols":["authorization/v1"],"actions":[ACTION]})).into_response(),Err(_)=>StatusCode::SERVICE_UNAVAILABLE.into_response()}
}
async fn handle(
    State(state): State<Arc<AppState>>,
    Path(operation): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !enabled() || !state.product_auth_enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    if !matches!(
        operation.as_str(),
        "prepare" | "request" | "execute" | "receipt"
    ) || headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_none_or(|v| v.split(';').next() != Some("application/json"))
    {
        return StatusCode::BAD_REQUEST.into_response();
    }
    static REQUESTS: std::sync::OnceLock<parking_lot::Mutex<Window>> = std::sync::OnceLock::new();
    if !clock().is_ok_and(|at| {
        REQUESTS
            .get_or_init(|| parking_lot::Mutex::new(Window::default()))
            .lock()
            .admit(at)
    }) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    // Do not queue unbounded blocking tasks. A disconnected HTTP requester does
    // not cancel the durable installer; its original ID remains observable.
    static SLOTS: std::sync::OnceLock<Arc<tokio::sync::Semaphore>> = std::sync::OnceLock::new();
    let Ok(slot) = SLOTS
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(8)))
        .clone()
        .try_acquire_owned()
    else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    let Some(provider) = state.product_authentication.provider("cardea") else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let config = match provider.cardea_device_configuration() {
        Ok(config) => config,
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    let io = Io {
        state,
        runtime: Handle::current(),
    };
    let path = format!("/cardea/v1/{operation}");
    let result = tokio::task::spawn_blocking(move || {
        let _slot = slot;
        let transport = Transport {
            io: io.clone(),
            issuer: config["issuer"]
                .as_str()
                .ok_or(Error::InvalidContract)?
                .into(),
            client: config["client_id"]
                .as_str()
                .ok_or(Error::InvalidContract)?
                .into(),
        };
        let mut adapter = adapter::Adapter {
            cardea: Cardea::new(transport),
            app: io.clone(),
            plans: Plans(io.clone()),
            journal: Journal(io),
        };
        adapter.handle(&path, &body, &clock)
    })
    .await;
    let response = match result {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(error)) => {
            let status = match error {
                Error::AuthenticationRequired => StatusCode::UNAUTHORIZED,
                Error::PermissionDenied => StatusCode::FORBIDDEN,
                Error::ApprovalRequired => StatusCode::PRECONDITION_REQUIRED,
                Error::Conflict | Error::Expired => StatusCode::CONFLICT,
                Error::InvalidContract => StatusCode::BAD_REQUEST,
                _ => StatusCode::SERVICE_UNAVAILABLE,
            };
            (status,Json(serde_json::json!({"error":if matches!(error,Error::OutcomeUnknown) {"outcome_unknown"}else{"authorization_rejected"}}))).into_response()
        }
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error":"outcome_unknown"})),
        )
            .into_response(),
    };
    let mut response = response;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response
}

#[cfg(test)]
mod tests;
