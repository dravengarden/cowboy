use super::*;
use std::sync::{Arc, Mutex};
use tokio_tungstenite::tungstenite::Message as Ws;

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
    use futures::StreamExt;
    use tokio_tungstenite::{WebSocketStream, tungstenite::protocol::Role};

    // A tiny duplex capacity gives actual WebSocket writes sustained
    // backpressure without relying on TCP buffer sizes or a real network.
    let (client, server) = tokio::io::duplex(1024);
    let client = WebSocketStream::from_raw_socket(client, Role::Client, None).await;
    let mut server = WebSocketStream::from_raw_socket(server, Role::Server, None).await;
    let (tx, rx) = mpsc::unbounded_channel();
    let bulk = "x".repeat(CHUNK_BYTES * 8);
    tx.send(Ws::Text(bulk.clone().into())).unwrap();
    let writer = tokio::spawn(write(client, rx, true));
    let first = server.next().await.unwrap().unwrap();
    let Ws::Binary(first) = first else {
        panic!("chunk")
    };
    let mut decoder = Decoder::default();
    assert!(decoder.chunk(true, &first).unwrap().is_none());
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
                    if let Some(text) = decoder.chunk(true, &bytes).unwrap() {
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
    let observed = observed.lock().unwrap();
    assert!(matches!(&observed[0], Ws::Binary(_)));
    assert!(observed[1].urgent());
    assert!(observed[2].urgent());
    let mut decoder = Decoder::default();
    let mut decoded = Vec::new();
    for message in observed.iter() {
        match message {
            Ws::Binary(bytes) => {
                if let Some(text) = decoder.chunk(true, bytes).unwrap() {
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

#[test]
fn corrupt_unnegotiated_oversized_and_interleaved_chunks_fail_closed() {
    let chunk = |total: u32, offset: u32, data: &[u8]| {
        [
            total.to_be_bytes().as_slice(),
            offset.to_be_bytes().as_slice(),
            data,
        ]
        .concat()
    };
    let first = chunk((CHUNK_BYTES + 1) as u32, 0, b"x");
    assert!(Decoder::default().chunk(false, &first).is_err());
    assert!(Decoder::default().chunk(true, &[0; 8]).is_err());
    assert!(
        Decoder::default()
            .chunk(true, &chunk(u32::MAX, 0, b"x"))
            .is_err()
    );
    assert!(
        Decoder::default()
            .chunk(true, &chunk((CHUNK_BYTES + 1) as u32, 1, b"x"))
            .is_err()
    );
    let mut decoder = Decoder::default();
    assert!(decoder.chunk(true, &first).unwrap().is_none());
    assert!(decoder.text("application data").is_err());
    assert!(decoder.chunk(true, &first).is_err());
    assert!(
        decoder
            .chunk(true, &chunk((CHUNK_BYTES + 2) as u32, 1, b"x"))
            .is_err()
    );
}
