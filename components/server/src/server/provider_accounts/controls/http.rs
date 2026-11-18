//! Owner actions on account controls (HTTP; Cowboy MCP reaches them through
//! `control.apply` `accounts.control` and `sessions.acknowledge_uncertain_effect`).
//!
//! The kill switch writes `blocked` and its stop fences durably before any
//! Machine is asked to stop; the fences then stop and confirm every unit.
//! Admission edits the Service configuration through its checked writer.
use super::*;
use crate::account_controls::OwnerAction;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(in crate::server) enum Action {
    Admit,
    Unadmit,
    Clear,
    Block,
    Unblock,
    StopUnits,
    ForceRelease,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::server) struct ControlRequest {
    action: Action,
    /// The control revision read from `accounts.list`; required to lower a
    /// state (clear, unblock), so a newer refusal is never cleared blindly.
    #[serde(default)]
    expected_revision: Option<i64>,
    /// Required by the actions that interrupt running work (block,
    /// force_release).
    #[serde(default)]
    confirm: bool,
}

fn refusal(code: &str) -> Response {
    let status = match code {
        "permission_denied" => StatusCode::FORBIDDEN,
        "account_unavailable" | "session_not_found" => StatusCode::NOT_FOUND,
        "unavailable" | "outcome_unknown" => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::CONFLICT,
    };
    (status, Json(json!({"error":{"code":code}}))).into_response()
}

pub(in crate::server) async fn http_control(
    State(state): State<Arc<AppState>>,
    Extension(auth): Extension<AuthenticatedProductRequest>,
    Path(account): Path<String>,
    Json(request): Json<ControlRequest>,
) -> Response {
    let owner = auth.principal.role == crate::admin::AdminRole::Owner;
    match apply(&state, &auth.principal.user_id, owner, &account, &request).await {
        Ok(value) => Json(value).into_response(),
        Err(code) => refusal(code),
    }
}

/// One owner action on `account`.
pub(in crate::server) async fn apply(
    state: &Arc<AppState>,
    user: &str,
    service_owner: bool,
    account: &str,
    request: &ControlRequest,
) -> Result<Value, &'static str> {
    let store = state.store.as_ref().ok_or("unavailable")?;
    let controls = of(state).ok_or("unavailable")?;
    let id: ProviderAccountId = account.parse().map_err(|_| "account_unavailable")?;
    let registry = store
        .provider_accounts(user)
        .await
        .map_err(|_| "unavailable")?;
    let card = registry
        .accounts
        .iter()
        .find(|card| card.id == id && crate::provider_account::named_only(&card.provider_id))
        .ok_or("account_unavailable")?;
    controls.refresh().await.map_err(|_| "unavailable")?;
    let control = controls
        .cache
        .control(account)
        .unwrap_or_else(|| initial(account, user, &card.provider_id));
    match request.action {
        Action::Admit | Action::Unadmit => {
            if !service_owner {
                return Err("permission_denied");
            }
            set_admitted(state, &id, request.action == Action::Admit).await?;
        }
        Action::StopUnits => controls.run_fences(),
        Action::ForceRelease => controls.force_release(&control, request.confirm).await?,
        Action::Clear | Action::Unblock | Action::Block => {
            let owner_action = match request.action {
                Action::Clear => OwnerAction::Clear,
                Action::Unblock => OwnerAction::Unblock,
                _ => OwnerAction::Block,
            };
            let next = policy::owner_transition(owner_action, control.state)?;
            if request.action == Action::Block && !request.confirm {
                return Err("confirmation_required");
            }
            if let Some(next) = next {
                // Lowering a state needs the revision the owner reviewed;
                // the kill switch applies over any revision.
                let expected = if request.action == Action::Block {
                    None
                } else {
                    Some(
                        request
                            .expected_revision
                            .ok_or("expected_revision_required")?,
                    )
                };
                let reason = match request.action {
                    Action::Block => "owner_block",
                    Action::Unblock => "owner_unblock",
                    _ => "owner_clear",
                };
                controls
                    .enter(&control, next, reason, None, expected, false)
                    .await?;
            }
        }
    }
    let registry = store
        .provider_accounts(user)
        .await
        .map_err(|_| "unavailable")?;
    let value = super::read_value(state, &registry);
    let current = value["accounts"]
        .as_array()
        .and_then(|accounts| accounts.iter().find(|entry| entry["account_id"] == account))
        .cloned()
        .unwrap_or(Value::Null);
    Ok(json!({"status":"committed","account":current}))
}

