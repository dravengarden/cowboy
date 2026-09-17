use super::super::tests::{content, fixture, navigation, observed, opened, request};
use super::*;
use crate::machine_protocol::code_buffer_navigation::{Destination as NativeDestination, Phase};
use crate::server::code_buffers::tests::native;

fn prepared(owners: &Owners, binding: &Binding, resource: &str, number: u64) -> String {
    owners
        .navigations
        .insert(
            owners.navigations.reserve().unwrap(),
            resource.into(),
            binding.clone(),
            request(),
            navigation(number),
        )
        .unwrap()
        .navigation_id
}

fn run(owners: &Owners, id: &str, action: Action) -> Box<Job> {
    let Admission::Run(job) = owners
        .navigations
        .admit(owners, "local", id, action)
        .unwrap()
    else {
        panic!("navigation job");
    };
    job.begin().unwrap();
    job
}

fn setup() -> (Arc<Owners>, Binding, String) {
    let fixture = fixture(21);
    let resource = opened(&fixture);
    let owners = fixture.context.code_buffers;
    let binding = owners
        .navigation_source("local", &resource)
        .unwrap()
        .binding
        .clone();
    (owners, binding, resource)
}

#[tokio::test]
async fn unknown_acquisition_and_release_never_expire_or_replay() {
    let (owners, binding, source) = setup();
    let id = prepared(&owners, &binding, &source, 1);
    drop(run(&owners, &id, Action::Execute));
    owners
        .navigations
        .slots
        .lock()
        .active
        .get_mut(&id)
        .unwrap()
        .until = Some(Instant::now());
    assert!(
        matches!(owners.navigations.admit(&owners, "local", &id, Action::Execute).unwrap(), Admission::Saved(snapshot, _) if snapshot.state == State::Unknown)
    );
    assert!(
        owners
            .navigations
            .admit(&owners, "local", &id, Action::Release)
            .is_err()
    );
    owners
        .navigations
        .slots
        .lock()
        .active
        .get_mut(&id)
        .unwrap()
        .until = None;
    let job = run(&owners, &id, Action::Query);
    assert!(job.finish(&owners, observed(Phase::Prepared)).is_err());
    job.finish(&owners, observed(Phase::Retained)).unwrap();
    drop(job);
    drop(run(&owners, &id, Action::Release));
    assert!(
        matches!(owners.navigations.admit(&owners, "local", &id, Action::Release).unwrap(), Admission::Saved(snapshot, _) if snapshot.state == State::ReleaseUnknown)
    );
    let job = run(&owners, &id, Action::Query);
    assert!(job.finish(&owners, observed(Phase::Retained)).is_err());
    let snapshot = job.finish(&owners, observed(Phase::Released)).unwrap();
    drop(job);
    assert_eq!(snapshot.state, State::Released);
    assert_eq!(owners.navigations.capacity.available_permits(), MAX_GROUPS);
    assert!(
        owners
            .navigations
            .insert(
                owners.navigations.reserve().unwrap(),
                source,
                binding,
                request(),
                navigation(1)
            )
            .is_err()
    );
}

#[tokio::test]
async fn capacity_includes_preparations_unknowns_and_only_inert_groups_expire() {
    let (owners, binding, source) = setup();
    let reservations: Vec<_> = (0..MAX_GROUPS)
        .map(|_| owners.navigations.reserve().unwrap())
        .collect();
    assert!(owners.navigations.reserve().is_err());
    drop(reservations);
    for number in 1..=MAX_GROUPS {
        let id = prepared(&owners, &binding, &source, number as u64);
        if number % 2 == 0 {
            drop(run(&owners, &id, Action::Execute));
        } else {
            owners
                .navigations
                .slots
                .lock()
                .active
                .get_mut(&id)
                .unwrap()
                .until = Some(Instant::now());
        }
    }
    owners.navigations.slots.lock().expire();
    assert_eq!(
        owners.navigations.capacity.available_permits(),
        MAX_GROUPS / 2
    );
    assert_eq!(owners.navigations.slots.lock().active.len(), MAX_GROUPS / 2);
}

#[tokio::test]
async fn lost_destination_receipt_is_recovered_once_by_original_query_and_original_ttl() {
    for expire in [false, true] {
        let (owners, binding, source) = setup();
        let id = prepared(&owners, &binding, &source, 1);
        run(&owners, &id, Action::Execute)
            .finish(&owners, observed(Phase::Retained))
            .unwrap();
        let action = Action::Destination {
            destination: 0,
            content: content(),
        };
        drop(run(&owners, &id, action.clone()));
        assert!(
            matches!(owners.navigations.admit(&owners, "local", &id, action.clone()).unwrap(), Admission::Saved(snapshot, _) if snapshot.destinations[0].state == DestinationState::Unknown)
        );
        if expire {
            owners
                .navigations
                .slots
                .lock()
                .active
                .get_mut(&id)
                .unwrap()
                .destinations
                .get_mut(&0)
                .unwrap()
                .reservation
                .as_mut()
                .unwrap()
                .until = Instant::now();
        }
        let mut value = observed(Phase::Retained);
        value.destinations.push(NativeDestination {
            destination: 0,
            lease: serde_json::from_value(native(2)).unwrap(),
        });
        let job = run(&owners, &id, Action::Query);
        let saved = job.finish(&owners, value.clone()).unwrap();
        drop(job);
        assert_eq!(
            saved.destinations[0].state,
            if expire {
                DestinationState::Expired
            } else {
                DestinationState::Prepared
            }
        );
        assert_eq!(saved.destinations[0].resource_id.is_some(), !expire);
        let job = run(&owners, &id, Action::Query);
        let again = job.finish(&owners, value.clone()).unwrap();
        assert_eq!(
            saved.destinations[0].resource_id,
            again.destinations[0].resource_id
        );
        value.destinations[0].lease = serde_json::from_value(native(3)).unwrap();
        assert!(job.finish(&owners, value).is_err());
    }
}

