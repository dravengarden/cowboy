use super::*;
use tokio::io::AsyncWriteExt as _;

fn frame_bytes(frame: &Frame) -> Vec<u8> {
    let payload = serde_json::to_vec(frame).unwrap();
    let mut bytes = u32::try_from(payload.len()).unwrap().to_be_bytes().to_vec();
    bytes.extend(payload);
    bytes
}

/// A hub whose clock started `age` ago, so silences can exceed the threshold
/// without sleeping.
fn aged_hub(age: Duration) -> Hub {
    Hub {
        origin: Instant::now() - age,
        ..Hub::default()
    }
}

fn take_rx(hub: &Hub) -> mpsc::Receiver<Box<GapNotice>> {
    hub.rx.lock().take().unwrap()
}

fn next_gap(rx: &mut mpsc::Receiver<Box<GapNotice>>) -> GapNotice {
    *rx.try_recv().expect("gap notice expected")
}

fn ms(at: Duration) -> u64 {
    u64::try_from(at.as_millis()).unwrap()
}

#[tokio::test]
async fn read_frame_keeps_wire_framing_and_stamps_progress() {
    let hub = Hub::default();
    let connection = hub.connect("sess-1", 7, "epoch-1", None, None);
    let (mut tx, mut rx) = tokio::io::duplex(1 << 16);
    tx.write_all(&frame_bytes(&Frame::Heartbeat)).await.unwrap();
    let frame = read_frame(&mut rx, &connection).await.unwrap();
    assert!(matches!(frame, Some(Frame::Heartbeat)));
    let evidence = connection.evidence(connection.now_ms());
    assert_eq!(Phase::name(evidence.phase), "handling");
    assert!(evidence.frame_age_ms.is_some());
    assert!(evidence.progress_age_ms.is_some());
    assert_eq!(evidence.frames, 1);
    // A partial header then EOF ends the stream, as `runtime_wire::read_frame`.
    tx.write_all(&[0, 0]).await.unwrap();
    drop(tx);
    assert!(read_frame(&mut rx, &connection).await.unwrap().is_none());
}

#[tokio::test]
async fn read_frame_reports_payload_progress_while_blocked() {
    let hub = Hub::default();
    let connection = hub.connect("sess-1", 7, "epoch-1", None, None);
    let (mut tx, mut rx) = tokio::io::duplex(1 << 16);
    let bytes = frame_bytes(&Frame::Heartbeat);
    tx.write_all(&bytes[..6]).await.unwrap();
    let reading = Arc::clone(&connection);
    let reader = tokio::spawn(async move { read_frame(&mut rx, &reading).await });
    tokio::time::sleep(Duration::from_millis(50)).await;
    let evidence = connection.evidence(connection.now_ms());
    assert_eq!(Phase::name(evidence.phase), "payload");
    assert_eq!(evidence.declared as usize, bytes.len() - 4);
    assert_eq!(evidence.read, 2);
    assert!(evidence.current_frame_ms.is_some());
    assert_eq!(evidence.waiting_ms, None);
    tx.write_all(&bytes[6..]).await.unwrap();
    assert!(matches!(
        reader.await.unwrap().unwrap(),
        Some(Frame::Heartbeat)
    ));
    let evidence = connection.evidence(connection.now_ms());
    assert!(evidence.slowest_frame_ms >= 40, "{evidence:?}");
    assert_eq!(evidence.slowest_frame_bytes as usize, bytes.len() - 4);
}

#[tokio::test]
async fn oversized_and_truncated_frames_are_errors() {
    let hub = Hub::default();
    let connection = hub.connect("sess-1", 7, "epoch-1", None, None);
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
    let limit = ms(GAP_THRESHOLD);
    assert_eq!(gap_ms(1_000, 1_000 + limit), None);
    assert_eq!(gap_ms(1_000, 1_001 + limit), Some(limit + 1));
    assert_eq!(gap_ms(5_000, 1_000), None);
}

