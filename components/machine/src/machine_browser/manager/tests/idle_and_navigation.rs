//! Supervisor-initiated idle stops that a concurrent attach renews, and
//! launcher navigation failures of an owner's tab navigation.

use super::*;

/// A detached interval that began longer than the grace ago.
fn expired_since(instance: &Instance) -> Instant {
    Instant::now()
        .checked_sub(instance.settings.hibernate_after + Duration::from_secs(1))
        .unwrap()
}

#[test]
fn an_idle_stop_requires_the_same_expired_detached_interval() {
    let grace = Duration::from_secs(120);
    let now = Instant::now();
    let since = now.checked_sub(grace + Duration::from_secs(1)).unwrap();
    let live = |phase, detached_since, attached: bool| Live {
        phase,
        geometry: Geometry {
            width: 1280,
            height: 800,
        },
        verified: Verified::unpinned(),
        attachment: attached.then(|| Attachment {
            id: "a1".into(),
            lease_expires: now + Duration::from_secs(30),
            stream: None,
        }),
        detached_since,
        last_snapshot: None,
        last_periodic: now,
        last_scan: now,
        scan_denied: 0,
        navigation_unsettled: false,
    };
    assert!(supervision::still_idle(
        &live(Phase::Detached, Some(since), false),
        grace,
        since
    ));
    // Attached meanwhile, or stopping: not idle.
    assert!(!supervision::still_idle(
        &live(Phase::Attached, None, true),
        grace,
        since
    ));
    assert!(!supervision::still_idle(
        &live(Phase::Stopping, Some(since), false),
        grace,
        since
    ));
    // Attached and detached again: a new interval that has not expired.
    assert!(!supervision::still_idle(
        &live(Phase::Detached, Some(now), false),
        grace,
        since
    ));
}

#[tokio::test]
async fn a_grace_stop_never_waits_for_the_lifecycle_lock_and_an_attach_renews_it() {
    let mut fixture = fixture().await;
    fixture.units.make_stoppable("cowboy-browser-inst-1");
    let since = expired_since(&fixture.instance);
    fixture.instance.live.lock().detached_since = Some(since);
    // The account lifecycle lock is held (a snapshot being published, a
    // reset): supervision puts the stop off instead of waiting for it.
    let slot = fixture.manager.slot(&fixture.account);
    let held = slot.lifecycle.lock().await;
    let gone = tokio::time::timeout(
        Duration::from_secs(5),
        fixture.manager.supervise(&fixture.instance),
    )
    .await
    .expect("supervision waited for the lifecycle lock");
    assert!(!gone);
    assert_eq!(fixture.instance.live.lock().phase, Phase::Detached);
    // A viewer attaches meanwhile (attach does not take the lifecycle lock).
    {
        let mut live = fixture.instance.live.lock();
        live.attachment = Some(Attachment {
            id: "a1".into(),
            lease_expires: Instant::now() + Duration::from_secs(30),
            stream: None,
        });
        live.phase = Phase::Attached;
        live.detached_since = None;
    }
    drop(held);
    // The idle stop that had been due is abandoned at its transition: the
    // detached interval it was decided for is over.
    let renewed = fixture
        .manager
        .stop_between_copies(
            &fixture.instance,
            Reason::Grace,
            LifecycleWait::Never,
            || Ok(()),
            |live| {
                if supervision::still_idle(live, fixture.instance.settings.hibernate_after, since) {
                    Ok(())
                } else {
                    Err(supervision::IdleRenewed)
                }
            },
        )
        .await;
    assert!(matches!(renewed, Err(supervision::IdleRenewed)));
    assert!(!fixture.manager.supervise(&fixture.instance).await);
    {
        let live = fixture.instance.live.lock();
        assert_eq!(live.phase, Phase::Attached);
        assert_eq!(
            live.attachment
                .as_ref()
                .map(|attachment| attachment.id.as_str()),
            Some("a1")
        );
    }
    assert!(
        fixture
            .manager
            .slot(&fixture.account)
            .instance
            .lock()
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &fixture.instance))
    );
    // Nothing reached the launcher.
    let mut probe = [0_u8; 1];
    assert!(!matches!(
        tokio::time::timeout(
            Duration::from_millis(200),
            tokio::io::AsyncReadExt::read(&mut fixture.launcher, &mut probe),
        )
        .await,
        Ok(Ok(1))
    ));
}

/// Answer the next launcher command, which must be a navigation to `url`.
async fn answer_navigation(launcher: &mut tokio::net::UnixStream, url: &str, ok: bool) {
    let mut reader = tokio::io::BufReader::new(&mut *launcher);
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(5), reader.read_line(&mut line))
        .await
        .expect("no navigation reached the launcher")
        .unwrap();
    let command: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(command["command"], "navigate");
    assert_eq!(command["url"], url);
    let reply = if ok {
        format!("{{\"id\":{},\"ok\":true}}\n", command["id"])
    } else {
        format!(
            "{{\"id\":{},\"ok\":false,\"error\":\"navigation refused\",\"kind\":\"internal\"}}\n",
            command["id"]
        )
    };
    launcher.write_all(reply.as_bytes()).await.unwrap();
}

