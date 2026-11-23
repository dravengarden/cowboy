use super::*;
use tokio::io::AsyncWriteExt as _;

fn frame_bytes(frame: &Frame) -> Vec<u8> {
    let payload = serde_json::to_vec(frame).unwrap();
    let mut bytes = u32::try_from(payload.len()).unwrap().to_be_bytes().to_vec();
    bytes.extend(payload);
    bytes
}

fn hub_connection(hub: &Hub) -> Arc<Connection> {
    hub.connect("sess-1", 7, "epoch-1", None, None)
}

#[tokio::test]
async fn read_frame_keeps_wire_framing_and_stamps_progress() {
    let hub = Hub::default();
    let connection = hub_connection(&hub);
    let (mut tx, mut rx) = tokio::io::duplex(1 << 16);
    tx.write_all(&frame_bytes(&Frame::Heartbeat)).await.unwrap();
    let frame = read_frame(&mut rx, &connection).await.unwrap();
    assert!(matches!(frame, Some(Frame::Heartbeat)));
    let state = connection.snapshot(connection.now_ms());
    assert_eq!(state.phase, "handling");
    assert!(state.frame_age_ms.is_some());
    assert!(state.progress_age_ms.is_some());
    // A partial header then EOF ends the stream, as `runtime_wire::read_frame`.
    tx.write_all(&[0, 0]).await.unwrap();
    drop(tx);
    assert!(read_frame(&mut rx, &connection).await.unwrap().is_none());
}

#[tokio::test]
async fn read_frame_reports_payload_progress_while_blocked() {
    let hub = Hub::default();
    let connection = hub_connection(&hub);
    let (mut tx, mut rx) = tokio::io::duplex(1 << 16);
    let bytes = frame_bytes(&Frame::Heartbeat);
    tx.write_all(&bytes[..6]).await.unwrap();
    let reading = Arc::clone(&connection);
    let reader = tokio::spawn(async move { read_frame(&mut rx, &reading).await });
    tokio::time::sleep(Duration::from_millis(50)).await;
    let state = connection.snapshot(connection.now_ms());
    assert_eq!(state.phase, "payload");
    assert_eq!(state.declared as usize, bytes.len() - 4);
    assert_eq!(state.read, 2);
    tx.write_all(&bytes[6..]).await.unwrap();
    assert!(matches!(
        reader.await.unwrap().unwrap(),
        Some(Frame::Heartbeat)
    ));
}

#[tokio::test]
async fn oversized_and_truncated_frames_are_errors() {
    let hub = Hub::default();
    let connection = hub_connection(&hub);
    let (mut tx, mut rx) = tokio::io::duplex(64);
    let too_large = u32::try_from(MAX_FRAME_BYTES + 1).unwrap().to_be_bytes();
    tx.write_all(&too_large).await.unwrap();
    let error = read_frame(&mut rx, &connection).await.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    let (mut tx, mut rx) = tokio::io::duplex(64);
    tx.write_all(&[0, 0, 0, 10, b'{']).await.unwrap();
    drop(tx);
    let error = read_frame(&mut rx, &connection).await.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
}

#[test]
fn gap_threshold_is_strict() {
    let limit = u64::try_from(GAP_THRESHOLD.as_millis()).unwrap();
    assert_eq!(gap_ms(1_000, 1_000 + limit), None);
    assert_eq!(gap_ms(1_000, 1_001 + limit), Some(limit + 1));
    assert_eq!(gap_ms(5_000, 1_000), None);
}

#[tokio::test]
async fn hub_routes_decisions_to_the_current_connection_only() {
    let hub = Hub::default();
    let first = hub.connect("sess-1", 1, "e1", None, None);
    let second = hub.connect("sess-1", 2, "e2", None, None);
    let mut rx = hub.rx.lock().take().unwrap();
    // Two binds.
    for _ in 0..2 {
        assert!(matches!(rx.try_recv(), Ok(Notice::Bind(_))));
    }
    hub.isolated("sess-1", 1, "stale connection");
    assert!(rx.try_recv().is_err());
    hub.isolated("sess-1", 2, "current connection");
    let Ok(Notice::Gap(gap)) = rx.try_recv() else {
        panic!("isolation notice expected");
    };
    assert_eq!(gap.kind, GapKind::Isolated);
    assert_eq!(gap.connection.connection, 2);
    assert_eq!(gap.detail.as_deref(), Some("current connection"));
    drop((first, second));
}

