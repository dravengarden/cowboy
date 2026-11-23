//! Process-local Cardea execution grant consumed by `operator_approval`.
use super::*;
use crate::machine_protocol::{DesiredPlugin, plugin_install::InstallTarget};
use crate::operation_budget::{OperationBudget, TimeSample};
use crate::plugin_operation::Actor;
use ::cardea_authorization::{Error, ExecutionContext};
use cardea_core::authorization::{self as wire, Plan};

pub const ACTION: &str = "plugin.install";
pub type AuthResult<T> = ::cardea_authorization::Result<T>;

pub fn unavailable<T>(_: T) -> Error {
    Error::Unavailable
}

pub fn version(value: &serde_json::Value) -> String {
    crate::admin::hex_sha256(wire::mutation_digest(value).as_bytes())
}

pub fn revision(
    hub: &Hub,
    user: &crate::store::ProductUser,
    provider: &crate::oidc::OidcProvider,
) -> AuthResult<String> {
    Ok(version(
        &serde_json::json!({"policy":permission_policy(hub),"user_id":user.id,"account":provider.account(),"provider":provider.cardea_device_configuration().map_err(unavailable)?}),
    ))
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub machine: String,
    pub plugin: String,
    pub version: String,
    pub digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<InstallTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub envelope_digest: Option<String>,
}

pub fn decode_input(p: &Plan) -> AuthResult<Input> {
    serde_json::from_value(p.input.clone()).map_err(|_| Error::InvalidContract)
}

/// Only an SDK-verified execution context can mint this process-local authority.
/// It cannot authorize uninstall, recovery, telemetry or a different install.
pub struct Grant {
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
    pub fn capture(
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
    pub fn actor(&self) -> Actor {
        self.actor.clone()
    }
    pub fn matches_service(&self, service: &str) -> bool {
        self.service_scope == version(&serde_json::json!(service))
    }
    pub fn expires_at_ms(&self) -> i64 {
        self.expiry_ms
    }
    pub fn dispatch_current(&self) -> bool {
        !self.start.expired() && self.continuation_current()
    }
    #[cfg(any(test, feature = "test-support"))]
    pub fn expire_start_for_test(&self) {
        self.start.expire_for_test();
    }
    pub fn continuation_current(&self) -> bool {
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
    pub fn matches(
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
    pub async fn current(&self, auth: ProductRequestAuth<'_>) -> Option<Actor> {
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
