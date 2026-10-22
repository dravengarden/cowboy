use super::*;
use crate::server::code_buffers::synchronization::tests::{applied, content, fixture, opened};
use crate::server::code_buffers::tests::{Fixture, native};

fn insert(fixture: &Fixture, number: u64) -> Snapshot {
    let resource = opened(fixture, number);
    let owners = &fixture.context.code_buffers;
    let (binding, fence) = owners.synchronization_owner("local", &resource).unwrap();
    owners
        .synchronizations
        .insert(
            owners.synchronizations.reserve().unwrap(),
            Prepared {
                resource,
                binding,
                fence,
                content: content(),
                operation: serde_json::from_value(native(number)).unwrap(),
            },
        )
        .unwrap()
}

fn job(operations: &Arc<Operations>, id: &str, action: Action) -> Box<Job> {
    match operations.admit("local", id, action).unwrap() {
        Admission::Run(job) => job,
        Admission::Saved(_) => panic!("expected job"),
    }
}

fn saved(operations: &Arc<Operations>, id: &str, action: Action) -> Snapshot {
    match operations.admit("local", id, action).unwrap() {
        Admission::Run(_) => panic!("expected observation"),
        Admission::Saved(snapshot) => snapshot,
    }
}

#[test]
fn inert_expiry_unfences_but_unknown_and_terminal_entries_never_evict() {
    let fixture = fixture(20);
    let owners = &fixture.context.code_buffers;
    let operations = &owners.synchronizations;
    let inert = insert(&fixture, 1);
    let unknown = insert(&fixture, 2);
    let terminal = insert(&fixture, 3);
    let attempt = job(operations, &unknown.operation_id, Action::Apply);
    attempt.begin().unwrap();
    drop(attempt);
    let attempt = job(operations, &terminal.operation_id, Action::Apply);
    attempt.begin().unwrap();
    attempt.finish(applied()).unwrap();
    drop(attempt);
    operations
        .slots
        .lock()
        .active
        .get_mut(&inert.operation_id)
        .unwrap()
        .until = Some(Instant::now());
    // Normal buffer admission triggers inert synchronization expiry too.
    drop(owners.admit_read("local", &inert.resource_id).unwrap());
    assert!(matches!(
        saved(operations, &inert.operation_id, Action::Query).state,
        State::Expired {}
    ));
    assert!(matches!(
        operations.admit("local", &inert.operation_id, Action::Apply),
        Err(StatusCode::CONFLICT)
    ));
    let pending: Vec<_> = (2..MAX_OPERATIONS)
        .map(|_| operations.reserve().unwrap())
        .collect();
    assert!(matches!(
        operations.reserve(),
        Err(StatusCode::TOO_MANY_REQUESTS)
    ));
    assert!(matches!(
        saved(operations, &unknown.operation_id, Action::Apply).state,
        State::Unknown {}
    ));
    assert_eq!(operations.slots.lock().active.len(), 2);
    drop(pending);
    assert_eq!(operations.capacity.available_permits(), MAX_OPERATIONS - 2);
}

#[test]
fn authority_loss_before_dispatch_preserves_preparation_but_not_its_deadline() {
    let fixture = fixture(20);
    let prepared = insert(&fixture, 1);
    let operations = &fixture.context.code_buffers.synchronizations;
    drop(job(operations, &prepared.operation_id, Action::Apply));
    let apply = job(operations, &prepared.operation_id, Action::Apply);
    operations
        .slots
        .lock()
        .active
        .get_mut(&prepared.operation_id)
        .unwrap()
        .until = Some(Instant::now());
    assert_eq!(apply.begin().unwrap_err(), StatusCode::CONFLICT);
    drop(apply);
    assert!(matches!(
        saved(operations, &prepared.operation_id, Action::Query).state,
        State::Expired {}
    ));
}