#[tokio::test]
async fn deferred_isolation_is_recorded_once_per_silence() {
    let hub = Hub::default();
    let connection = hub.connect("sess-1", 1, "e1", None, None);
    let mut rx = hub.rx.lock().take().unwrap();
    assert!(matches!(rx.try_recv(), Ok(Notice::Bind(_))));
    connection.frame_done();
    hub.deferred("sess-1", 512, Duration::from_secs(50));
    assert!(matches!(
        rx.try_recv(),
        Ok(Notice::Gap(GapNotice {
            kind: GapKind::Deferred,
            unread_bytes: Some(512),
            ..
        }))
    ));
    hub.deferred("sess-1", 512, Duration::from_secs(65));
    assert!(rx.try_recv().is_err());
    tokio::time::sleep(Duration::from_millis(2)).await;
    connection.frame_done();
    hub.deferred("sess-1", 64, Duration::from_secs(50));
    assert!(matches!(rx.try_recv(), Ok(Notice::Gap(_))));
}

#[test]
fn full_queue_counts_drops_without_blocking() {
    let hub = Hub::default();
    let connection = hub_connection(&hub);
    for _ in 0..(NOTICE_CAPACITY * 2) {
        hub.isolated("sess-1", 7, "flood");
    }
    // The first slot held the bind notice.
    assert_eq!(
        hub.dropped.load(Ordering::Relaxed),
        (NOTICE_CAPACITY + 1) as u64
    );
    drop(connection);
}

#[test]
fn psi_and_events_parse() {
    let psi = parse_psi(
        "some avg10=0.00 avg60=0.10 avg300=0.20 total=1234\nfull avg10=0.00 avg60=0.00 avg300=0.00 total=99\n",
    )
    .unwrap();
    assert_eq!(
        psi,
        Psi {
            some_us: 1234,
            full_us: Some(99)
        }
    );
    let cpu_v1 = parse_psi("some avg10=0.00 avg60=0.00 avg300=0.00 total=5\n").unwrap();
    assert_eq!(cpu_v1.full_us, None);
    assert_eq!(parse_psi("garbage"), None);
    let events =
        parse_events("low 0\nhigh 12\nmax 3\noom 1\noom_kill 1\noom_group_kill 0\n").unwrap();
    assert_eq!(
        events,
        Events {
            high: 12,
            max: 3,
            oom: 1,
            oom_kill: 1
        }
    );
    assert_eq!(parse_events(""), None);
    assert_eq!(parse_limit("max\n"), Some(u64::MAX));
    assert_eq!(parse_limit("51539607552\n"), Some(51_539_607_552));
    assert_eq!(parse_limit("x"), None);
}

#[test]
fn cgroup_paths_are_validated_and_expanded_nearest_first() {
    assert_eq!(relative_cgroup("/"), None);
    assert_eq!(relative_cgroup("relative"), None);
    assert_eq!(relative_cgroup("/a/../b"), None);
    assert_eq!(relative_cgroup("/a//b"), None);
    assert_eq!(
        ancestry("/user.slice/user-1000.slice/x.service"),
        vec![
            Scope::Cgroup("user.slice/user-1000.slice/x.service".into()),
            Scope::Cgroup("user.slice/user-1000.slice".into()),
            Scope::Cgroup("user.slice".into()),
        ]
    );
    assert_eq!(
        unified_cgroup("1:name=systemd:/x\n0::/a/b\n").as_deref(),
        Some("/a/b")
    );
}

#[test]
fn unit_show_output_requires_a_cgroup_and_an_invocation() {
    let id = "0123456789abcdef0123456789abcdef";
    let parsed =
        parse_unit_show(&format!("ControlGroup=/a/b.service\nInvocationID={id}\n")).unwrap();
    assert_eq!(parsed.control_group, "/a/b.service");
    assert_eq!(parsed.invocation, id);
    assert!(parse_unit_show(&format!("ControlGroup=\nInvocationID={id}\n")).is_none());
    assert!(parse_unit_show("ControlGroup=/a/b.service\nInvocationID=\n").is_none());
}

