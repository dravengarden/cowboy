use super::*;
use crate::server::code_buffers::tests::{Fixture, native};

fn binding(fixture: &Fixture, id: u64) -> Binding {
    Binding {
        user: "local".into(),
        scope: fixture.context.hub.session_code_scope("session").unwrap(),
        connection: fixture.connection.clone(),
        native: serde_json::from_value(native(id)).unwrap(),
    }
}

fn insert(fixture: &Fixture, id: u64) -> Snapshot {
    fixture
        .context
        .code_buffers
        .insert(
            fixture.context.code_buffers.reserve().unwrap(),
            binding(fixture, id),
        )
        .unwrap()
}

fn job(owners: &Arc<Owners>, id: &str, action: Action) -> Job {
    match owners.admit("local", id, action).unwrap() {
        Admission::Run(job) => *job,
        Admission::Saved(_) => panic!("expected admitted job"),
    }
}

fn saved(owners: &Arc<Owners>, id: &str, action: Action) -> Snapshot {
    match owners.admit("local", id, action).unwrap() {
        Admission::Saved(snapshot) => snapshot,
        Admission::Run(_) => panic!("expected saved observation"),
    }
}

#[test]
fn original_user_and_process_are_required_even_for_terminal_evidence() {
    let fixture = Fixture::new();
    let owners = &fixture.context.code_buffers;
    let prepared = insert(&fixture, 1);
    for action in [Action::Open, Action::Query, Action::Release] {
        assert!(matches!(
            owners.admit("foreign", &prepared.resource_id, action),
            Err(StatusCode::NOT_FOUND)
        ));
        assert!(matches!(
            Arc::new(Owners::default()).admit("local", &prepared.resource_id, action),
            Err(StatusCode::NOT_FOUND)
        ));
    }
    let release = job(owners, &prepared.resource_id, Action::Release);
    release.begin().unwrap();
    release.finish(LeaseState::Released).unwrap();
    assert!(matches!(
        owners.admit("foreign", &prepared.resource_id, Action::Query),
        Err(StatusCode::NOT_FOUND)
    ));
    assert_eq!(owners.capacity.available_permits(), MAX_LEASES);
    let another = insert(&fixture, 2);
    assert_ne!(prepared.resource_id, another.resource_id);
}

#[test]
fn only_inert_preparations_expire_including_cancelled_admissions() {
    let fixture = Fixture::new();
    let owners = &fixture.context.code_buffers;
    let inert = insert(&fixture, 1);
    let active = insert(&fixture, 2);
    let unknown = insert(&fixture, 3);
    let before_effect = job(owners, &inert.resource_id, Action::Open);
    drop(before_effect);
    let opened = job(owners, &active.resource_id, Action::Open);
    opened.begin().unwrap();
    opened.finish(LeaseState::Open).unwrap();
    let ambiguous = job(owners, &unknown.resource_id, Action::Open);
    ambiguous.begin().unwrap();
    drop(ambiguous);
    owners
        .slots
        .lock()
        .active
        .get_mut(&inert.resource_id)
        .unwrap()
        .until = Some(Instant::now());
    assert_eq!(
        saved(owners, &inert.resource_id, Action::Query).state,
        LeaseState::Released
    );
    assert_eq!(
        saved(owners, &active.resource_id, Action::Open).state,
        LeaseState::Open
    );
    assert_eq!(
        saved(owners, &unknown.resource_id, Action::Open).state,
        LeaseState::Unknown
    );
    assert_eq!(owners.slots.lock().active.len(), 2);
    assert_eq!(owners.capacity.available_permits(), MAX_LEASES - 2);
}