#[test]
fn closing_heartbeat_freezes_the_interval_evidence() {
    let hub = aged_hub(Duration::from_secs(60));
    let mut rx = take_rx(&hub);
    let connection = hub.connect("sess-1", 1, "e1", None, None);
    // A heartbeat 30 s ago, then one slow 32 MiB frame in this interval.
    connection.last_heartbeat.store(30_001, Ordering::Relaxed);
    connection.slowest_frame_ms.store(29_000, Ordering::Relaxed);
    connection
        .slowest_frame_bytes
        .store(32 << 20, Ordering::Relaxed);
    connection.frames.store(2, Ordering::Relaxed);
    connection.heartbeat();
    let gap = next_gap(&mut rx);
    assert_eq!(gap.kind, GapKind::Heartbeat);
    assert!(gap.gap_ms >= 30_000, "{}", gap.gap_ms);
    assert_eq!(gap.evidence.slowest_frame_ms, 29_000);
    assert_eq!(gap.evidence.slowest_frame_bytes, 32 << 20);
    assert_eq!(gap.evidence.frames, 2);
    assert!(gap.since < gap.at);
    // The next interval starts clean.
    assert_eq!(connection.slowest_frame_ms.load(Ordering::Relaxed), 0);
    connection.heartbeat();
    assert!(rx.try_recv().is_err());
}

#[test]
fn sweep_decisions_carry_their_own_measurement() {
    let hub = aged_hub(Duration::from_secs(60));
    let mut rx = take_rx(&hub);
    let connection = hub.connect("sess-1", 1, "e1", None, None);
    connection.frame_done();
    connection.deferred(512, Duration::from_secs(50));
    let gap = next_gap(&mut rx);
    assert_eq!(gap.kind, GapKind::Deferred);
    assert_eq!(gap.unread_bytes, Some(512));
    // Once per frame-silence episode.
    connection.deferred(512, Duration::from_secs(65));
    assert!(rx.try_recv().is_err());
    // The last heartbeat was 60 s ago.
    connection.last_heartbeat.store(1, Ordering::Relaxed);
    connection.isolated("worker heartbeat timed out after 46s", Some(0));
    let gap = next_gap(&mut rx);
    assert_eq!(gap.kind, GapKind::Isolated);
    assert_eq!(gap.unread_bytes, Some(0));
    assert_eq!(
        gap.detail.as_deref(),
        Some("worker heartbeat timed out after 46s")
    );
    assert!(gap.gap_ms >= 59_000, "{}", gap.gap_ms);
}

#[test]
fn threshold_crossings_are_reported_once_per_silence() {
    let hub = aged_hub(Duration::from_secs(60));
    let connection = hub.connect("sess-1", 1, "e1", None, None);
    connection.last_heartbeat.store(30_001, Ordering::Relaxed);
    let gap = connection.check_threshold().expect("30 s silence crosses");
    assert_eq!(gap.kind, GapKind::Threshold);
    assert!(connection.check_threshold().is_none());
    let mut rx = take_rx(&hub);
    connection.heartbeat();
    let closing = next_gap(&mut rx);
    assert_eq!(
        closing.detail.as_deref(),
        Some("closes a reported threshold crossing")
    );
    // A fresh heartbeat leaves nothing to report.
    assert!(connection.check_threshold().is_none());
}

#[test]
fn full_queue_counts_drops_without_blocking() {
    let hub = Hub::default();
    let connection = hub.connect("sess-1", 7, "e1", None, None);
    for _ in 0..(NOTICE_CAPACITY * 2) {
        connection.isolated("flood", None);
    }
    assert_eq!(hub.dropped.load(Ordering::Relaxed), NOTICE_CAPACITY as u64);
}