#[test]
fn scopes_deduplicate_shared_ancestors() {
    let hub = Hub::default();
    let one = hub.connect("s1", 1, "e", None, None);
    let two = hub.connect("s2", 2, "e", None, None);
    let mut sampler = Sampler::new(Arc::new(AtomicU64::new(0)));
    sampler.broker_cgroup = Some("user.slice/app.slice/cowboy-machine.service".into());
    for (connection, unit) in [(&one, "w1.service"), (&two, "w2.service")] {
        sampler.bound.insert(
            connection.connection,
            Bound {
                connection: Arc::downgrade(connection),
                state: BindingState::Verified(Binding {
                    control_group: format!("/user.slice/cowboy-agents.slice/{unit}"),
                    invocation: "0".repeat(32),
                }),
            },
        );
    }
    let scopes = sampler.scopes();
    let count = |wanted: &str| {
        scopes
            .iter()
            .filter(|scope| matches!(scope, Scope::Cgroup(path) if path == wanted))
            .count()
    };
    assert_eq!(count("user.slice/cowboy-agents.slice/w1.service"), 1);
    assert_eq!(count("user.slice/cowboy-agents.slice"), 1);
    assert_eq!(count("user.slice"), 1);
    assert_eq!(count("user.slice/app.slice/cowboy-machine.service"), 1);
    assert_eq!(scopes.last(), Some(&Scope::Host));
    // A closed connection no longer contributes scopes once the sampler has
    // consumed its bind notice.
    let mut rx = hub.rx.lock().take().unwrap();
    while rx.try_recv().is_ok() {}
    drop(two);
    assert_eq!(
        sampler
            .scopes()
            .iter()
            .filter(|scope| matches!(scope, Scope::Cgroup(path) if path.ends_with("w2.service")))
            .count(),
        0
    );
}

#[test]
fn delta_reports_unavailable_fields_and_coverage() {
    let start = Instant::now();
    let before = Sample {
        cpu: Some(Psi {
            some_us: 10,
            full_us: Some(1),
        }),
        events: Some(Events {
            high: 5,
            ..Events::default()
        }),
        ..Sample::default()
    };
    let after = Sample {
        cpu: Some(Psi {
            some_us: 70,
            full_us: Some(4),
        }),
        events: Some(Events {
            high: 9,
            ..Events::default()
        }),
        ..Sample::default()
    };
    let change = delta(
        Some((start, before)),
        (start + Duration::from_secs(30), after),
    );
    assert_eq!(change.coverage_ms, 30_000);
    assert_eq!(change.cpu, (60, 3));
    assert_eq!(change.memory, (UNAVAILABLE, UNAVAILABLE));
    assert_eq!(change.events.0, 4);
    assert_eq!(delta(None, (start, after)).coverage_ms, UNAVAILABLE);
}

#[test]
fn baseline_prefers_the_sample_before_the_gap() {
    let start = Instant::now();
    let ring: VecDeque<_> = (0..5)
        .map(|n| (start + Duration::from_secs(n * 10), Sample::default()))
        .collect();
    let at = |secs| start + Duration::from_secs(secs);
    assert_eq!(baseline(&ring, at(25)).map(|(t, _)| t), Some(at(20)));
    // A gap older than the history uses the oldest sample (partial coverage).
    assert_eq!(
        baseline(&VecDeque::from(vec![ring[3]]), at(5)).map(|(t, _)| t),
        Some(at(30))
    );
}

#[test]
fn bounded_reads_refuse_symlinks_and_escapes() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join("unit")).unwrap();
    std::fs::write(
        directory.path().join("unit/cpu.pressure"),
        "some avg10=0 avg60=0 avg300=0 total=7\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(
        "/proc/self/environ",
        directory.path().join("unit/memory.pressure"),
    )
    .unwrap();
    let root = Root::open(directory.path().to_str().unwrap(), None).unwrap();
    let sample = read_cgroup(&root, "unit");
    assert_eq!(sample.cpu.map(|psi| psi.some_us), Some(7));
    assert_eq!(sample.memory, None);
    assert!(root.read("../etc/passwd").is_err());
    assert!(
        Root::open(
            directory.path().to_str().unwrap(),
            Some(CGROUP2_SUPER_MAGIC)
        )
        .is_err()
    );
}

#[test]
fn peer_membership_uses_the_live_process_cgroup() {
    let Some(own) = std::fs::read_to_string("/proc/self/cgroup")
        .ok()
        .and_then(|text| unified_cgroup(&text))
    else {
        return;
    };
    let pid = std::process::id();
    assert_eq!(peer_member(pid, &own), Ok(()));
    assert_eq!(
        peer_member(pid, "/definitely/not/here"),
        Err("peer_outside_unit")
    );
    assert_eq!(peer_member(u32::MAX - 1, &own), Err("peer_exited"));
}

#[tokio::test]
async fn direct_mode_and_unknown_peers_are_unavailable() {
    assert!(matches!(
        resolve_binding(None, Some(1)).await,
        BindingState::Unavailable("direct_mode")
    ));
    assert!(matches!(
        resolve_binding(Some("x.service"), None).await,
        BindingState::Unavailable("peer_unknown")
    ));
}
