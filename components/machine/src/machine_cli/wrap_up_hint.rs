//! Reliable-first bounded wrap-up hint routing between the Machine WebSocket
//! client (`machine_cli.rs`) and the local Machine broker UnixStream.

use std::time::Duration;

use anyhow::Context as _;
use tokio::sync::mpsc;

use crate::machine_broker::wrap_up::{encoded_frame_bytes, unix_socket_has_hint_headroom};
use crate::machine_protocol::WRAP_UP_STEERING_PROTOCOL_VERSION;
use crate::runtime_wire::{CoreCommand, Frame};
use crate::wrap_up::wrap_up_hint_within_size_limit;

/// Bounded per-connection hint queue capacity from `machine_cli` to broker.
pub(super) const HINT_RUNTIME_CAPACITY: usize = 32;

pub(super) struct RuntimeWriterChannels {
    pub(super) reliable_tx: mpsc::UnboundedSender<Frame>,
    pub(super) hint_tx: mpsc::Sender<Frame>,
    pub(super) diag_tx: mpsc::Sender<Frame>,
    pub(super) task: tokio::task::JoinHandle<anyhow::Result<()>>,
}

pub(super) fn spawn_runtime_channels(
    runtime_writer: tokio::net::unix::OwnedWriteHalf,
    timeout: Duration,
) -> RuntimeWriterChannels {
    let (reliable_tx, reliable_rx) = mpsc::unbounded_channel();
    let (hint_tx, hint_rx) = mpsc::channel(HINT_RUNTIME_CAPACITY);
    let (diag_tx, diag_rx) = mpsc::channel(super::post_edit::DIAG_RUNTIME_REPLY_CAPACITY);
    let task = tokio::spawn(run_runtime_unix_writer_with_hints(
        runtime_writer,
        reliable_rx,
        hint_rx,
        diag_rx,
        timeout,
    ));
    RuntimeWriterChannels {
        reliable_tx,
        hint_tx,
        diag_tx,
        task,
    }
}

/// Route an inbound `MachineFrame::Runtime` frame from the Controller WebSocket:
/// - Stamps authoritative negotiated `outer_machine_protocol` onto `Frame::Hello`.
/// - Routes `CoreCommand::WrapUpHint` to the bounded drop-on-full `hint_tx`
///   (only when outer WS protocol >= 53 and frame size <= 512 bytes).
/// - Sends all reliable runtime frames to `reliable_tx`.
pub(super) fn route_inbound_runtime_frame(
    mut frame: Frame,
    outer_protocol: u16,
    post_edit_http_enabled: bool,
    reliable_tx: &mpsc::UnboundedSender<Frame>,
    hint_tx: &mpsc::Sender<Frame>,
) -> anyhow::Result<()> {
    match &mut frame {
        Frame::Hello {
            outer_machine_protocol,
            post_edit_http,
            ..
        } => {
            *outer_machine_protocol = Some(outer_protocol);
            *post_edit_http = post_edit_http_enabled;
            reliable_tx
                .send(frame)
                .map_err(|_| anyhow::anyhow!("Machine runtime writer queue closed"))?;
        }
        Frame::CoreCommand {
            command: CoreCommand::WrapUpHint { .. },
        } => {
            if outer_protocol >= WRAP_UP_STEERING_PROTOCOL_VERSION
                && wrap_up_hint_within_size_limit(&frame)
            {
                let _ = hint_tx.try_send(frame);
            }
        }
        _ => {
            reliable_tx
                .send(frame)
                .map_err(|_| anyhow::anyhow!("Machine runtime writer queue closed"))?;
        }
    }
    Ok(())
}

/// Write runtime frames to the local broker UnixStream with strict reliable-first
/// priority (`biased;`) and complete-frame atomic writes under `timeout`.
#[cfg(test)]
pub(super) async fn write_runtime_frames_with_headroom<W, F>(
    writer: &mut W,
    reliable: &mut mpsc::UnboundedReceiver<Frame>,
    hints: Option<mpsc::Receiver<Frame>>,
    timeout: Duration,
    has_headroom: F,
) -> anyhow::Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
    F: FnMut(&W, usize) -> bool,
{
    write_runtime_frames_with_headroom_and_diag(
        writer,
        reliable,
        hints,
        None,
        timeout,
        has_headroom,
    )
    .await
}

pub(super) async fn write_runtime_frames_with_headroom_and_diag<W, F>(
    writer: &mut W,
    reliable: &mut mpsc::UnboundedReceiver<Frame>,
    mut hints: Option<mpsc::Receiver<Frame>>,
    mut diag_replies: Option<mpsc::Receiver<Frame>>,
    timeout: Duration,
    mut has_headroom: F,
) -> anyhow::Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
    F: FnMut(&W, usize) -> bool,
{
    loop {
        let (frame, best_effort) = tokio::select! {
            biased;
            frame = reliable.recv() => match frame {
                Some(frame) => (frame, false),
                None => return Ok(()),
            },
            hint = async {
                match hints.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            }, if hints.is_some() => match hint {
                Some(hint) => (hint, true),
                None => {
                    hints = None;
                    continue;
                }
            },
            diag = async {
                match diag_replies.as_mut() {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            }, if diag_replies.is_some() => match diag {
                Some(diag) => (diag, true),
                None => {
                    diag_replies = None;
                    continue;
                }
            },
        };
        if best_effort {
            let Some(frame_bytes) = encoded_frame_bytes(&frame) else {
                continue;
            };
            if !has_headroom(writer, frame_bytes) {
                continue;
            }
        }
        tokio::time::timeout(timeout, crate::runtime_wire::write_frame(writer, &frame))
            .await
            .context("Machine runtime frame write timed out")??;
    }
}

#[cfg(test)]
pub(super) async fn write_runtime_frames_with_hints<W>(
    writer: &mut W,
    reliable: &mut mpsc::UnboundedReceiver<Frame>,
    hints: Option<mpsc::Receiver<Frame>>,
    timeout: Duration,
) -> anyhow::Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    write_runtime_frames_with_headroom(writer, reliable, hints, timeout, |_, _| true).await
}