#[test]
fn identifiers_are_bounded() {
    let hub = Hub::default();
    let long = "s".repeat(10_000);
    let connection = hub.connect(&long, 1, &long, Some(long.clone()), None);
    assert_eq!(connection.session.len(), MAX_IDENTIFIER);
    assert_eq!(connection.epoch.len(), MAX_IDENTIFIER);
    assert_eq!(connection.unit.as_deref().map(str::len), Some(MAX_TEXT));
    assert_eq!(bounded("é".repeat(200).as_str(), 3), ("é".to_owned(), true));
}

#[test]
fn lag_counts_stalls_overlapping_the_gap_and_reports_coverage() {
    let lag = Lag::default();
    let start = Instant::now();
    let at = |ms| start + Duration::from_millis(ms);
    // An on-time tick, a tick due at 1 s that woke 900 ms late, an on-time one.
    lag.record(at(0), at(0));
    lag.record(at(1_000), at(1_900));
    lag.record(at(2_000), at(2_001));
    assert_eq!(lag.max_between(at(0), at(2_000)), (Some(900), true));
    // A gap that closed while the late tick was still waiting counts it.
    assert_eq!(lag.max_between(at(1_200), at(1_500)), (Some(900), true));
    // The probe has not yet woken past the end: partial.
    assert!(!lag.max_between(at(0), at(5_000)).1);
    // Nor reached back to the start.
    assert!(!lag.max_between(start - Duration::from_secs(1), at(2_000)).1);
}

#[test]
fn unit_show_output_requires_a_cgroup_and_an_invocation() {
    let id = "0123456789abcdef0123456789abcdef";
    let (control_group, invocation) =
        parse_unit_show(&format!("ControlGroup=/a/b.service\nInvocationID={id}\n")).unwrap();
    assert_eq!(control_group, "/a/b.service");
    assert_eq!(invocation, id);
    assert!(parse_unit_show(&format!("ControlGroup=\nInvocationID={id}\n")).is_none());
    assert!(parse_unit_show("ControlGroup=/a/b.service\nInvocationID=\n").is_none());
}

fn directory() -> Arc<CgroupDir> {
    let file = std::fs::File::open(std::env::temp_dir()).unwrap();
    Arc::new(std::os::fd::OwnedFd::from(file))
}

fn sampler_for(hub: &Hub) -> (Sampler, mpsc::Receiver<BindJob>) {
    let (jobs, jobs_rx) = mpsc::channel(BIND_QUEUE);
    let sampler = Sampler::new(SamplerLinks {
        instance: 1,
        dropped: Arc::clone(&hub.dropped),
        lag: Arc::new(Lag::default()),
        jobs,
        lane_generation: Arc::new(AtomicU64::new(0)),
        registry: Arc::clone(&hub.registry),
    });
    (sampler, jobs_rx)
}

fn verify(sampler: &mut Sampler, connection: &Arc<Connection>, control_group: &str) {
    sampler.bound.insert(
        connection.connection,
        Bound {
            connection: Arc::downgrade(connection),
            generation: sampler.generation,
            state: BindingState::Verified {
                control_group: control_group.to_owned(),
                invocation: "0".repeat(32),
                directory: directory(),
            },
        },
    );
}

#[test]
fn plan_deduplicates_shared_ancestors_and_reserves_broker_and_host() {
    let hub = Hub::default();
    let one = hub.connect("s1", 1, "e", None, None);
    let two = hub.connect("s2", 2, "e", None, None);
    let (mut sampler, _jobs) = sampler_for(&hub);
    sampler.broker_cgroup = Some("user.slice/app.slice/cowboy-machine.service".into());
    let agents = "/user.slice/cowboy.slice/cowboy-agents.slice";
    verify(&mut sampler, &one, &format!("{agents}/w1.service"));
    verify(&mut sampler, &two, &format!("{agents}/w2.service"));
    let keys: Vec<Key> = sampler.plan().iter().map(target_key).collect();
    let count = |wanted: &Key| keys.iter().filter(|key| *key == wanted).count();
    assert_eq!(count(&Key::Worker(1)), 1);
    assert_eq!(count(&Key::Worker(2)), 1);
    assert_eq!(count(&Key::Ancestor(agents[1..].to_owned())), 1);
    assert_eq!(count(&Key::Ancestor("user.slice".into())), 1);
    assert_eq!(count(&Key::Broker), 1);
    assert_eq!(keys.last(), Some(&Key::Host));
    // A closed connection stops contributing scopes.
    two.close();
    sampler.adopt();
    let keys: Vec<Key> = sampler.plan().iter().map(target_key).collect();
    assert!(!keys.contains(&Key::Worker(2)));
}

