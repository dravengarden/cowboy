use super::*;
use std::sync::{Arc, Mutex};
use tokio_tungstenite::tungstenite::Message as Ws;

async fn capture(features: impl Into<Features>, messages: Vec<Ws>) -> Vec<Ws> {
    let (tx, rx) = mpsc::unbounded_channel();
    for message in messages {
        tx.send(message).unwrap();
    }
    drop(tx);
    let output = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&output);
    let sink = futures::sink::unfold((), move |(), frame| {
        captured.lock().unwrap().push(frame);
        async { Ok::<_, std::io::Error>(()) }
    });
    write(Box::pin(sink), rx, features).await.unwrap();
    Arc::try_unwrap(output).unwrap().into_inner().unwrap()
}

#[tokio::test]
async fn compressed_transcript_unblocks_following_tool_reply_with_identical_application_bytes() {
    // Native JSONL copied by a hook is base64 inside the Machine envelope.
    // Unique records prevent a constant-byte-only compression benchmark.
    use base64::Engine;
    let transcript: String = (0..12000).map(|i| format!("{{\"id\":{i},\"role\":\"assistant\",\"text\":\"Read the target file; 中文; preserve bytes\\n\"}}\n")).collect();
    let bulk = serde_json::json!({"dataBase64": base64::engine::general_purpose::STANDARD.encode(transcript)}).to_string();
    let messages = vec![Ws::Text(bulk.clone().into()), Ws::Text("tool reply".into())];
    let plain = capture(true, messages.clone()).await;
    let compressed = capture((true, true), messages).await;
    let wire_bytes = |frames: &[Ws]| frames.iter().map(|frame| frame.len()).sum::<usize>();
    assert!(wire_bytes(&compressed) * 10 < wire_bytes(&plain));
    let mut decoder = Decoder::default();
    let mut decoded = Vec::new();
    for frame in compressed {
        match frame {
            Ws::Binary(bytes) => {
                if let Some(text) = decoder.chunk((true, true), &bytes).await.unwrap() {
                    decoded.push(text);
                }
            }
            Ws::Text(text) => {
                decoder.text(&text).unwrap();
                decoded.push(text.to_string());
            }
            _ => panic!("unexpected frame"),
        }
    }
    assert_eq!(decoded, [bulk, "tool reply".to_owned()]);
    assert!(
        plain
            .iter()
            .any(|frame| matches!(frame, Ws::Binary(bytes) if bytes[0] & 0x80 == 0))
    );
}

#[tokio::test]
async fn compression_refuses_unnegotiated_corrupt_truncated_trailing_and_inflated_frames() {
    let packed = compress(&vec![b'x'; CHUNK_BYTES * 2]).unwrap();
    let frame = |bytes: &[u8]| {
        [
            ((bytes.len() as u32) | COMPRESSED).to_be_bytes().as_slice(),
            &0_u32.to_be_bytes(),
            bytes,
        ]
        .concat()
    };
    assert!(
        Decoder::default()
            .chunk(true, &frame(&packed))
            .await
            .is_err()
    );
    assert!(
        Decoder::default()
            .chunk((false, true), &frame(&packed))
            .await
            .is_err()
    );
    assert!(
        Decoder::default()
            .chunk((true, true), &frame(b"invalid"))
            .await
            .is_err()
    );
    for cut in 1..=6 {
        assert!(
            Decoder::default()
                .chunk((true, true), &frame(&packed[..packed.len() - cut]))
                .await
                .is_err()
        );
    }
    let mut corrupt = packed.clone();
    *corrupt.last_mut().unwrap() ^= 1;
    assert!(
        Decoder::default()
            .chunk((true, true), &frame(&corrupt))
            .await
            .is_err()
    );
    let mut trailing = packed.clone();
    trailing.push(0);
    assert!(
        Decoder::default()
            .chunk((true, true), &frame(&trailing))
            .await
            .is_err()
    );
    let oversized = compress(&vec![b'x'; MAX_BYTES + 1]).unwrap();
    assert!(decompress(&oversized).is_err());
    let mut changed = Decoder::default();
    let half = packed.len() / 2;
    let mut first = frame(&packed);
    first.truncate(8 + half);
    assert!(changed.chunk((true, true), &first).await.unwrap().is_none());
    let mut next = frame(&packed[half..]);
    next[..4].copy_from_slice(&(packed.len() as u32).to_be_bytes());
    next[4..8].copy_from_slice(&(half as u32).to_be_bytes());
    assert!(changed.chunk((true, true), &next).await.is_err());
}