#[test]
fn completed_observation_keeps_job_exclusion_until_its_authority_check_drops() {
    let fixture = fixture(20);
    let prepared = insert(&fixture, 1);
    let operations = &fixture.context.code_buffers.synchronizations;
    let id = &prepared.operation_id;
    let query = job(operations, id, Action::Query);
    query.begin().unwrap();
    assert!(!query.finish(NativeState::Prepared {}).unwrap().pending);
    // The HTTP response still owns the query while rechecking permission.
    // No successor can enter whose exclusion an older Job's Drop could erase.
    for action in [Action::Apply, Action::Query, Action::Retire] {
        assert!(saved(operations, id, action).pending);
    }
    drop(query);
    let apply = job(operations, id, Action::Apply);
    apply.begin().unwrap();
    assert!(!apply.finish(applied()).unwrap().pending);
    for action in [Action::Apply, Action::Query, Action::Retire] {
        assert!(saved(operations, id, action).pending);
    }
    drop(apply);
    assert!(!saved(operations, id, Action::Query).pending);
    let retire = job(operations, id, Action::Retire);
    retire.begin().unwrap();
    retire.finish(NativeState::Retired {}).unwrap();
}

#[test]
fn unknown_queries_never_rearm_or_dispose_of_an_attempt_and_retirement_does_not_replay() {
    let fixture = fixture(20);
    let prepared = insert(&fixture, 1);
    let operations = &fixture.context.code_buffers.synchronizations;
    let id = &prepared.operation_id;
    let apply = job(operations, id, Action::Apply);
    apply.begin().unwrap();
    drop(apply);
    for state in [NativeState::Prepared {}, NativeState::Retired {}] {
        let query = job(operations, id, Action::Query);
        query.begin().unwrap();
        assert_eq!(query.finish(state).unwrap_err(), StatusCode::BAD_GATEWAY);
    }
    assert!(matches!(
        operations.admit("local", id, Action::Retire),
        Err(StatusCode::CONFLICT)
    ));
    let query = job(operations, id, Action::Query);
    query.begin().unwrap();
    query.finish(applied()).unwrap();
    drop(query);
    let retire = job(operations, id, Action::Retire);
    retire.begin().unwrap();
    drop(retire); // lost receipt
    assert!(matches!(
        saved(operations, id, Action::Retire).state,
        State::Applied { .. }
    ));
    let query = job(operations, id, Action::Query);
    query.begin().unwrap();
    assert_eq!(
        query.finish(NativeState::Pending {}).unwrap_err(),
        StatusCode::BAD_GATEWAY
    );
    query.finish(NativeState::Retired {}).unwrap();
    assert_eq!(operations.capacity.available_permits(), MAX_OPERATIONS);
}

#[test]
fn operation_and_job_limits_include_pending_work_and_never_evict_a_fence() {
    let fixture = fixture(20);
    let operations = &fixture.context.code_buffers.synchronizations;
    let mut jobs = Vec::new();
    for number in 1..=MAX_JOBS as u64 {
        let prepared = insert(&fixture, number);
        jobs.push(job(operations, &prepared.operation_id, Action::Apply));
        assert!(saved(operations, &prepared.operation_id, Action::Query).pending);
    }
    let extra = insert(&fixture, MAX_JOBS as u64 + 1);
    assert!(matches!(
        operations.admit("local", &extra.operation_id, Action::Apply),
        Err(StatusCode::TOO_MANY_REQUESTS)
    ));
    drop(jobs);
    drop(job(operations, &extra.operation_id, Action::Apply));
    assert_eq!(operations.jobs.available_permits(), MAX_JOBS);
}

#[test]
fn users_processes_and_nonrecycled_ids_remain_separate_through_retirement() {
    let fixture = fixture(20);
    let operations = &fixture.context.code_buffers.synchronizations;
    let prepared = insert(&fixture, 1);
    for action in [Action::Apply, Action::Query, Action::Retire] {
        assert!(matches!(
            operations.admit("foreign", &prepared.operation_id, action),
            Err(StatusCode::NOT_FOUND)
        ));
        assert!(matches!(
            Arc::new(Operations::default()).admit("local", &prepared.operation_id, action),
            Err(StatusCode::NOT_FOUND)
        ));
    }
    for number in 1..=MAX_OPERATIONS as u64 + 1 {
        let current = if number == 1 {
            prepared.clone()
        } else {
            insert(&fixture, number)
        };
        let retire = job(operations, &current.operation_id, Action::Retire);
        retire.begin().unwrap();
        retire.finish(NativeState::Retired {}).unwrap();
    }
    assert_eq!(operations.slots.lock().retired.len(), MAX_OPERATIONS);
    assert!(operations.slots.lock().active.is_empty());
    assert_eq!(operations.capacity.available_permits(), MAX_OPERATIONS);
    assert!(matches!(
        operations.admit("local", &prepared.operation_id, Action::Query),
        Err(StatusCode::NOT_FOUND)
    ));
}