fn reference(fixture: &Fixture) -> InstanceRef {
    InstanceRef {
        account_id: fixture.account.clone(),
        instance_id: fixture.instance.instance_id.clone(),
        incarnation: fixture.instance.incarnation,
    }
}

fn spawn_navigation(
    fixture: &Fixture,
    request: &str,
    url: &str,
) -> tokio::task::JoinHandle<Result<StateEvent, String>> {
    let manager = Arc::clone(&fixture.manager);
    let reference = reference(fixture);
    let (request, url) = (request.to_owned(), url.to_owned());
    tokio::spawn(async move { manager.navigate(&request, &reference, &url, None).await })
}

#[tokio::test]
async fn a_rejected_navigation_fails_and_a_later_one_succeeds() {
    let mut fixture = fixture().await;
    fixture
        .manager
        .trust_confinement
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let url = "https://claude.ai/oauth/authorize?fixture";

    // The launcher refuses: the navigation reports `failed` on the running
    // browser, which keeps running.
    let first = spawn_navigation(&fixture, "n1", url);
    answer_navigation(&mut fixture.launcher, url, false).await;
    let refused = first.await.unwrap().unwrap();
    assert_eq!(refused.state, State::Detached);
    assert_eq!(refused.reason, Some(Reason::Failed));
    assert!(fixture.instance.live.lock().navigable());

    // The owner navigates again; this one lands.
    let second = spawn_navigation(&fixture, "n2", url);
    answer_navigation(&mut fixture.launcher, url, true).await;
    let shown = second.await.unwrap().unwrap();
    assert_eq!(shown.reason, None);
}

#[tokio::test]
async fn an_unanswered_navigation_stops_the_instance_instead_of_reusing_it() {
    let fixture = fixture().await;
    fixture
        .manager
        .trust_confinement
        .store(true, std::sync::atomic::Ordering::SeqCst);
    // The launcher reads the navigation but never answers: its helper may
    // still navigate later, so the instance is not reused.
    let answer = fixture
        .manager
        .navigate("n1", &reference(&fixture), "https://example.com/a", None)
        .await
        .unwrap();
    assert_ne!(answer.reason, None);
    assert_eq!(fixture.instance.live.lock().phase, Phase::Stopping);
}

#[tokio::test]
async fn a_navigation_waiting_on_an_unanswered_one_never_reaches_the_launcher() {
    let mut fixture = fixture().await;
    fixture
        .manager
        .trust_confinement
        .store(true, std::sync::atomic::Ordering::SeqCst);
    // An owner navigation reaches the launcher, which never answers.
    let first = spawn_navigation(&fixture, "n1", "https://example.com/tab");
    let second_navigation = {
        let manager = Arc::clone(&fixture.manager);
        let reference = reference(&fixture);
        move || {
            tokio::spawn(async move {
                manager
                    .navigate("n2", &reference, "https://example.com/other", None)
                    .await
            })
        }
    };
    let mut reader = tokio::io::BufReader::new(&mut fixture.launcher);
    let mut line = String::new();
    tokio::time::timeout(Duration::from_secs(5), reader.read_line(&mut line))
        .await
        .expect("the navigation did not reach the launcher")
        .unwrap();
    // A second navigation waits for the navigation lock meanwhile.
    let second = second_navigation();
    let refused = tokio::time::timeout(Duration::from_secs(10), second)
        .await
        .expect("the second navigation did not finish")
        .unwrap()
        .unwrap();
    assert_ne!(
        refused.reason, None,
        "an unsettled instance navigated again"
    );
    let _ = tokio::time::timeout(Duration::from_secs(10), first).await;
    assert_eq!(fixture.instance.live.lock().phase, Phase::Stopping);
    // Nothing further reached the launcher.
    line.clear();
    let read = tokio::time::timeout(Duration::from_millis(200), reader.read_line(&mut line)).await;
    assert!(read.is_err() || !line.contains("other"));
}

#[tokio::test]
async fn a_navigation_fenced_to_a_replaced_attachment_never_reaches_the_launcher() {
    let mut fixture = fixture().await;
    fixture
        .manager
        .trust_confinement
        .store(true, std::sync::atomic::Ordering::SeqCst);
    {
        let mut live = fixture.instance.live.lock();
        live.attachment = Some(Attachment {
            id: "a2".into(),
            lease_expires: Instant::now() + Duration::from_secs(30),
            stream: None,
        });
        live.phase = Phase::Attached;
        live.detached_since = None;
    }
    // The tapped viewer's attachment "a1" was taken over by "a2".
    let refused = fixture
        .manager
        .navigate(
            "n1",
            &reference(&fixture),
            "https://example.com/a",
            Some("a1"),
        )
        .await
        .unwrap();
    assert_eq!(refused.reason, Some(Reason::Superseded));
    // The current attachment navigates; the launcher sees only this URL.
    let url = "https://example.com/b";
    let manager = Arc::clone(&fixture.manager);
    let target = reference(&fixture);
    let current =
        tokio::spawn(async move { manager.navigate("n2", &target, url, Some("a2")).await });
    answer_navigation(&mut fixture.launcher, url, true).await;
    let answer = current.await.unwrap().unwrap();
    assert_eq!(answer.reason, None);
}