#[tokio::test]
async fn unauthorized_ids_unrequested_targets_and_changed_observations_cannot_be_adopted() {
    let (owners, binding, source) = setup();
    let id = prepared(&owners, &binding, &source, 1);
    assert!(
        owners
            .navigations
            .admit(&owners, "other", &id, Action::Execute)
            .is_err()
    );
    let job = run(&owners, &id, Action::Execute);
    let mut invalid = observed(Phase::Retained);
    invalid.destinations.push(NativeDestination {
        destination: 0,
        lease: serde_json::from_value(native(2)).unwrap(),
    });
    assert!(job.finish(&owners, invalid).is_err());
    job.finish(&owners, observed(Phase::Retained)).unwrap();
    drop(job);
    let job = run(&owners, &id, Action::Query);
    let mut invalid = observed(Phase::Retained);
    invalid.locations[0].path = "different.rs".into();
    assert!(job.finish(&owners, invalid).is_err());
}

#[tokio::test]
async fn completed_job_holds_exclusion_through_final_auth_and_queued_destination_is_reclaimable() {
    let (owners, binding, source) = setup();
    let id = prepared(&owners, &binding, &source, 1);
    let job = run(&owners, &id, Action::Execute);
    assert!(
        !job.finish(&owners, observed(Phase::Retained))
            .unwrap()
            .pending
    );
    assert!(
        matches!(owners.navigations.admit(&owners, "local", &id, Action::Query).unwrap(), Admission::Saved(snapshot, _) if snapshot.pending)
    );
    drop(job);
    let action = Action::Destination {
        destination: 0,
        content: content(),
    };
    drop(
        owners
            .navigations
            .admit(&owners, "local", &id, action.clone())
            .unwrap(),
    );
    assert!(
        owners.navigations.slots.lock().active[&id]
            .destinations
            .is_empty()
    );
    let job = run(&owners, &id, action);
    drop(job);
    assert_eq!(
        owners.navigations.slots.lock().active[&id].destinations[&0]
            .snapshot
            .state,
        DestinationState::Unknown
    );
}

#[tokio::test]
async fn ordinary_capacity_reaps_expired_navigation_reservations_without_reentry() {
    let (owners, binding, source) = setup();
    let id = prepared(&owners, &binding, &source, 1);
    run(&owners, &id, Action::Execute)
        .finish(&owners, observed(Phase::Retained))
        .unwrap();
    drop(run(
        &owners,
        &id,
        Action::Destination {
            destination: 0,
            content: content(),
        },
    ));
    let mut reservations = Vec::new();
    while let Ok(reservation) = owners.reserve() {
        reservations.push(reservation);
    }
    owners
        .navigations
        .slots
        .lock()
        .active
        .get_mut(&id)
        .unwrap()
        .destinations
        .get_mut(&0)
        .unwrap()
        .reservation
        .as_mut()
        .unwrap()
        .until = Instant::now();
    assert!(owners.reserve().is_ok());
    assert_eq!(
        owners.navigations.slots.lock().active[&id].destinations[&0]
            .snapshot
            .state,
        DestinationState::Expired
    );
}

#[tokio::test]
async fn expired_queued_execute_cannot_begin_and_shutdown_cannot_admit_new_effects() {
    let (owners, binding, source) = setup();
    let id = prepared(&owners, &binding, &source, 1);
    let Admission::Run(job) = owners
        .navigations
        .admit(&owners, "local", &id, Action::Execute)
        .unwrap()
    else {
        panic!("queued job");
    };
    owners
        .navigations
        .slots
        .lock()
        .active
        .get_mut(&id)
        .unwrap()
        .until = Some(Instant::now());
    assert_eq!(job.begin(), Err(StatusCode::CONFLICT));
    drop(job);
    assert!(
        matches!(owners.navigations.admit(&owners, "local", &id, Action::Query).unwrap(), Admission::Saved(snapshot, _) if snapshot.state == State::Expired)
    );
    let id = prepared(&owners, &binding, &source, 2);
    owners.shutdown().await;
    assert!(owners.navigations.reserve().is_err());
    assert!(
        owners
            .navigations
            .admit(&owners, "local", &id, Action::Execute)
            .is_err()
    );
}
