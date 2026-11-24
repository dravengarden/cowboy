//! Reliable-first bounded wrap-up hint routing between the Machine WebSocket
//! client (`machine_cli.rs`) and the local Machine broker UnixStream.

use std::time::Duration;

use anyhow::Context as _;
use tokio::sync::mpsc;

use crate::machine_protocol::WRAP_UP_STEERING_PROTOCOL_VERSION;
use crate::runtime_wire::{CoreCommand, Frame};
use crate::wrap_up::wrap_up_hint_within_size_limit;

/// Bounded per-connection hint queue capacity from `machine_cli` to broker.
pub(super) const HINT_RUNTIME_CAPACITY: usize = 32;

/// Route an inbound `MachineFrame::Runtime` frame from the Controller WebSocket:
/// - Stamps authoritative negotiated `outer_machine_protocol` onto `Frame::Hello`.
/// - Routes `CoreCommand::WrapUpHint` to the bounded drop-on-full `hint_tx`
///   (only when outer WS protocol >= 53 and frame size <= 512 bytes).
/// - Sends all reliable runtime frames to `reliable_tx`.
pub(super) fn route_inbound_runtime_frame(
    mut frame: Frame,
    outer_protocol: u16,
    reliable_tx: &mpsc::UnboundedSender<Frame>,
    hint_tx: &mpsc::Sender<Frame>,
) -> anyhow::Result<()> {
    match &mut frame {
        Frame::Hello {
            outer_machine_protocol,
            ..
        } => {
            *outer_machine_protocol = Some(outer_protocol);
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
pub(super) async fn write_runtime_frames_with_hints<W>(
    writer: &mut W,
    reliable: &mut mpsc::UnboundedReceiver<Frame>,
    mut hints: Option<mpsc::Receiver<Frame>>,
    timeout: Duration,
) -> anyhow::Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    loop {
        let frame = match hints.as_mut() {
            Some(hint_rx) => tokio::select! {
                biased;
                frame = reliable.recv() => match frame {
                    Some(frame) => frame,
                    None => return Ok(()),
                },
                hint = hint_rx.recv() => match hint {
                    Some(hint) => hint,
                    None => {
                        hints = None;
                        continue;
                    }
                },
            },
            None => match reliable.recv().await {
                Some(frame) => frame,
                None => return Ok(()),
            },
        };
        tokio::time::timeout(timeout, crate::runtime_wire::write_frame(writer, &frame))
            .await
            .context("Machine runtime frame write timed out")??;
    }
}

pub async fn run_runtime_writer_with_hints<W>(
    mut writer: W,
    mut frames: mpsc::UnboundedReceiver<Frame>,
    hints: mpsc::Receiver<Frame>,
    timeout: Duration,
) -> anyhow::Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    write_runtime_frames_with_hints(&mut writer, &mut frames, Some(hints), timeout).await
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
        };
        route_inbound_runtime_frame(hello, 53, &reliable_tx, &hint_tx).unwrap();
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
        route_inbound_runtime_frame(hint.clone(), 52, &reliable_tx, &hint_tx).unwrap();
        assert!(hint_tx.capacity() == HINT_RUNTIME_CAPACITY);

        // Protocol 53 admits hint to hint_tx, reliable Cancel still jumps ahead.
        route_inbound_runtime_frame(hint, 53, &reliable_tx, &hint_tx).unwrap();
        let cancel = Frame::CoreCommand {
            command: CoreCommand::Cancel {
                session_id: "s1".into(),
                command_id: "cmd-cancel".into(),
            },
        };
        route_inbound_runtime_frame(cancel, 53, &reliable_tx, &hint_tx).unwrap();
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
}