pub(super) async fn run_runtime_unix_writer_with_hints(
    mut writer: tokio::net::unix::OwnedWriteHalf,
    mut frames: mpsc::UnboundedReceiver<Frame>,
    hints: mpsc::Receiver<Frame>,
    diag_replies: mpsc::Receiver<Frame>,
    timeout: Duration,
) -> anyhow::Result<()> {
    write_runtime_frames_with_headroom_and_diag(
        &mut writer,
        &mut frames,
        Some(hints),
        Some(diag_replies),
        timeout,
        unix_socket_has_hint_headroom,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_wire::{DeliveryTarget, PeerRole, read_frame};

    #[tokio::test]
    async fn stamps_outer_protocol_and_prioritizes_reliable_over_hints() {
        let (reliable_tx, mut reliable_rx) = mpsc::unbounded_channel();
        let (hint_tx, hint_rx) = mpsc::channel(HINT_RUNTIME_CAPACITY);

        let hello = Frame::Hello {
            role: PeerRole::Core,
            min_protocol: 1,
            max_protocol: 8,
            build: "test".into(),
            session_id: None,
            worker_epoch: None,
            generation: None,
            executable: None,
            fallback_for: None,
            outer_machine_protocol: None,
            post_edit_http: false,
        };
        route_inbound_runtime_frame(hello, 53, true, &reliable_tx, &hint_tx).unwrap();
        let hint = Frame::CoreCommand {
            command: CoreCommand::WrapUpHint {
                session_id: "s1".into(),
                target: DeliveryTarget {
                    worker_epoch: "e1".into(),
                    turn_id: "t1".into(),
                },
            },
        };
        // Pre-53 outer protocol drops hints.
        route_inbound_runtime_frame(hint.clone(), 52, false, &reliable_tx, &hint_tx).unwrap();
        assert!(hint_tx.capacity() == HINT_RUNTIME_CAPACITY);

        // Protocol 53 admits hint to hint_tx, reliable Cancel still jumps ahead.
        route_inbound_runtime_frame(hint, 53, true, &reliable_tx, &hint_tx).unwrap();
        let cancel = Frame::CoreCommand {
            command: CoreCommand::Cancel {
                session_id: "s1".into(),
                command_id: "cmd-cancel".into(),
            },
        };
        route_inbound_runtime_frame(cancel, 53, true, &reliable_tx, &hint_tx).unwrap();
        let (mut client, mut server) = tokio::io::duplex(4096);
        let write_task = tokio::spawn(async move {
            write_runtime_frames_with_hints(
                &mut client,
                &mut reliable_rx,
                Some(hint_rx),
                Duration::from_secs(5),
            )
            .await
            .unwrap();
        });

        let f1 = read_frame(&mut server).await.unwrap().unwrap();
        assert!(matches!(
            f1,
            Frame::Hello {
                outer_machine_protocol: Some(53),
                post_edit_http: true,
                ..
            }
        ));
        let f2 = read_frame(&mut server).await.unwrap().unwrap();
        assert!(matches!(
            f2,
            Frame::CoreCommand {
                command: CoreCommand::Cancel { .. }
            }
        ));
        let f3 = read_frame(&mut server).await.unwrap().unwrap();
        assert!(matches!(
            f3,
            Frame::CoreCommand {
                command: CoreCommand::WrapUpHint { .. }
            }
        ));
        drop(reliable_tx);
        drop(hint_tx);
        write_task.await.unwrap();
    }

    #[tokio::test]
    async fn drops_hint_before_byte_zero_when_broker_socket_headroom_is_insufficient() {
        let (reliable_tx, mut reliable_rx) = mpsc::unbounded_channel();
        let (hint_tx, hint_rx) = mpsc::channel(HINT_RUNTIME_CAPACITY);
        hint_tx
            .try_send(Frame::CoreCommand {
                command: CoreCommand::WrapUpHint {
                    session_id: "s1".into(),
                    target: DeliveryTarget {
                        worker_epoch: "e1".into(),
                        turn_id: "t1".into(),
                    },
                },
            })
            .unwrap();
        let (mut client, mut server) = tokio::io::duplex(1024);
        let write_task = tokio::spawn(async move {
            write_runtime_frames_with_headroom(
                &mut client,
                &mut reliable_rx,
                Some(hint_rx),
                Duration::from_secs(5),
                |_, _| false,
            )
            .await
            .unwrap();
        });
        tokio::task::yield_now().await;
        reliable_tx
            .send(Frame::CoreCommand {
                command: CoreCommand::Cancel {
                    session_id: "s1".into(),
                    command_id: "cancel-first".into(),
                },
            })
            .unwrap();
        let first = read_frame(&mut server).await.unwrap().unwrap();
        assert!(matches!(
            first,
            Frame::CoreCommand {
                command: CoreCommand::Cancel { command_id, .. }
            } if command_id == "cancel-first"
        ));
        drop(reliable_tx);
        drop(hint_tx);
        write_task.await.unwrap();
    }
}
