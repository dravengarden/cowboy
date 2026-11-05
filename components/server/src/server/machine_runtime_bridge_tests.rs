
use super::{
    MACHINE_HEARTBEAT_TIMEOUT, MachineHeartbeatWatchdog, forward_machine_runtime_frames,
    queue_machine_json, write_machine_messages, write_machine_runtime_frames,
};
use crate::machine_protocol::MachineFrame;
use crate::runtime_wire::{CoreCommand, Frame, FrameReader, read_frame, write_frame};
use std::time::Duration;

#[test]
fn machine_heartbeat_watchdog_starts_on_first_heartbeat_and_resets() {
    let start = std::time::Instant::now();
    let mut watchdog = MachineHeartbeatWatchdog::default();
    assert!(!watchdog.expired_at(start + MACHINE_HEARTBEAT_TIMEOUT));

    watchdog.observe_at(start);
    assert!(!watchdog.expired_at(start + MACHINE_HEARTBEAT_TIMEOUT - Duration::from_millis(1)));
    assert!(watchdog.expired_at(start + MACHINE_HEARTBEAT_TIMEOUT));

    watchdog.observe_at(start + Duration::from_secs(30));
    assert!(!watchdog.expired_at(start + MACHINE_HEARTBEAT_TIMEOUT));
    assert!(watchdog.expired_at(start + Duration::from_secs(30) + MACHINE_HEARTBEAT_TIMEOUT));
}

#[tokio::test]
async fn runtime_bridge_makes_full_duplex_progress_under_backpressure() {
    // A tiny transport makes two multi-kilobyte frames deterministically
    // reproduce the full-duplex write cycle seen on Hawk.
    let (core, tunnel) = tokio::io::duplex(128);
    let (mut core_reader, mut core_writer) = tokio::io::split(core);
    let (tunnel_reader, tunnel_writer) = tokio::io::split(tunnel);
    let controller_frame = Frame::CoreCommand {
        command: CoreCommand::SetDesiredGeneration {
            generation: "controller".repeat(1_024),
            worker_command: None,
        },
    };
    let machine_frame = Frame::Reject {
        reason: "machine".repeat(1_024),
    };
    let expected_controller_frame = controller_frame.clone();
    let expected_machine_frame = machine_frame.clone();
    let (outbound_tx, mut outbound_rx) = tokio::sync::mpsc::channel(1);
    let forwarder = tokio::spawn(forward_machine_runtime_frames(
        FrameReader::new(tunnel_reader),
        outbound_tx,
    ));
    let (inbound_tx, inbound_rx) = tokio::sync::mpsc::unbounded_channel();
    let writer = tokio::spawn(write_machine_runtime_frames(tunnel_writer, inbound_rx));
    let controller = tokio::spawn(async move {
        write_frame(&mut core_writer, &controller_frame)
            .await
            .expect("write Controller frame");
        read_frame(&mut core_reader)
            .await
            .expect("read Machine frame")
            .expect("Machine frame")
    });

    let (forwarded, received) = tokio::time::timeout(Duration::from_secs(1), async {
        inbound_tx.send(machine_frame).expect("queue Machine frame");
        let forwarded = outbound_rx
            .recv()
            .await
            .expect("forwarded Controller frame");
        let received = controller.await.expect("Controller task");
        (forwarded, received)
    })
    .await
    .expect("full-duplex bridge must make progress");

    assert_eq!(forwarded, expected_controller_frame);
    assert_eq!(received, expected_machine_frame);
    forwarder.abort();
    writer.abort();
}

#[tokio::test]
async fn machine_outbound_burst_does_not_disconnect_while_writer_is_backpressured() {
    let sink = futures::sink::unfold((), |(), _message| async move {
        std::future::pending::<Result<(), std::io::Error>>().await
    });
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let writer = tokio::spawn(write_machine_messages(Box::pin(sink), rx, false, None));

    queue_machine_json(&tx, &MachineFrame::Heartbeat { sent_at_ms: 0 })
        .expect("queue frame that stalls in the Machine WebSocket sink");
    tokio::task::yield_now().await;
    for sent_at_ms in 1..=128 {
        queue_machine_json(&tx, &MachineFrame::Heartbeat { sent_at_ms })
            .expect("buffer burst beyond the retired 64-frame limit");
    }

    assert!(
        !writer.is_finished(),
        "backpressure must not close the Machine WebSocket writer"
    );
    writer.abort();
    assert!(
        writer
            .await
            .expect_err("aborted writer must stop")
            .is_cancelled(),
        "writer task should only stop because the test cancelled it"
    );
}