#[test]
fn capacity_includes_pending_preparations_and_does_not_evict_unknown_owners() {
    let fixture = Fixture::new();
    let owners = &fixture.context.code_buffers;
    let unknown = insert(&fixture, 1);
    let attempt = job(owners, &unknown.resource_id, Action::Open);
    attempt.begin().unwrap();
    drop(attempt);
    let pending: Vec<_> = (1..MAX_LEASES).map(|_| owners.reserve().unwrap()).collect();
    assert!(matches!(
        owners.reserve(),
        Err(StatusCode::TOO_MANY_REQUESTS)
    ));
    assert_eq!(
        saved(owners, &unknown.resource_id, Action::Open).state,
        LeaseState::Unknown
    );
    drop(pending);
    assert_eq!(owners.capacity.available_permits(), MAX_LEASES - 1);
    let mut expired = owners.reserve().unwrap();
    expired.until = Instant::now();
    assert_eq!(
        owners.insert(expired, binding(&fixture, 2)).unwrap_err(),
        StatusCode::CONFLICT
    );
    assert_eq!(owners.capacity.available_permits(), MAX_LEASES - 1);
}

#[test]
fn concurrent_operations_coalesce_observation_and_count_against_a_separate_job_limit() {
    let fixture = Fixture::new();
    let owners = &fixture.context.code_buffers;
    let mut jobs = Vec::new();
    for id in 1..=MAX_JOBS as u64 {
        let prepared = insert(&fixture, id);
        jobs.push(job(owners, &prepared.resource_id, Action::Open));
        for action in [Action::Open, Action::Query, Action::Release] {
            assert!(saved(owners, &prepared.resource_id, action).pending);
        }
    }
    let extra = insert(&fixture, MAX_JOBS as u64 + 1);
    assert!(matches!(
        owners.admit("local", &extra.resource_id, Action::Open),
        Err(StatusCode::TOO_MANY_REQUESTS)
    ));
    drop(jobs);
    drop(job(owners, &extra.resource_id, Action::Open));
    assert_eq!(owners.job_capacity.available_permits(), MAX_JOBS);
}

#[test]
fn queries_cannot_rearm_an_attempt_or_regress_an_observed_open() {
    let fixture = Fixture::new();
    let owners = &fixture.context.code_buffers;
    let prepared = insert(&fixture, 1);
    let opening = job(owners, &prepared.resource_id, Action::Open);
    opening.begin().unwrap();
    drop(opening);
    let query = job(owners, &prepared.resource_id, Action::Query);
    query.begin().unwrap();
    query.finish(LeaseState::Prepared).unwrap();
    assert_eq!(
        saved(owners, &prepared.resource_id, Action::Open).state,
        LeaseState::Prepared
    );
    let query = job(owners, &prepared.resource_id, Action::Query);
    query.begin().unwrap();
    query.finish(LeaseState::Open).unwrap();
    let query = job(owners, &prepared.resource_id, Action::Query);
    query.begin().unwrap();
    assert_eq!(
        query.finish(LeaseState::Prepared).unwrap_err(),
        StatusCode::BAD_GATEWAY
    );
    assert_eq!(
        saved(owners, &prepared.resource_id, Action::Open).state,
        LeaseState::Open
    );
}

#[test]
fn duplicate_native_owners_are_rejected_and_terminal_cache_is_bounded() {
    let fixture = Fixture::new();
    let owners = &fixture.context.code_buffers;
    let first = insert(&fixture, 1);
    assert_eq!(
        owners
            .insert(owners.reserve().unwrap(), binding(&fixture, 1))
            .unwrap_err(),
        StatusCode::BAD_GATEWAY
    );
    let original_scope = owners.slots.lock().active[&first.resource_id]
        .binding
        .scope
        .clone();
    for id in 1..=MAX_LEASES as u64 + 1 {
        let prepared = if id == 1 {
            first.clone()
        } else {
            insert(&fixture, id)
        };
        let release = job(owners, &prepared.resource_id, Action::Release);
        release.begin().unwrap();
        release.finish(LeaseState::Released).unwrap();
    }
    assert!(owners.slots.lock().active.is_empty());
    assert_eq!(owners.slots.lock().released.len(), MAX_LEASES);
    assert!(matches!(
        owners.admit("local", &first.resource_id, Action::Open),
        Err(StatusCode::NOT_FOUND)
    ));
    assert!(fixture.context.hub.code_scope_is_current(&original_scope));
}

