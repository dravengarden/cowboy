//! Supervisor-initiated idle stops that a concurrent attach renews, and
//! launcher navigation failures that a sign-in open reports and retries.

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
        pending_navigation: None,
        failed_navigation: None,
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_attach_while_a_grace_stop_waits_keeps_the_instance_running() {
    let fixture = fixture().await;
    fixture.instance.live.lock().detached_since = Some(expired_since(&fixture.instance));
    // The stop waits for the account lifecycle lock (as it would behind a
    // snapshot publication).
    let slot = fixture.manager.slot(&fixture.account);
    let held = slot.lifecycle.lock().await;
    let supervised = {
        let manager = Arc::clone(&fixture.manager);
        let instance = Arc::clone(&fixture.instance);
        tokio::spawn(async move { manager.supervise(&instance).await })
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!supervised.is_finished());
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
    let gone = tokio::time::timeout(Duration::from_secs(5), supervised)
        .await
        .expect("supervision did not resume")
        .unwrap();
    assert!(!gone, "the abandoned idle stop must keep supervising");
    let live = fixture.instance.live.lock();
    assert_eq!(live.phase, Phase::Attached);
    assert_eq!(
        live.attachment
            .as_ref()
            .map(|attachment| attachment.id.as_str()),
        Some("a1")
    );
    drop(live);
    assert!(
        fixture
            .manager
            .slot(&fixture.account)
            .instance
            .lock()
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &fixture.instance))
    );
}

#[tokio::test]
async fn a_login_ended_stop_spares_an_instance_rebound_to_another_login_meanwhile() {
    let fixture = fixture().await;
    fixture.instance.binding.lock().purpose = Purpose::SignIn {
        login_request_id: "login-a".into(),
    };
    // The stop waits for the account lifecycle lock (as it would behind a
    // snapshot publication or a sign-in open).
    let slot = fixture.manager.slot(&fixture.account);
    let held = slot.lifecycle.lock().await;
    fixture.manager.login_ended("login-a");
    tokio::time::sleep(Duration::from_millis(200)).await;
    // A sign-in open rebinds the same instance to login B meanwhile.
    fixture.instance.binding.lock().purpose = Purpose::SignIn {
        login_request_id: "login-b".into(),
    };
    drop(held);
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_ne!(fixture.instance.live.lock().phase, Phase::Stopping);
    assert!(
        fixture
            .manager
            .slot(&fixture.account)
            .instance
            .lock()
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &fixture.instance))
    );
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

#[tokio::test]
async fn a_rejected_sign_in_navigation_fails_the_open_and_a_repeat_retries_it() {
    let mut fixture = fixture().await;
    fixture
        .manager
        .trust_confinement
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let logins: LoginSessions = Arc::default();
    let (events, _received) = tokio::sync::mpsc::unbounded_channel();
    let _uplink = fixture.manager.attach(
        events,
        reqwest::Url::parse("https://controller.invalid/").unwrap(),
        &logins,
    );
    let url = "https://claude.ai/oauth/authorize?fixture";
    let mut session = login(&fixture.account, unix_ms() + 60_000);
    session.verification_url = Some(url.into());
    logins.lock().insert("login-1".into(), session);
    let open = |request: &str| {
        let manager = Arc::clone(&fixture.manager);
        let account = fixture.account.clone();
        let request = request.to_owned();
        tokio::spawn(async move {
            manager
                .open(
                    &request,
                    account,
                    "inst-1".into(),
                    Purpose::SignIn {
                        login_request_id: "login-1".into(),
                    },
                    Viewport {
                        width: 1280,
                        height: 800,
                        scale: 1,
                    },
                )
                .await
        })
    };

    // The launcher refuses: the open reports `failed` on the running
    // browser (the Controller refuses the sign-in), which stays bound.
    let first = open("o1");
    answer_navigation(&mut fixture.launcher, url, false).await;
    let refused = first.await.unwrap();
    assert_eq!(refused.state, State::Detached);
    assert_eq!(refused.reason, Some(Reason::Failed));
    assert_eq!(fixture.instance.login_request().as_deref(), Some("login-1"));
    assert_eq!(
        fixture.instance.live.lock().failed_navigation.as_deref(),
        Some(url)
    );

    // Opening the same login again retries the navigation.
    let second = open("o2");
    answer_navigation(&mut fixture.launcher, url, true).await;
    let shown = second.await.unwrap();
    assert_eq!(shown.reason, None);
    assert!(fixture.instance.live.lock().failed_navigation.is_none());

    // Once shown, a repeated open does not navigate (and reload) again.
    let third = tokio::time::timeout(Duration::from_secs(5), open("o3"))
        .await
        .expect("a shown sign-in must not wait for the launcher")
        .unwrap();
    assert_eq!(third.reason, None);
}

#[tokio::test]
async fn a_queued_challenge_navigation_never_lands_after_a_rebinding() {
    let mut fixture = fixture().await;
    fixture
        .manager
        .trust_confinement
        .store(true, std::sync::atomic::Ordering::SeqCst);
    fixture.instance.binding.lock().purpose = Purpose::SignIn {
        login_request_id: "login-a".into(),
    };
    // Login A's challenge queues behind the navigation lock (as it would
    // behind a snapshot copy or a sign-in rebinding).
    let held = fixture.instance.navigation.lock().await;
    fixture
        .manager
        .login_challenge("login-a", "https://claude.ai/oauth/authorize?stale");
    tokio::time::sleep(Duration::from_millis(200)).await;
    // The instance is rebound to login B before the challenge proceeds.
    fixture.instance.binding.lock().purpose = Purpose::SignIn {
        login_request_id: "login-b".into(),
    };
    drop(held);
    // Login A's navigation is superseded: nothing reaches the launcher.
    let mut reader = tokio::io::BufReader::new(&mut fixture.launcher);
    let mut line = String::new();
    let sent = tokio::time::timeout(Duration::from_millis(500), reader.read_line(&mut line)).await;
    assert!(
        sent.is_err(),
        "a superseded navigation reached the launcher: {line}"
    );
    assert!(fixture.instance.live.lock().failed_navigation.is_none());
}