#[tokio::test]
async fn small_frame_replay_yields_to_the_heartbeat_producer() {
    let (tx, rx) = mpsc::unbounded_channel();
    for index in 0..32 {
        tx.send(Ws::Text(format!("application-{index}").into()))
            .unwrap();
    }
    let (first_write, written) = tokio::sync::oneshot::channel();
    let producer = tokio::spawn(async move {
        written.await.unwrap();
        tx.send(Ws::Text("{\"type\":\"heartbeat\",\"sent_at_ms\":1}".into()))
            .unwrap();
    });
    let output = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&output);
    let sink = futures::sink::unfold(Some(first_write), move |signal, message: Ws| {
        let captured = Arc::clone(&captured);
        async move {
            captured.lock().unwrap().push(message);
            if let Some(signal) = signal {
                signal.send(()).unwrap();
            }
            Ok::<_, std::io::Error>(None)
        }
    });
    write(Box::pin(sink), rx, true).await.unwrap();
    producer.await.unwrap();
    let output = output.lock().unwrap();
    assert!(output[1].urgent(), "writer starved the heartbeat producer");
    assert_eq!(output.len(), 33);
}

#[tokio::test]
async fn heartbeats_overtake_a_backlog_of_small_application_frames() {
    let (tx, rx) = mpsc::unbounded_channel();
    for index in 0..32 {
        tx.send(Ws::Text(format!("application-{index}").into()))
            .unwrap();
    }
    let output = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&output);
    let sink = futures::sink::unfold(Some(tx), move |sender, message: Ws| {
        let captured = Arc::clone(&captured);
        async move {
            captured.lock().unwrap().push(message);
            if let Some(sender) = sender {
                sender
                    .send(Ws::Text("{\"type\":\"heartbeat\",\"sent_at_ms\":1}".into()))
                    .unwrap();
            }
            Ok::<_, std::io::Error>(None)
        }
    });
    write(Box::pin(sink), rx, true).await.unwrap();
    let output = output.lock().unwrap();
    assert!(output[1].urgent(), "heartbeat waited behind the replay");
    let application: Vec<_> = output
        .iter()
        .filter(|message| !message.urgent())
        .map(|message| message.text().unwrap().to_owned())
        .collect();
    assert_eq!(
        application,
        (0..32)
            .map(|index| format!("application-{index}"))
            .collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn real_websocket_backpressure_preserves_heartbeat_during_bulk_transfer() {
    for features in [Features::from(true), Features::from((true, true))] {
        websocket_backpressure(features).await;
    }
}

async fn websocket_backpressure(features: Features) {
    use futures::StreamExt;
    use tokio_tungstenite::{WebSocketStream, tungstenite::protocol::Role};

    // A tiny duplex capacity gives actual WebSocket writes sustained
    // backpressure without relying on TCP buffer sizes or a real network.
    let (client, server) = tokio::io::duplex(1024);
    let client = WebSocketStream::from_raw_socket(client, Role::Client, None).await;
    let mut server = WebSocketStream::from_raw_socket(server, Role::Server, None).await;
    let (tx, rx) = mpsc::unbounded_channel();
    let bulk: String = (0_u64..10000)
        .map(|i| format!("{:016x}abcdef", i.wrapping_mul(6364136223846793005)))
        .collect();
    tx.send(Ws::Text(bulk.clone().into())).unwrap();
    let writer = tokio::spawn(write(client, rx, features));
    let first = server.next().await.unwrap().unwrap();
    let Ws::Binary(first) = first else {
        panic!("chunk")
    };
    let mut decoder = Decoder::default();
    assert!(decoder.chunk(features, &first).await.unwrap().is_none());
    tx.send(Ws::Text("{\"type\":\"heartbeat\",\"sent_at_ms\":1}".into()))
        .unwrap();
    drop(tx);
    let mut saw_heartbeat = false;
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match server.next().await.unwrap().unwrap() {
                Ws::Text(text) => {
                    decoder.text(&text).unwrap();
                    assert!(heartbeat(&text));
                    saw_heartbeat = true;
                }
                Ws::Binary(bytes) => {
                    if let Some(text) = decoder.chunk(features, &bytes).await.unwrap() {
                        assert!(saw_heartbeat);
                        assert_eq!(text, bulk);
                        break;
                    }
                }
                _ => panic!("unexpected frame"),
            }
        }
    })
    .await
    .unwrap();
    writer.await.unwrap().unwrap();
}