#[tokio::test]
async fn shutdown_cancels_owned_jobs_without_erasing_their_possible_effect() {
    let fixture = Fixture::new();
    let owners = &fixture.context.code_buffers;
    let prepared = insert(&fixture, 1);
    let attempt = job(owners, &prepared.resource_id, Action::Open);
    let (sent, started) = oneshot::channel();
    let observer = owners
        .spawn(async move {
            attempt.begin()?;
            sent.send(()).unwrap();
            std::future::pending::<()>().await;
            attempt.finish(LeaseState::Open)
        })
        .unwrap();
    started.await.unwrap();
    owners.shutdown().await;
    assert!(observer.await.is_err());
    assert_eq!(
        owners.slots.lock().active[&prepared.resource_id].state,
        LeaseState::Unknown
    );
    assert!(!owners.slots.lock().active[&prepared.resource_id].busy);
    assert_eq!(owners.job_capacity.available_permits(), MAX_JOBS);
    assert!(matches!(
        owners.reserve(),
        Err(StatusCode::SERVICE_UNAVAILABLE)
    ));
    assert!(matches!(
        owners.spawn::<Snapshot>(async { unreachable!() }),
        Err(StatusCode::SERVICE_UNAVAILABLE)
    ));
}

#[test]
fn reads_borrow_only_confirmed_original_owners_without_rearming_effects() {
    let fixture = Fixture::new();
    let owners = &fixture.context.code_buffers;
    let prepared = insert(&fixture, 1);
    let id = &prepared.resource_id;
    assert!(matches!(
        owners.admit_read("local", id),
        Err(StatusCode::CONFLICT)
    ));
    let open = job(owners, id, Action::Open);
    open.begin().unwrap();
    drop(open);
    assert!(matches!(
        owners.admit_read("local", id),
        Err(StatusCode::CONFLICT)
    ));
    job(owners, id, Action::Query)
        .finish(LeaseState::Open)
        .unwrap();
    assert!(matches!(
        owners.admit_read("foreign", id),
        Err(StatusCode::NOT_FOUND)
    ));
    let read = owners.admit_read("local", id).unwrap();
    assert!(matches!(
        owners.admit_read("local", id),
        Err(StatusCode::CONFLICT)
    ));
    assert!(saved(owners, id, Action::Release).pending);
    assert_eq!(owners.job_capacity.available_permits(), MAX_JOBS - 1);
    drop(read);
    assert_eq!(owners.job_capacity.available_permits(), MAX_JOBS);
    assert_eq!(saved(owners, id, Action::Open).state, LeaseState::Open);
    let release = job(owners, id, Action::Release);
    release.begin().unwrap();
    drop(release);
    job(owners, id, Action::Query)
        .finish(LeaseState::Open)
        .unwrap();
    assert!(matches!(
        owners.admit_read("local", id),
        Err(StatusCode::CONFLICT)
    ));
    job(owners, id, Action::Query)
        .finish(LeaseState::Released)
        .unwrap();
    assert!(matches!(
        owners.admit_read("local", id),
        Err(StatusCode::NOT_FOUND)
    ));
}

#[tokio::test]
async fn observer_loss_cannot_cancel_a_still_owned_continuation() {
    let fixture = Fixture::new();
    let owners = &fixture.context.code_buffers;
    let prepared = insert(&fixture, 1);
    let attempt = job(owners, &prepared.resource_id, Action::Open);
    let (release, waiting) = oneshot::channel();
    let (done, finished) = oneshot::channel();
    let observer = owners
        .spawn(async move {
            attempt.begin()?;
            waiting.await.unwrap();
            let result = attempt.finish(LeaseState::Open);
            done.send(()).unwrap();
            result
        })
        .unwrap();
    drop(observer);
    release.send(()).unwrap();
    finished.await.unwrap();
    assert_eq!(
        saved(owners, &prepared.resource_id, Action::Open).state,
        LeaseState::Open
    );
    owners.shutdown().await;
}
