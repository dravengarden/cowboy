
use super::send_json_with_timeout;
use std::time::Duration;

#[tokio::test]
async fn stalled_websocket_write_times_out_so_the_connection_can_reconcile() {
    let sink = futures::sink::unfold((), |(), _message| async move {
        std::future::pending::<Result<(), std::io::Error>>().await
    });
    futures::pin_mut!(sink);

    let result = send_json_with_timeout(
        &mut sink,
        &serde_json::json!({ "type": "runtime" }),
        Duration::from_millis(10),
    )
    .await;

    assert!(result.is_err(), "a stalled WebSocket write must be fenced");
}