#[test]
fn registration_is_adopted_and_binds_fill_the_queue_in_order() {
    let hub = Hub::default();
    let (mut sampler, mut jobs) = sampler_for(&hub);
    let connections: Vec<_> = (0..(BIND_QUEUE as u64 + 3))
        .map(|id| hub.connect("s", id, "e", Some(format!("w{id}.service")), None))
        .collect();
    sampler.adopt();
    assert_eq!(sampler.bound.len(), connections.len());
    sampler.fill();
    let pending = |sampler: &Sampler| {
        sampler
            .bound
            .values()
            .filter(|bound| bound.state.label() == "pending")
            .count()
    };
    assert_eq!(pending(&sampler), BIND_QUEUE);
    // Room frees as the lane takes jobs; the rest are submitted later.
    for _ in 0..3 {
        jobs.try_recv().unwrap();
    }
    sampler.fill();
    assert_eq!(pending(&sampler), connections.len());
}

#[test]
fn stale_bind_results_are_discarded() {
    let hub = Hub::default();
    let connection = hub.connect("s1", 1, "e", None, None);
    let (mut sampler, mut jobs) = sampler_for(&hub);
    sampler.adopt();
    sampler.fill();
    assert!(jobs.try_recv().is_ok());
    assert_eq!(sampler.bound[&1].state.label(), "pending");
    // A kill-switch transition happened while the query ran.
    sampler.generation += 1;
    sampler.bound(BindResult {
        connection: 1,
        generation: 0,
        state: BindingState::Unavailable("unit_query_failed"),
    });
    assert_eq!(sampler.bound[&1].state.label(), "pending");
    drop(connection);
}

#[test]
fn closed_connections_are_not_live() {
    let hub = aged_hub(Duration::from_secs(60));
    let connection = hub.connect("s1", 1, "e", None, None);
    let (mut sampler, _jobs) = sampler_for(&hub);
    sampler.adopt();
    assert_eq!(sampler.live().len(), 1);
    // A queued job or notice may still hold it; closing is what counts.
    let held = Arc::clone(&connection);
    connection.close();
    assert!(sampler.live().is_empty());
    sampler.adopt();
    assert!(sampler.bound.is_empty());
    drop(held);
}

#[test]
fn direct_mode_peers_are_never_pinned() {
    let hub = Hub::default();
    let direct = hub.connect("s1", 1, "e", None, Some(std::process::id()));
    assert!(direct.peer.is_none());
    assert_eq!(hub.pinned.load(Ordering::Relaxed), 0);
}

#[test]
fn records_use_the_periodic_history_around_the_gap() {
    let hub = aged_hub(Duration::from_secs(60));
    let mut rx = take_rx(&hub);
    let connection = hub.connect("s1", 1, "e", Some("w1.service".into()), None);
    let (mut sampler, _jobs) = sampler_for(&hub);
    verify(&mut sampler, &connection, "/cowboy.slice/w1.service");
    connection.last_heartbeat.store(20_001, Ordering::Relaxed);
    connection.heartbeat();
    let gap = next_gap(&mut rx);
    let ring: History = [
        (gap.since - Duration::from_secs(5), Sample::default()),
        (gap.at + Duration::from_secs(5), Sample::default()),
    ]
    .into_iter()
    .collect();
    sampler.history.insert(Key::Worker(1), ring.clone());
    sampler.history.insert(Key::Host, ring);
    sampler.queue(gap);
    assert_eq!(sampler.pending.len(), 1);
    sampler.flush();
    assert!(sampler.pending.is_empty());
    assert_eq!(sampler.gap_ids, 1);
}

