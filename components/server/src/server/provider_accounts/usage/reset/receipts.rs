//! Receipts of manual named-account resets, keyed by the caller's stable
//! operation ID. The spend runs in a Service-owned task, so a caller that
//! disconnects or retries never loses the terminal (or unknown) receipt and
//! never starts a second spend: the same operation ID maps to the same
//! idempotency key, whose receipt is the running attempt here, its finished
//! result, or, after a restart, the action log (an attempt that only started
//! is unknown).
use super::*;
use tokio::sync::watch;

/// HTTP status and JSON body of one reset attempt.
pub(super) type Receipt = (StatusCode, Value);

type Flights = std::collections::BTreeMap<String, watch::Receiver<Option<Receipt>>>;

static FLIGHTS: std::sync::LazyLock<parking_lot::Mutex<Flights>> =
    std::sync::LazyLock::new(Default::default);

/// Bound on remembered finished receipts; the action log covers older ones.
const RETAINED: usize = 256;
/// How long a repeated request waits for the original attempt to finish.
const WAIT: std::time::Duration = std::time::Duration::from_secs(60);

fn flight(lane: &str, key: &str) -> String {
    format!("{lane}/{key}")
}

pub(super) fn unknown(message: &str) -> Receipt {
    (
        StatusCode::CONFLICT,
        json!({"error":{"code":"reset_outcome_unknown","message":message}}),
    )
}

/// The idempotency key of one caller operation: stable for the same owner,
/// account lane and operation ID, and a valid Provider request ID.
pub(super) fn operation_key(
    owner: &str,
    lane: &str,
    operation: &str,
) -> Result<String, &'static str> {
    if !(8..=128).contains(&operation.len())
        || !operation
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("reset_operation_invalid");
    }
    let digest = crate::admin::hex_sha256(
        &serde_json::to_vec(&(owner, lane, operation)).expect("strings serialize"),
    );
    Ok(format!("cowboy-reset-op-{}", &digest[..40]))
}

/// The receipt of an earlier attempt with this key, if any.
pub(super) async fn existing(
    store: &crate::store::Store,
    lane: &str,
    key: &str,
) -> Option<Receipt> {
    let pending = FLIGHTS.lock().get(&flight(lane, key)).cloned();
    if let Some(mut receipt) = pending {
        let _ = tokio::time::timeout(WAIT, receipt.wait_for(Option::is_some)).await;
        let current = receipt.borrow().clone();
        return Some(match current {
            Some(receipt) => receipt,
            None => (
                StatusCode::CONFLICT,
                json!({"error":{"code":"reset_in_progress","message":error_message("reset_in_progress")}}),
            ),
        });
    }
    recorded(store, lane, key).await
}

/// The newest manual action-log row of this key, as a receipt.
async fn recorded(store: &crate::store::Store, lane: &str, key: &str) -> Option<Receipt> {
    let suffix: String = key
        .chars()
        .rev()
        .take(8)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    let logs = store.provider_action_logs(200).await.ok()?;
    let row = logs.iter().find(|row| {
        row.provider == lane
            && row.trigger == "manual"
            && row.idempotency_suffix.as_deref() == Some(suffix.as_str())
    })?;
    Some(match row.status.as_str() {
        "succeeded" => (
            StatusCode::OK,
            json!({"outcome":row.message,"credit_id":row.credit_id}),
        ),
        "failed" => (
            StatusCode::CONFLICT,
            json!({"error":{"code":"reset_failed","message":row.message,"credit_id":row.credit_id}}),
        ),
        // `unknown`, or `started` with no terminal row: interrupted.
        _ => unknown(error_message("native_reset_outcome_unknown")),
    })
}

/// Register an attempt of this key; `None` if one is already registered.
pub(super) fn begin(lane: &str, key: &str) -> Option<watch::Sender<Option<Receipt>>> {
    let mut flights = FLIGHTS.lock();
    let id = flight(lane, key);
    if flights.contains_key(&id) {
        return None;
    }
    if flights.len() >= RETAINED {
        flights.retain(|_, receipt| receipt.borrow().is_none());
    }
    let (sender, receiver) = watch::channel(None);
    flights.insert(id, receiver);
    Some(sender)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_operation_maps_to_one_valid_provider_request_id() {
        let key = operation_key("owner", "account-x", "op-12345678").unwrap();
        assert_eq!(
            key,
            operation_key("owner", "account-x", "op-12345678").unwrap()
        );
        assert_ne!(
            key,
            operation_key("other", "account-x", "op-12345678").unwrap()
        );
        let request = crate::machine_protocol::NativeResetRequest {
            provider: "openai".into(),
            idempotency_key: key,
            expected_credit_id: None,
        };
        assert!(request.valid());
        for invalid in ["short", "has space here", &"x".repeat(129)] {
            assert_eq!(
                operation_key("owner", "account-x", invalid),
                Err("reset_operation_invalid")
            );
        }
    }

    #[tokio::test]
    async fn a_repeated_operation_returns_the_original_receipt_and_never_spends_again() {
        let root = tempfile::tempdir().unwrap();
        let store = crate::store::Store::connect("sqlite::memory:", root.path().join("artifacts"))
            .await
            .unwrap();
        store.migrate().await.unwrap();
        let lane = "account-11111111-1111-4111-8111-111111111111";
        let key = operation_key("owner", lane, "op-repeated-1").unwrap();
        assert!(existing(&store, lane, &key).await.is_none());
        let publish = begin(lane, &key).expect("first attempt");
        assert!(begin(lane, &key).is_none(), "one attempt per operation");
        let waiter = {
            let (store, key) = (store.clone(), key.clone());
            tokio::spawn(async move { existing(&store, lane, &key).await })
        };
        publish.send_replace(Some((StatusCode::OK, json!({"outcome":"reset"}))));
        assert_eq!(
            waiter.await.unwrap(),
            Some((StatusCode::OK, json!({"outcome":"reset"})))
        );
        // After a restart only the action log remains: a started attempt
        // without a terminal row is unknown, never a new spend.
        let restarted = operation_key("owner", lane, "op-restarted").unwrap();
        log_reset(
            &store,
            lane,
            "manual",
            "started",
            "preflight",
            "m",
            None,
            Some(&restarted),
        )
        .await;
        let (status, body) = existing(&store, lane, &restarted).await.unwrap();
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["error"]["code"], "reset_outcome_unknown");
    }
}