impl Controls {
    /// Release a turn slot nothing can settle. Only after every unit of the
    /// account is confirmed stopped: no turn of it can still be running.
    pub(in crate::server) async fn force_release(
        &self,
        control: &Control,
        confirm: bool,
    ) -> Result<(), &'static str> {
        if !confirm {
            return Err("confirmation_required");
        }
        if !self.stop_confirmed(&control.account_id) {
            return Err("account_stop_unconfirmed");
        }
        let turn = self
            .cache
            .turn(&control.account_id)
            .ok_or("no_account_turn")?;
        self.store
            .release_account_turn(&control.account_id, &turn.turn_id, None)
            .await
            .map_err(|_| "outcome_unknown")?;
        tracing::warn!(
            account = %control.account_id,
            session = %turn.session_id,
            "owner force-released an unsettled account turn after a confirmed stop"
        );
        self.refresh().await.map_err(|_| "unavailable")
    }

    /// Acknowledge exactly the session's current uncertain effect, durably,
    /// before its parked queue is released.
    pub(in crate::server) async fn acknowledge(
        &self,
        session: &str,
        sequence: u64,
    ) -> Result<(String, Value), &'static str> {
        if self.env.session(session).is_none() {
            return Err("session_not_found");
        }
        let (method, current) = self.uncertain(session).ok_or("no_uncertain_effect")?;
        if current != sequence {
            return Err("uncertain_effect_changed");
        }
        let key = uncertain_key(session);
        let mut record = self.env.setting(&key).unwrap_or_else(|| json!({}));
        record["observed"] = json!(record["observed"].as_u64().unwrap_or(0).max(sequence));
        record["method"] = json!(method);
        record["acknowledged"] = json!(sequence);
        record["acknowledged_at_ms"] = json!(now_ms());
        self.store
            .put_setting(&key, &record)
            .await
            .map_err(|_| "outcome_unknown")?;
        Ok((key, record))
    }
}

/// Add or remove `account` in `accounts.antigravity_admitted` through the
/// Service configuration's checked writer; the watcher applies it live.
async fn set_admitted(
    state: &AppState,
    account: &ProviderAccountId,
    admit: bool,
) -> Result<(), &'static str> {
    let path = state.service_config_path.clone();
    let id = account.as_str().to_owned();
    tokio::task::spawn_blocking(move || {
        let current = crate::config::editable(crate::config::Scope::Service, &path)
            .map_err(|_| "configuration_unavailable")?;
        let revision = current["revision"]
            .as_str()
            .ok_or("configuration_unavailable")?;
        let mut admitted: Vec<String> = current["values"]["accounts"]["antigravity_admitted"]
            .as_array()
            .map(|ids| {
                ids.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        if admitted.contains(&id) == admit {
            return Ok(());
        }
        if admit {
            admitted.push(id);
        } else {
            admitted.retain(|other| *other != id);
        }
        crate::config::patch_checked(
            crate::config::Scope::Service,
            &path,
            revision,
            &json!({"accounts.antigravity_admitted": admitted}),
            || Ok(()),
        )
        .map(drop)
        .map_err(|error| match error.to_string().as_str() {
            "revision_conflict" => "revision_conflict",
            _ => "invalid_configuration",
        })
    })
    .await
    .map_err(|_| "unavailable")?
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::server) struct Acknowledge {
    sequence: u64,
}

/// The owner reviewed the workspace after an uncertain target effect:
/// release the session's parked queue for exactly that sequence.
pub(in crate::server) async fn http_acknowledge(
    State(state): State<Arc<AppState>>,
    Path(session): Path<String>,
    Json(request): Json<Acknowledge>,
) -> Response {
    match acknowledge(&state, &session, request.sequence).await {
        Ok(value) => Json(value).into_response(),
        Err(code) => refusal(code),
    }
}

pub(in crate::server) async fn acknowledge(
    state: &Arc<AppState>,
    session: &str,
    sequence: u64,
) -> Result<Value, &'static str> {
    let (key, record) = of(state)
        .ok_or("unavailable")?
        .acknowledge(session, sequence)
        .await?;
    // Durable above; publish it, then let the parked queue drain.
    state.hub.load_settings(vec![(key, record)]);
    state.hub.retry_resource_wait(session);
    Ok(json!({"status":"committed","session_id":session,"acknowledged":sequence}))
}