#[test]
fn pending_gaps_are_bounded() {
    let hub = Hub::default();
    let connection = hub.connect("s1", 1, "e", None, None);
    let (mut sampler, _jobs) = sampler_for(&hub);
    let notice = || {
        connection.notice(Notice {
            kind: GapKind::Isolated,
            since_ms: 0,
            at_ms: 1,
            gap_ms: 1,
            evidence: Evidence::default(),
            unread_bytes: None,
            detail: None,
        })
    };
    for _ in 0..(MAX_PENDING_GAPS + 3) {
        sampler.queue(notice());
    }
    assert_eq!(sampler.pending.len(), MAX_PENDING_GAPS);
    assert_eq!(sampler.links.dropped.load(Ordering::Relaxed), 3);
}

#[test]
fn a_removed_cgroup_keeps_its_last_readable_counters_as_the_end() {
    let start = Instant::now();
    let at = |secs| start + Duration::from_secs(secs);
    let readable = |some_us| Sample {
        cpu: Some(counters::Psi {
            some_us,
            full_us: None,
        }),
        ..Sample::default()
    };
    let ring: History = [
        (at(0), readable(10)),
        (at(10), readable(30)),
        (at(20), Sample::default()),
    ]
    .into_iter()
    .collect();
    let ((start_at, _), (end_at, end)) = endpoints(&ring, at(5)).unwrap();
    assert_eq!((*start_at, *end_at), (at(0), at(10)));
    assert_eq!(end.cpu.map(|psi| psi.some_us), Some(30));
    assert!(endpoints(&[(at(0), Sample::default())].into_iter().collect(), at(0)).is_none());
}

#[test]
fn signed_offsets_report_partial_coverage() {
    let start = Instant::now();
    let later = start + Duration::from_millis(1500);
    assert_eq!(signed_ms(later, start), 1500);
    assert_eq!(signed_ms(start, later), -1500);
}

#[tokio::test]
async fn direct_mode_and_unknown_peers_are_unavailable() {
    let hub = Hub::default();
    let direct = hub.connect("s1", 1, "e", None, Some(std::process::id()));
    assert_eq!(resolve_binding(&direct).await.label(), "direct_mode");
    let unknown = hub.connect("s2", 2, "e", Some("x.service".into()), None);
    assert_eq!(resolve_binding(&unknown).await.label(), "peer_unknown");
}

#[cfg(target_os = "linux")]
mod linux {
    use super::super::super::liveness_counters as counters;

    #[test]
    fn peer_membership_uses_the_pinned_process() {
        let Some(own) = std::fs::read_to_string("/proc/self/cgroup")
            .ok()
            .and_then(|text| counters::unified_cgroup(&text))
        else {
            return;
        };
        let peer = counters::open_peer(std::process::id()).expect("own /proc entry");
        assert_eq!(counters::peer_member(&peer, &own), Ok(()));
        assert_eq!(
            counters::peer_member(&peer, "/definitely/not/here"),
            Err("peer_outside_unit")
        );
        assert!(counters::open_peer(u32::MAX - 1).is_none());
    }

    #[test]
    fn bounded_reads_refuse_symlinks_and_escapes() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("cpu.pressure"),
            "some avg10=0 avg60=0 avg300=0 total=7\n",
        )
        .unwrap();
        std::os::unix::fs::symlink(
            "/proc/self/environ",
            directory.path().join("memory.pressure"),
        )
        .unwrap();
        let handle = std::os::fd::OwnedFd::from(std::fs::File::open(directory.path()).unwrap());
        let sample = counters::counters(&handle);
        assert_eq!(sample.cpu.map(|psi| psi.some_us), Some(7));
        assert_eq!(sample.memory, None);
        assert!(counters::read_at(&handle, "../etc/passwd").is_err());
    }
}

