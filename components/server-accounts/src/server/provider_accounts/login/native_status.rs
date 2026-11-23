//! Login completion is a durable observation, never a session launch grant.
use super::*;
use crate::store::provider_accounts::logins::Receipt;

pub async fn observe(
    store: &Store,
    owner: &str,
    receipt: &Receipt,
) -> Result<Option<&'static str>, &'static str> {
    if receipt.native_generation.is_none() {
        return Ok(None);
    }
    let current = if !receipt.cancel_requested
        && receipt.native_binding_revision.is_some()
        && let Some(completion) = &receipt.native_completion
    {
        let operation = store
            .provider_login_operation_for_request(owner, &receipt.request_id)
            .await
            .map_err(|_| "unavailable")?
            .ok_or("login_operation_not_found")?;
        store
            .with_native_login_binding(owner, &operation, completion, || Ok(()))
            .await
            .is_ok()
    } else {
        false
    };
    Ok(project(receipt, current))
}

fn project(receipt: &Receipt, current: bool) -> Option<&'static str> {
    if receipt.cancel_requested {
        return Some("cancellation_requested");
    }
    if receipt.dispatch_rejection.is_some() {
        return None;
    }
    receipt.native_completion.as_ref()?;
    if receipt
        .native_logout
        .as_ref()
        .is_some_and(|logout| logout.observation.succeeded == Some(true))
    {
        return Some("signed_out");
    }
    Some(if receipt.native_binding_revision.is_none() {
        "binding_pending"
    } else if current {
        "completed"
    } else {
        "binding_unavailable"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn historical_completion_cannot_imply_current_binding_or_override_cancellation() {
        let mut receipt: Receipt = serde_json::from_value(json!({
            "request_id":"login", "provider_id":"claude-code", "native_generation":1,
            "input":{"account_id":"11111111-1111-4111-8111-111111111111","machine_id":"machine", "provider_version":"1", "generation_digest":"digest", "method":"native"}
        })).unwrap();
        assert_eq!(project(&receipt, false), None);
        receipt.native_completion = Some(serde_json::from_value(json!({
            "machine_id":"machine", "recipient_fingerprint":"fingerprint",
            "profile":{"account_id":receipt.input.account_id,"provider_id":"claude-code", "request_id":"login", "generation":1,"method":"native", "provider_version":"1", "generation_digest":"digest", "auth_contract_fingerprint":"contract", "identity":{"authority":"claude-ai","subject":"user"}}
        })).unwrap());
        assert_eq!(project(&receipt, false), Some("binding_pending"));
        receipt.native_binding_revision = Some(2);
        assert_eq!(project(&receipt, false), Some("binding_unavailable"));
        assert_eq!(project(&receipt, true), Some("completed"));
        // The kill switch's logout ended this generation.
        let mut logout = json!({
            "revocation_operation":"account-logout-1", "machine_id":"machine", "recipient_fingerprint":"fingerprint",
            "observation":{"request":{"account_id":receipt.input.account_id,"provider_id":"claude-code","generation":1,
                "login_request_id":"login","operation_id":"account-logout-1"},"succeeded":false}
        });
        receipt.native_logout = Some(serde_json::from_value(logout.clone()).unwrap());
        assert_eq!(project(&receipt, true), Some("completed"));
        logout["observation"]["succeeded"] = json!(true);
        receipt.native_logout = Some(serde_json::from_value(logout).unwrap());
        assert_eq!(project(&receipt, true), Some("signed_out"));
        receipt.cancel_requested = true;
        assert_eq!(project(&receipt, true), Some("cancellation_requested"));
    }
}
