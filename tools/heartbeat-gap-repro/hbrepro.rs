// Heartbeat-gap reproduction harness (docs/heartbeat-gap-repro.md).
//
// A std-only stand-in for the worker -> broker heartbeat path. It uses the
// runtime wire's framing (4-byte big-endian length + JSON) and the worker's
// shape: one writer shared by a 10 s heartbeat and bulk event frames, so a
// heartbeat can wait behind a large frame exactly as `send_outbox` makes it.
//
//   hbrepro broker <socket> <read_delay_us_per_64k>
//   hbrepro worker <socket> <interval_ms> <bulk_bytes> <bulk_every_ms> <alloc_bytes_per_beat>
//
// The broker prints one JSON line per heartbeat with the receive gap; the
// worker reports how late each heartbeat left (send lateness) and how long the
// write took, so the three candidates can be told apart: the sender was not
// scheduled, the write was blocked, or the reader was slow.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn wall_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn arg(args: &[String], index: usize) -> u64 {
    args.get(index)
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| panic!("argument {index} must be an integer"))
}

fn write_frame(stream: &mut UnixStream, payload: &[u8]) -> std::io::Result<()> {
    stream.write_all(&(payload.len() as u32).to_be_bytes())?;
    stream.write_all(payload)
}

fn broker(args: &[String]) {
    let path = &args[2];
    let delay = Duration::from_micros(arg(args, 3));
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path).expect("bind");
    let (mut stream, _) = listener.accept().expect("accept");
    let mut last: Option<Instant> = None;
    let mut chunk = vec![0_u8; 64 * 1024];
    loop {
        let mut header = [0_u8; 4];
        if stream.read_exact(&mut header).is_err() {
            return;
        }
        let mut remaining = u32::from_be_bytes(header) as usize;
        let mut payload = Vec::with_capacity(remaining.min(4096));
        while remaining > 0 {
            let take = remaining.min(chunk.len());
            if stream.read_exact(&mut chunk[..take]).is_err() {
                return;
            }
            if payload.len() < 4096 {
                payload.extend_from_slice(&chunk[..take.min(4096 - payload.len())]);
            }
            remaining -= take;
            if !delay.is_zero() {
                std::thread::sleep(delay);
            }
        }
        if !payload.starts_with(b"{\"type\":\"heartbeat\"") {
            continue;
        }
        let now = Instant::now();
        let gap = last.map_or(0, |at| now.duration_since(at).as_millis());
        last = Some(now);
        let text = String::from_utf8_lossy(&payload);
        println!(
            "{{\"event\":\"heartbeat\",\"recv_ms\":{},\"gap_ms\":{gap},\"frame\":{text}}}",
            wall_ms()
        );
    }
}

fn worker(args: &[String]) {
    let path = &args[2];
    let interval = Duration::from_millis(arg(args, 3));
    let bulk_bytes = arg(args, 4) as usize;
    let bulk_every = Duration::from_millis(arg(args, 5));
    let alloc = arg(args, 6) as usize;
    let stream = loop {
        match UnixStream::connect(path) {
            Ok(stream) => break stream,
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    };
    let writer = Arc::new(Mutex::new(stream));
    if bulk_bytes > 0 {
        let writer = Arc::clone(&writer);
        std::thread::spawn(move || {
            let body = format!("{{\"type\":\"event\",\"pad\":\"{}\"}}", "x".repeat(bulk_bytes));
            loop {
                std::thread::sleep(bulk_every);
                let mut stream = writer.lock().unwrap();
                if write_frame(&mut stream, body.as_bytes()).is_err() {
                    return;
                }
            }
        });
    }
    let start = Instant::now();
    for seq in 1_u64.. {
        let due = start + interval * u32::try_from(seq).unwrap_or(u32::MAX);
        if let Some(wait) = due.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        }
        let woke = Instant::now();
        // The real worker allocates per beat (serialization, outbox clone):
        // touch fresh pages so memory.high throttling, if any, applies here.
        let mut scratch = vec![0_u8; alloc];
        for page in scratch.iter_mut().step_by(4096) {
            *page = 1;
        }
        std::hint::black_box(&scratch);
        let allocated = Instant::now();
        let mut stream = writer.lock().unwrap();
        let locked = Instant::now();
        let frame = format!(
            "{{\"type\":\"heartbeat\",\"seq\":{seq},\"late_ms\":{},\"alloc_ms\":{},\"lock_ms\":{}",
            woke.saturating_duration_since(due).as_millis(),
            allocated.duration_since(woke).as_millis(),
            locked.duration_since(allocated).as_millis(),
        );
        // write_ms is only known after the write; send it in the next beat.
        let payload = format!("{frame},\"sent_ms\":{}}}", wall_ms());
        let began = Instant::now();
        if write_frame(&mut stream, payload.as_bytes()).is_err() {
            return;
        }
        drop(stream);
        eprintln!(
            "{{\"event\":\"sent\",\"seq\":{seq},\"write_ms\":{}}}",
            began.elapsed().as_millis()
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("broker") => broker(&args),
        Some("worker") => worker(&args),
        _ => {
            eprintln!("usage: hbrepro broker|worker ...");
            std::process::exit(2);
        }
    }
}