/// L0-3 overhead: the sampler's per-tick read cost against this host's live
/// Cowboy worker and execution cgroups (read-only). Run with
/// `just test-rust-filter --run-ignored only -E 'test(sampler_cost_on_this_host)' --no-capture`.
#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "measures against this host's live cgroups"]
async fn sampler_cost_on_this_host() {
    let relative = format!(
        "user.slice/user-{uid}.slice/user@{uid}.service/cowboy.slice/cowboy-agents.slice",
        uid = rustix::process::getuid().as_raw()
    );
    let Ok(entries) = std::fs::read_dir(format!("/sys/fs/cgroup/{relative}")) else {
        eprintln!("no {relative} on this host");
        return;
    };
    let mut plan = Vec::new();
    for (id, entry) in entries.flatten().enumerate() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".service") {
            continue;
        }
        if let Ok(directory) = counters::open_cgroup(&format!("/{relative}/{name}")) {
            plan.push(Target::Worker(id as u64, Arc::new(directory)));
        }
    }
    let workers = plan.len();
    for ancestor in counters::ancestors(&format!("{relative}/x.service")) {
        plan.push(Target::Path(Key::Ancestor(ancestor.clone()), ancestor));
    }
    plan.push(Target::Host);
    let mut timings = Vec::new();
    for _ in 0..10 {
        let again = plan
            .iter()
            .map(|target| match target {
                Target::Worker(id, directory) => Target::Worker(*id, Arc::clone(directory)),
                Target::Path(key, path) => Target::Path(key.clone(), path.clone()),
                Target::Host => Target::Host,
            })
            .collect();
        let started = Instant::now();
        let samples = read_plan(again).await;
        timings.push(started.elapsed());
        assert_eq!(samples.len(), plan.len());
    }
    timings.sort();
    eprintln!(
        "sampler tick over {workers} workers + {} shared scopes: median {:?}, max {:?}",
        plan.len() - workers,
        timings[timings.len() / 2],
        timings[timings.len() - 1]
    );
}

mod parsing {
    use super::super::super::liveness_counters::*;
    use std::time::{Duration, Instant};

    #[test]
    fn psi_events_and_limits_parse() {
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
        let cpu = parse_psi("some avg10=0.00 avg60=0.00 avg300=0.00 total=5\n").unwrap();
        assert_eq!(cpu.full_us, None);
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
            ancestors("user.slice/user-1000.slice/x.service"),
            vec!["user.slice/user-1000.slice", "user.slice"]
        );
        assert_eq!(
            unified_cgroup("1:name=systemd:/x\n0::/a/b\n").as_deref(),
            Some("/a/b")
        );
    }

    #[test]
    fn delta_reports_unavailable_fields() {
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
        let change = delta(&before, &after);
        assert_eq!(change.cpu, (60, 3));
        assert_eq!(change.memory, (UNAVAILABLE, UNAVAILABLE));
        assert_eq!(change.events.0, 4);
    }

    #[test]
    fn baseline_prefers_the_sample_before_the_gap() {
        let start = Instant::now();
        let ring: std::collections::VecDeque<_> = (0..5)
            .map(|n| (start + Duration::from_secs(n * 10), ()))
            .collect();
        let at = |secs| start + Duration::from_secs(secs);
        assert_eq!(baseline(&ring, at(25)).map(|(t, _)| *t), Some(at(20)));
        let late: std::collections::VecDeque<_> = [ring[3]].into_iter().collect();
        assert_eq!(baseline(&late, at(5)).map(|(t, _)| *t), Some(at(30)));
    }
}
