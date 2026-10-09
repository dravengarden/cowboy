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
    write(Box::pin(sink), rx, features, None).await.unwrap();
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
    write(Box::pin(sink), rx, true, None).await.unwrap();
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
    write(Box::pin(sink), rx, true, None).await.unwrap();
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
    let writer = tokio::spawn(write(client, rx, features, None));
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
    let writer = tokio::spawn(write(Box::pin(sink), rx, true, None));
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
    write(Box::pin(sink), rx, false, None).await.unwrap();
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

fn incompressible(bytes: usize) -> String {
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    (0..bytes / 16)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            format!("{state:016x}")
        })
        .collect()
}

const REPLY: &str = "{\"type\":\"runtime\",\"frame\":{\"type\":\"execution_reply\",\"reply\":{}}}";
const EVENT: &str =
    "{\"type\":\"runtime\",\"frame\":{\"type\":\"worker_event\",\"runtime_seq\":2}}";

#[tokio::test]
async fn paced_bulk_waits_for_credit_while_execution_replies_overtake() {
    let (tx, rx) = mpsc::unbounded_channel();
    let output = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&output);
    let sink = futures::sink::unfold((), move |(), frame: Ws| {
        captured.lock().unwrap().push(frame);
        async { Ok::<_, std::io::Error>(()) }
    });
    let credit = Arc::new(Credit::default());
    let bulk = incompressible(1024 * 1024);
    tx.send(Ws::Text(bulk.clone().into())).unwrap();
    tx.send(Ws::Text(EVENT.into())).unwrap();
    let writer = tokio::spawn(write(
        Box::pin(sink),
        rx,
        (true, true, true),
        Some(Arc::clone(&credit)),
    ));
    let chunk_bytes = |frames: &[Ws]| {
        frames
            .iter()
            .filter_map(|frame| match frame {
                Ws::Binary(bytes) => Some(bytes.len() as u64 - 8),
                _ => None,
            })
            .sum::<u64>()
    };
    let settle = || tokio::time::sleep(Duration::from_millis(50));
    // Compression runs off the executor first; then, without credit, only
    // one window of chunks leaves the writer.
    tokio::time::timeout(Duration::from_secs(10), async {
        while chunk_bytes(&output.lock().unwrap()) < CREDIT_WINDOW {
            settle().await;
        }
    })
    .await
    .unwrap();
    settle().await;
    let sent = chunk_bytes(&output.lock().unwrap());
    assert!(
        sent >= CREDIT_WINDOW && sent < CREDIT_WINDOW + CHUNK_BYTES as u64,
        "{sent}"
    );
    // An execution reply is not held behind the waiting bulk; a worker event
    // (ordered by runtime sequence) is.
    tx.send(Ws::Text(REPLY.into())).unwrap();
    settle().await;
    {
        let frames = output.lock().unwrap();
        assert_eq!(
            frames.last().and_then(|frame| frame.to_text().ok()),
            Some(REPLY)
        );
        assert!(
            !frames
                .iter()
                .any(|frame| frame.to_text().ok() == Some(EVENT))
        );
    }
    // Credits release the rest; the event follows the complete bulk frame.
    let mut acknowledged;
    loop {
        acknowledged = chunk_bytes(&output.lock().unwrap());
        assert!(credit.observe(format!("cowboy-credit-v1:{acknowledged}").as_bytes()));
        settle().await;
        if output
            .lock()
            .unwrap()
            .last()
            .and_then(|frame| frame.to_text().ok())
            == Some(EVENT)
        {
            break;
        }
        assert!(chunk_bytes(&output.lock().unwrap()) > acknowledged);
    }
    drop(tx);
    writer.await.unwrap().unwrap();
    let frames = Arc::try_unwrap(output).unwrap().into_inner().unwrap();
    let mut decoder = Decoder::new((true, true, true));
    let mut decoded = Vec::new();
    for frame in frames {
        match frame {
            Ws::Binary(bytes) => {
                if let Some(text) = decoder.chunk((true, true), &bytes).await.unwrap() {
                    decoded.push(text);
                }
                let Some(Ws::Pong(payload)) = decoder.credit::<Ws>() else {
                    panic!("paced decoder returns credit")
                };
                assert!(Credit::default().observe(&payload));
            }
            Ws::Text(text) => {
                decoder.text(&text).unwrap();
                decoded.push(text.to_string());
            }
            _ => panic!("unexpected frame"),
        }
    }
    assert!(acknowledged > 0);
    assert_eq!(decoded, [REPLY.to_owned(), bulk, EVENT.to_owned()]);
}

