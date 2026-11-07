//! Drop an actual installed adapter's successful Open reply after admission.
//! The fixture owns both sockets; no production transport fault is injected.
use super::*;

impl CodeRuntimeHost {
    pub(crate) async fn fixture_acquire_with_lost_open_reply(
        &self,
        plugin: &str,
        payload: &Value,
    ) -> Value {
        let worktree = request_worktree(payload).unwrap().unwrap();
        let route = self.routes.lock().await[&(plugin.to_owned(), worktree)].clone();
        let mut route = route.lock().await;
        let original = route.target.as_ref().unwrap().socket().to_owned();
        let directory = PrivateRuntimeDirectory::create().unwrap();
        let proxy = directory.0.join("proxy.sock");
        let listener = tokio::net::UnixListener::bind(&proxy).unwrap();
        let mut relay = tokio::spawn(async move {
            for expected in [
                "agentAcquisitionSupport",
                "prepareAgentAcquisition",
                "agentAcquisition",
            ] {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut line = String::new();
                BufReader::new(&mut stream)
                    .read_line(&mut line)
                    .await
                    .unwrap();
                let request: Value = serde_json::from_str(&line).unwrap();
                assert_eq!(request["type"], expected);
                let response = exchange(&original, &request).await.unwrap();
                if expected == "agentAcquisition" {
                    assert_eq!(request["action"], "execute");
                    assert_eq!(response["state"], "ready");
                    assert_eq!(response["buffer"]["leases"], 1);
                    assert!(
                        response["buffer"]["buffer_id"]
                            .as_u64()
                            .is_some_and(|id| id != 0)
                    );
                    return response["buffer"].clone(); // Drop this successful reply on the wire.
                }
                let mut bytes = serde_json::to_vec(&response).unwrap();
                bytes.push(b'\n');
                stream.write_all(&bytes).await.unwrap();
            }
            unreachable!()
        });
        assert!(
            self.acquire_admitted(&mut route, &proxy, payload)
                .await
                .is_err()
        );
        let result = tokio::time::timeout(Duration::from_secs(10), &mut relay).await;
        if result.is_err() {
            relay.abort();
        }
        result
            .expect("native Open reply was not discarded")
            .unwrap()
    }
}