#[tokio::test]
async fn slow_bulk_transfer_passes_heartbeats_without_reordering_application_data() {
    let (tx, rx) = mpsc::unbounded_channel();
    let observed = Arc::new(Mutex::new(Vec::new()));
    let output = Arc::clone(&observed);
    let heartbeat_tx = tx.clone();
    let sink = futures::sink::unfold(false, move |sent_first, message: Ws| {
        let output = Arc::clone(&output);
        let tx = heartbeat_tx.clone();
        async move {
            if !sent_first {
                // A heartbeat arrives while the first chunk is backpressured.
                tx.send(Ws::Text("{\"type\":\"heartbeat\",\"sent_at_ms\":1}".into()))
                    .unwrap();
                tx.send(Ws::Text(
                    "{\"type\":\"runtime\",\"frame\":{\"type\":\"heartbeat\"}}".into(),
                ))
                .unwrap();
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
            output.lock().unwrap().push(message);
            Ok::<_, std::io::Error>(true)
        }
    });
    let text = format!("{{\"data\":\"{}中文\"}}", "x".repeat(CHUNK_BYTES * 3));
    tx.send(Ws::Text(text.clone().into())).unwrap();
    tx.send(Ws::Text("second".into())).unwrap();
    let writer = tokio::spawn(write(Box::pin(sink), rx, true));
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if observed.lock().unwrap().last() == Some(&Ws::Text("second".into())) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .unwrap();
    writer.abort();
    let observed = observed.lock().unwrap().clone();
    assert!(matches!(&observed[0], Ws::Binary(_)));
    assert!(observed[1].urgent());
    assert!(observed[2].urgent());
    let mut decoder = Decoder::default();
    let mut decoded = Vec::new();
    for message in observed.iter() {
        match message {
            Ws::Binary(bytes) => {
                if let Some(text) = decoder.chunk(true, bytes).await.unwrap() {
                    decoded.push(text);
                }
            }
            Ws::Text(text) => {
                decoder.text(text).unwrap();
                if !heartbeat(text) {
                    decoded.push(text.to_string());
                }
            }
            _ => panic!("unexpected message"),
        }
    }
    assert_eq!(decoded, [text, "second".to_owned()]);
}

#[tokio::test]
async fn old_peers_receive_original_text_frames() {
    let (tx, rx) = mpsc::unbounded_channel();
    let text = "x".repeat(CHUNK_BYTES * 2);
    tx.send(Ws::Text(text.clone().into())).unwrap();
    drop(tx);
    let output = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&output);
    let sink = futures::sink::unfold((), move |(), frame| {
        captured.lock().unwrap().push(frame);
        async { Ok::<_, std::io::Error>(()) }
    });
    write(Box::pin(sink), rx, false).await.unwrap();
    assert_eq!(*output.lock().unwrap(), [Ws::Text(text.into())]);
}

#[tokio::test]
async fn corrupt_unnegotiated_oversized_and_interleaved_chunks_fail_closed() {
    let chunk = |total: u32, offset: u32, data: &[u8]| {
        [
            total.to_be_bytes().as_slice(),
            offset.to_be_bytes().as_slice(),
            data,
        ]
        .concat()
    };
    let first = chunk((CHUNK_BYTES + 1) as u32, 0, b"x");
    assert!(Decoder::default().chunk(false, &first).await.is_err());
    assert!(Decoder::default().chunk(true, &[0; 8]).await.is_err());
    assert!(
        Decoder::default()
            .chunk(true, &chunk(u32::MAX, 0, b"x"))
            .await
            .is_err()
    );
    assert!(
        Decoder::default()
            .chunk(true, &chunk((CHUNK_BYTES + 1) as u32, 1, b"x"))
            .await
            .is_err()
    );
    let mut decoder = Decoder::default();
    assert!(decoder.chunk(true, &first).await.unwrap().is_none());
    assert!(decoder.text("application data").is_err());
    assert!(decoder.chunk(true, &first).await.is_err());
    assert!(
        decoder
            .chunk(true, &chunk((CHUNK_BYTES + 2) as u32, 1, b"x"))
            .await
            .is_err()
    );
}