#[tokio::test]
async fn only_negotiated_pacing_interleaves_or_returns_credit() {
    let bulk = incompressible(64 * 1024);
    let frames = capture((true, false), vec![Ws::Text(bulk.into())]).await;
    let Ws::Binary(first) = &frames[0] else {
        panic!("chunk")
    };
    let mut unpaced = Decoder::default();
    assert!(unpaced.chunk(true, first).await.unwrap().is_none());
    assert!(unpaced.credit::<Ws>().is_none());
    assert!(unpaced.text(REPLY).is_err());
    assert!(
        unpaced
            .text("{\"type\":\"heartbeat\",\"sent_at_ms\":1}")
            .is_ok()
    );
    let mut paced = Decoder::new((true, false, true));
    assert!(paced.chunk(true, first).await.unwrap().is_none());
    assert_eq!(
        paced.credit::<Ws>(),
        Some(Ws::Pong(
            format!("cowboy-credit-v1:{}", first.len() - 8)
                .into_bytes()
                .into()
        ))
    );
    assert!(paced.text(REPLY).is_ok());
    assert!(paced.text(EVENT).is_err());
    // Pacing requires chunking.
    assert!(!Features::from((false, true, true)).paced);
    let credit = Credit::default();
    assert!(!credit.observe(b"ordinary pong"));
    assert!(!credit.observe(b"cowboy-credit-v1:-1"));
    assert!(credit.observe(b"cowboy-credit-v1:7"));
    assert!(credit.observe(b"cowboy-credit-v1:3"));
    assert_eq!(credit.acknowledged(), 7);
}

#[tokio::test(start_paused = true)]
async fn paced_bulk_without_credit_progress_fails_the_connection() {
    let (tx, rx) = mpsc::unbounded_channel();
    let sink = futures::sink::unfold((), |(), _frame: Ws| async { Ok::<_, std::io::Error>(()) });
    tx.send(Ws::Text(incompressible(512 * 1024).into()))
        .unwrap();
    let error = write(
        Box::pin(sink),
        rx,
        (true, false, true),
        Some(Arc::new(Credit::default())),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("credit stalled"));
    drop(tx);
}

#[tokio::test]
async fn paced_websocket_credits_flow_back_over_the_same_connection() {
    use futures::StreamExt;
    use tokio_tungstenite::{WebSocketStream, tungstenite::protocol::Role};

    let (client, server) = tokio::io::duplex(64 * 1024);
    let client = WebSocketStream::from_raw_socket(client, Role::Client, None).await;
    let server = WebSocketStream::from_raw_socket(server, Role::Server, None).await;
    let (client_sink, mut client_stream) = client.split();
    let (mut server_sink, mut server_stream) = server.split();
    let credit = Arc::new(Credit::default());
    let (tx, rx) = mpsc::unbounded_channel();
    let bulk = incompressible(2 * 1024 * 1024);
    tx.send(Ws::Text(bulk.clone().into())).unwrap();
    tx.send(Ws::Text(REPLY.into())).unwrap();
    drop(tx);
    let writer = tokio::spawn(write(
        client_sink,
        rx,
        (true, true, true),
        Some(Arc::clone(&credit)),
    ));
    // The writer's own read side records the receiver's credits.
    let observed = Arc::clone(&credit);
    let reader = tokio::spawn(async move {
        while let Some(Ok(message)) = client_stream.next().await {
            if let Ws::Pong(payload) = message {
                assert!(observed.observe(&payload));
            }
        }
    });
    let mut decoder = Decoder::new((true, true, true));
    let mut decoded = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), async {
        while decoded.len() < 2 {
            match server_stream.next().await.unwrap().unwrap() {
                Ws::Binary(bytes) => {
                    if let Some(text) = decoder.chunk((true, true), &bytes).await.unwrap() {
                        decoded.push(text);
                    }
                    server_sink.send(decoder.credit().unwrap()).await.unwrap();
                }
                Ws::Text(text) => {
                    decoder.text(&text).unwrap();
                    decoded.push(text.to_string());
                }
                _ => panic!("unexpected frame"),
            }
        }
    })
    .await
    .unwrap();
    writer.await.unwrap().unwrap();
    // Queued before any chunk, the reply overtakes the whole transfer.
    assert_eq!(decoded, [REPLY.to_owned(), bulk]);
    drop(server_sink);
    reader.abort();
}

