#![allow(unused_imports)]
use crate::acp::{self, AgentCommand};
use crate::agent_model::{Event, Status};
use crate::agent_sink::AgentSink;
use crate::provider;
use crate::runtime_wire::{
    Frame, FrameReader, MIN_PROTOCOL_VERSION, PROTOCOL_VERSION, PeerRole, RuntimeEvent,
    WorkerCommand, WorkerSnapshot, WorkerState, read_frame, write_frame,
};
use crate::worker::shared;
use crate::worker::*;
use agent_client_protocol::schema::v1::ContentBlock;
use anyhow::{Context as _, Result};
use parking_lot::Mutex;
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::os::unix::fs::MetadataExt as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::net::UnixStream;
use tokio::sync::mpsc;

#[tokio::test]
#[cfg(feature = "machine-host")]
async fn runtime_trace_crosses_real_dispatch_and_framing_without_history_or_replay_duplication() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("telemetry");
    let file = crate::telemetry_file::TelemetryFile::open(
        directory.clone(),
        65536,
        2,
        chrono::Utc::now().timestamp_millis(),
    )
    .unwrap();
    let telemetry = crate::observability::Observability::start(None, file, None);
    let dispatch = crate::observability::Observability::begin_command_trace(
        r#"{"traceparent":"00-11111111111111111111111111111111-2222222222222222-01"}"#,
    )
    .unwrap();
    telemetry.bind_runtime_trace(
        "fixture-owner",
        "fixture-machine",
        "sess-1",
        "fixture-cmid",
        &dispatch,
    );
    telemetry.finish_command_trace("fixture-owner", dispatch, true);
    let trace = telemetry
        .dispatch_trace("sess-1", Some("fixture-cmid"))
        .unwrap();
    let correlation = trace.context.correlation_id.clone();

    let (shared, _notify) = shared();
    let hub = crate::core::Hub::new();
    hub.create_local_session(
        "sess-1".into(),
        "codex".into(),
        "/tmp".into(),
        "fixture".into(),
        crate::core::SessionOrigin::Web,
        false,
    );
    let runtime = crate::remote_runtime::RemoteRuntime::for_test(hub, vec![shared.snapshot()]);
    runtime.attach_telemetry("fixture-machine".into(), telemetry.clone());
    runtime.prompt_traced(
        "sess-1",
        vec![serde_json::json!({"type":"text","text":"fixture-private-prompt"})],
        Some("fixture-cmid".into()),
        Some(trace),
    );
    let pending = runtime.pending_for_test().pop().unwrap();
    let command = crate::machine_broker::dispatch_trace_fixture(pending).await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut tx = Some(tx);
    let ack = handle_command(&shared, &mut tx, command.clone());
    let sequence = shared.snapshot().last_runtime_seq;
    assert!(matches!(
        handle_command(&shared, &mut tx, command),
        Frame::CommandAck { accepted: true, .. }
    ));
    assert_eq!(shared.snapshot().last_runtime_seq, sequence);
    let AgentCommand::Prompt(blocks, _, _) = rx.try_recv().unwrap() else {
        panic!("ACP prompt");
    };
    let content = serde_json::to_string(&blocks).unwrap();
    assert!(content.contains("fixture-private-prompt"));
    assert!(!content.contains("traceparent"));
    assert!(!content.contains(&correlation));
    assert!(rx.try_recv().is_err());

    let sink = RemoteSink {
        shared: Arc::clone(&shared),
    };
    sink.prompt_started("sess-1", Some("fixture-cmid"));
    sink.push("sess-1", Event::Update { update: serde_json::json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"fixture-private-answer"}}) });
    sink.prompt_completed("sess-1", Some("fixture-cmid"), "EndTurn");
    sink.push(
        "sess-1",
        Event::TurnEnd {
            usage: None,
            stop_reason: "EndTurn".into(),
        },
    );

    let (mut wire, mut reader) = tokio::io::duplex(65536);
    send_outbox(&shared, &mut wire, &mut 0).await.unwrap();
    drop(wire);
    let mut frames = Vec::new();
    while let Some(frame) = read_frame(&mut reader).await.unwrap() {
        if let Frame::WorkerEvent { event, .. } = &frame {
            assert!(!serde_json::to_string(event).unwrap().contains(&correlation));
        }
        runtime.project_trace_fixture(frame.clone()).await;
        frames.push(frame);
    }
    runtime.project_trace_fixture(ack).await;
    for frame in frames {
        runtime.project_trace_fixture(frame).await;
    }
    shared.ack(shared.snapshot().last_runtime_seq);
    assert!(shared.trace_outbox.lock().is_empty());
    telemetry.drain().await;

    let local = std::fs::read_to_string(directory.join("telemetry.jsonl")).unwrap();
    assert!(!local.contains("fixture-private"));
    assert!(!local.contains("fixture-owner"));
    assert!(!local.contains("fixture-cmid"));
    assert!(!local.contains(&correlation));
    let spans: BTreeMap<String, opentelemetry_proto::tonic::trace::v1::Span> = local
        .lines()
        .map(|line| {
            let row: serde_json::Value = serde_json::from_str(line).unwrap();
            let span: opentelemetry_proto::tonic::trace::v1::Span =
                serde_json::from_value(row["span"].clone()).unwrap();
            (span.name.clone(), span)
        })
        .collect();
    assert_eq!(
        local.lines().count(),
        7,
        "one local record per stage despite command/event replay"
    );
    assert_eq!(spans.len(), 7);
    let chain = [
        "cowboy.controller.dispatch",
        "cowboy.controller.queue",
        "cowboy.controller.delivery",
        "cowboy.machine.dispatch",
        "cowboy.worker.queue",
        "cowboy.worker.prompt",
        "cowboy.worker.first_output",
    ];
    let mut parent = vec![0x22; 8];
    for name in chain {
        let span = &spans[name];
        assert_eq!(span.trace_id, vec![0x11; 16]);
        assert_eq!(span.parent_span_id, parent, "{name}");
        assert!(span.end_time_unix_nano >= span.start_time_unix_nano);
        parent.clone_from(&span.span_id);
    }
}
