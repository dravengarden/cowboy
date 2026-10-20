//! Owner-scoped native login polling uses durable completion, not event history.
use super::*;
use crate::machine_protocol::{AuthState, MachineEvent};

pub(in crate::server) async fn poll(
    state: &AppState,
    owner: &str,
    provider: &str,
    request: &str,
    executor: Option<&ProviderAuthExecutor>,
) -> Response {
    let result = async {
        let store = state.store.as_ref().ok_or("unavailable")?;
        let operation = store
            .provider_login_operation_for_request(owner, request)
            .await
            .map_err(|_| "unavailable")?
            .ok_or("login_operation_not_found")?;
        let receipt = store
            .provider_login_receipt(owner, &operation)
            .await
            .map_err(|_| "unavailable")?
            .ok_or("login_operation_not_found")?;
        if provider != "claude-code"
            || receipt.provider_id != provider
            || receipt.native_generation.is_none()
            || receipt.input.account_id.is_default()
        {
            return Err("login_operation_not_found");
        }
        let value = operation_receipt(state, owner, &operation).await?;
        let status = value
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let events = project(
            request,
            provider,
            status,
            executor.is_some_and(|executor| executor.active(&state.machine_control, now_ms())),
            state.machine_control.events(&receipt.input.machine_id),
        );
        Ok::<_, &'static str>(json!({"request_id":request, "status":status, "events":events}))
    }
    .await;
    if matches!(result, Err("login_operation_not_found")) {
        return (
            StatusCode::NOT_FOUND,
            "authentication request is not active",
        )
            .into_response();
    }
    operation_response(result)
}

fn project(
    request: &str,
    provider: &str,
    status: &str,
    active: bool,
    events: Vec<MachineEvent>,
) -> Vec<MachineEvent> {
    if status == "completed" {
        return vec![MachineEvent::LoginState {
            request_id: request.into(),
            provider: provider.into(),
            state: AuthState::SignedIn,
            account_label: None,
            detail: Some("Native login completed; credentials remain on this Machine".into()),
        }];
    }
    if !active || !matches!(status, "active" | "unknown") {
        return Vec::new();
    }
    events
        .into_iter()
        .filter(|event| match event {
            MachineEvent::LoginChallenge {
                request_id,
                provider: source,
                ..
            }
            | MachineEvent::LoginState {
                request_id,
                provider: source,
                state: AuthState::Pending | AuthState::Error,
                ..
            } => request_id == request && source == provider,
            MachineEvent::CommandResult {
                request_id,
                accepted: false,
                ..
            } => request_id == request,
            _ => false,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending(request: &str) -> MachineEvent {
        MachineEvent::LoginState {
            request_id: request.into(),
            provider: "claude-code".into(),
            state: AuthState::Pending,
            account_label: None,
            detail: None,
        }
    }

    #[test]
    fn native_rejection_is_visible_only_for_the_current_active_request() {
        let rejected = |request: &str| MachineEvent::CommandResult {
            request_id: request.into(),
            accepted: false,
            detail: Some("native login unavailable".into()),
        };
        let projected = project(
            "login",
            "claude-code",
            "active",
            true,
            vec![rejected("other"), rejected("login")],
        );
        assert!(
            matches!(&projected[..], [MachineEvent::CommandResult { request_id, accepted: false, .. }] if request_id == "login")
        );
        for (status, active) in [("active", false), ("cancellation_requested", true)] {
            assert!(
                project(
                    "login",
                    "claude-code",
                    status,
                    active,
                    vec![rejected("login")]
                )
                .is_empty()
            );
        }
    }

    #[test]
    fn durable_completion_replaces_pending_history_without_live_executor() {
        let events = project(
            "login",
            "claude-code",
            "completed",
            false,
            vec![pending("login")],
        );
        assert!(matches!(
            &events[..],
            [MachineEvent::LoginState {
                state: AuthState::SignedIn,
                ..
            }]
        ));
        for status in [
            "binding_pending",
            "binding_unavailable",
            "cancellation_requested",
            "not_started",
        ] {
            assert!(
                project("login", "claude-code", status, true, vec![pending("login")]).is_empty()
            );
        }
        assert!(
            project(
                "login",
                "claude-code",
                "active",
                false,
                vec![pending("login")]
            )
            .is_empty()
        );
        assert!(
            project(
                "login",
                "claude-code",
                "active",
                true,
                vec![pending("other")]
            )
            .is_empty()
        );
        assert_eq!(
            project(
                "login",
                "claude-code",
                "active",
                true,
                vec![pending("login")]
            )
            .len(),
            1
        );
    }
}