/// Bytes written on one side reach the other after a delay, at most `rate`
/// bytes/s, with an unbounded queue in between: the overlay proxy's buffers.
async fn link<R, W>(mut from: R, mut to: W, rate: u64, delay: Duration)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (tx, mut rx) = mpsc::unbounded_channel::<(tokio::time::Instant, Vec<u8>)>();
    tokio::spawn(async move {
        let mut buffer = vec![0; 4096];
        while let Ok(read) = from.read(&mut buffer).await {
            if read == 0
                || tx
                    .send((tokio::time::Instant::now(), buffer[..read].to_vec()))
                    .is_err()
            {
                break;
            }
        }
    });
    let mut free_at = tokio::time::Instant::now();
    while let Some((sent, bytes)) = rx.recv().await {
        free_at = free_at.max(sent) + Duration::from_secs_f64(bytes.len() as f64 / rate as f64);
        tokio::time::sleep_until(free_at.max(sent + delay)).await;
        if to.write_all(&bytes).await.is_err() {
            break;
        }
    }
}

async fn small_request_delays(paced: bool) -> Vec<Duration> {
    use futures::StreamExt;
    use tokio_tungstenite::{WebSocketStream, tungstenite::protocol::Role};

    let (machine, machine_link) = tokio::io::duplex(64 * 1024);
    let (controller, controller_link) = tokio::io::duplex(64 * 1024);
    let (machine_read, machine_write) = tokio::io::split(machine_link);
    let (controller_read, controller_write) = tokio::io::split(controller_link);
    // The measured OVH overlay: ~100 KiB/s upstream, ~200 ms round trip.
    tokio::spawn(link(
        machine_read,
        controller_write,
        100 * 1024,
        Duration::from_millis(100),
    ));
    tokio::spawn(link(
        controller_read,
        machine_write,
        200 * 1024,
        Duration::from_millis(100),
    ));
    let machine = WebSocketStream::from_raw_socket(machine, Role::Client, None).await;
    let controller = WebSocketStream::from_raw_socket(controller, Role::Server, None).await;
    let (machine_sink, mut machine_stream) = machine.split();
    let (mut controller_sink, mut controller_stream) = controller.split();
    let features = (true, false, paced);
    let credit = Arc::new(Credit::default());
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(write(machine_sink, rx, features, Some(Arc::clone(&credit))));
    tokio::spawn(async move {
        while let Some(Ok(message)) = machine_stream.next().await {
            if let Ws::Pong(payload) = message {
                credit.observe(&payload);
            }
        }
    });
    let started = tokio::time::Instant::now();
    // A full hook transcript copy (1.4 MiB), then one small execution request
    // every 200 ms while other sessions keep working.
    tx.send(Ws::Text(incompressible(1400 * 1024).into()))
        .unwrap();
    let producer = tokio::spawn(async move {
        for index in 0..40_u32 {
            tokio::time::sleep_until(started + Duration::from_millis(200) * index).await;
            let request = format!(
                "{{\"type\":\"runtime\",\"frame\":{{\"type\":\"execution_request\",\"index\":{index}}}}}"
            );
            tx.send(Ws::Text(request.into())).unwrap();
        }
        tx
    });
    let mut decoder = Decoder::new(features);
    let mut delays = Vec::new();
    while delays.len() < 40 {
        match controller_stream.next().await.unwrap().unwrap() {
            Ws::Binary(bytes) => {
                decoder.chunk(features, &bytes).await.unwrap();
                if let Some(credit) = decoder.credit() {
                    controller_sink.send(credit).await.unwrap();
                }
            }
            Ws::Text(text) => {
                decoder.text(&text).unwrap();
                let index: u32 = text
                    .rsplit_once(':')
                    .unwrap()
                    .1
                    .trim_end_matches('}')
                    .parse()
                    .unwrap();
                delays.push(started.elapsed() - Duration::from_millis(200) * index);
            }
            _ => {}
        }
    }
    drop(producer.await.unwrap());
    delays
}

#[tokio::test(start_paused = true)]
async fn pacing_bounds_small_request_delay_behind_bulk_on_a_slow_link() {
    let percentile = |delays: &mut Vec<Duration>, p: usize| {
        delays.sort();
        delays[(delays.len() - 1) * p / 100]
    };
    let mut fifo = small_request_delays(false).await;
    let mut paced = small_request_delays(true).await;
    eprintln!(
        "small request delay behind 1.4 MiB at 100 KiB/s: unpaced p50 {:?} max {:?}; paced p50 {:?} max {:?}",
        percentile(&mut fifo, 50),
        percentile(&mut fifo, 100),
        percentile(&mut paced, 50),
        percentile(&mut paced, 100),
    );
    assert!(percentile(&mut fifo, 50) > Duration::from_secs(5));
    assert!(percentile(&mut paced, 100) < Duration::from_secs(3));
}
