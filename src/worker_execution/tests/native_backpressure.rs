//! Real native output, a slow relay and full-duplex filesystem traffic. Keep
//! payload assertions here instead of asking a scripted model to count bytes.
use super::*;
use base64::Engine as _;
use transport::{connect, receive, send};

pub(super) async fn check(endpoint: &Endpoint, binding: &BindingV1) {
    tokio::time::timeout(Duration::from_secs(120), run(endpoint, binding))
        .await
        .expect("native backpressure fixture timed out");
}

async fn run(endpoint: &Endpoint, binding: &BindingV1) {
    let mut socket = connect(endpoint).await;
    send(
        &mut socket,
        json!({"id":0,"method":"initialize","params":{}}),
    )
    .await;
    let initialized = receive(&mut socket).await;
    assert!(initialized["result"]["sessionId"].is_string());
    let target = std::path::Path::new(&binding.workspace.cwd);
    let start = |id: u64, process: &str, script: &str| {
        json!({
            "id":id,"method":"process/start","params":{
                "processId":process,"argv":["/run/current-system/sw/bin/bash","-c",script],
                "cwd":url::Url::from_directory_path(target).unwrap(),
                "env":{},"tty":false,"pipeStdin":false,"arg0":null,
                "envPolicy":{"inherit":"all","ignoreDefaultExcludes":false,"exclude":[],"set":{},"includeOnly":[]}
            }
        })
    };
    send(&mut socket, start(1, "flood", "printf once >> flood-starts; head -c 8388608 /dev/zero; head -c 1048576 /dev/zero >&2; exit 17")).await;
    // Larger than either OS pipe: a writer awaiting this frame must continue
    // polling the native reader, which is carrying the output flood.
    let contents = vec![0xa5; 3 * 1024 * 1024];
    send(
        &mut socket,
        json!({"id":2,"method":"fs/writeFile","params":{
            "path":url::Url::from_file_path(target.join("duplex-stress.bin")).unwrap(),
            "dataBase64":base64::engine::general_purpose::STANDARD.encode(&contents)
        }}),
    )
    .await;
    let mut replies = HashSet::new();
    // Upstream stdout, stderr and exit tasks assign sequence numbers under a
    // lock, then send independently. Native clients reorder these notifications.
    // Require every sequence exactly once and preserve each stream's order.
    let mut sequences = HashSet::new();
    let mut stdout_seq = 0;
    let mut stderr_seq = 0;
    let mut stdout = 0;
    let mut stderr = 0;
    let mut exited = false;
    let mut closed_seq = None;
    while closed_seq.is_none_or(|seq| sequences.len() as u64 != seq) || replies.len() < 2 {
        let frame = receive(&mut socket).await;
        if let Some(id) = frame["id"].as_u64() {
            assert!(frame.get("error").is_none(), "native request failed");
            assert!(replies.insert(id));
            continue;
        }
        let params = &frame["params"];
        assert_eq!(params["processId"], "flood");
        let seq = params["seq"].as_u64().unwrap();
        assert!(seq > 0 && sequences.insert(seq), "native event repeated");
        match frame["method"].as_str().unwrap() {
            "process/output" => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(params["chunk"].as_str().unwrap())
                    .unwrap();
                assert!(bytes.iter().all(|byte| *byte == 0));
                match params["stream"].as_str().unwrap() {
                    "stdout" => {
                        assert!(seq > stdout_seq, "stdout reordered");
                        stdout_seq = seq;
                        stdout += bytes.len();
                    }
                    "stderr" => {
                        assert!(seq > stderr_seq, "stderr reordered");
                        stderr_seq = seq;
                        stderr += bytes.len();
                    }
                    _ => panic!("unexpected native output stream"),
                }
            }
            "process/exited" => {
                assert_eq!(params["exitCode"], 17);
                exited = true;
            }
            "process/closed" => {
                assert!(closed_seq.replace(seq).is_none());
            }
            _ => panic!("unexpected native event"),
        }
    }
    assert!(exited);
    assert_eq!(sequences.iter().copied().max(), closed_seq);
    assert_eq!(stdout, 8 * 1024 * 1024);
    assert_eq!(stderr, 1024 * 1024);
    assert_eq!(
        std::fs::read_to_string(target.join("flood-starts")).unwrap(),
        "once"
    );
    assert_eq!(
        std::fs::read(target.join("duplex-stress.bin")).unwrap(),
        contents
    );

    send(
        &mut socket,
        start(
            3,
            "cancel-flood",
            "printf once >> cancel-starts; while :; do head -c 65536 /dev/zero; done",
        ),
    )
    .await;
    let mut terminated = false;
    let mut closed = false;
    let mut replies = HashSet::new();
    while !closed || replies.len() < 2 {
        let frame = receive(&mut socket).await;
        if let Some(id) = frame["id"].as_u64() {
            assert!(frame.get("error").is_none());
            replies.insert(id);
        } else {
            assert_eq!(frame["params"]["processId"], "cancel-flood");
            if !terminated && frame["method"] == "process/output" {
                send(&mut socket, json!({"id":4,"method":"process/terminate","params":{"processId":"cancel-flood"}})).await;
                terminated = true;
            }
            closed |= frame["method"] == "process/closed";
        }
    }
    assert!(terminated);
    assert_eq!(
        std::fs::read_to_string(target.join("cancel-starts")).unwrap(),
        "once"
    );
    send(
        &mut socket,
        json!({"id":5,"method":"fs/getMetadata","params":{
            "path":url::Url::from_file_path(target.join("duplex-stress.bin")).unwrap()
        }}),
    )
    .await;
    assert_eq!(receive(&mut socket).await["id"], 5);
    socket.close(None).await.unwrap();
}
