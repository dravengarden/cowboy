use super::*;
use serde_json::{Value, json};

fn request() -> Value {
    json!({"service_id":format!("svc-{}", "a".repeat(32)), "machine_id":"hawk",
        "action":{"kind":"prepare","lease":{"instance":"b".repeat(32),"id":"0000000000000001"},
        "content":{"sha256":"c".repeat(64),"utf8Bytes":4},"position":{"row":0,"column":1},"query":"typeDefinition"}})
}

#[test]
fn closed_navigation_declares_no_path_runtime_deadline_or_grant() {
    let value = request();
    serde_json::from_value::<Request>(value.clone())
        .unwrap()
        .validate()
        .unwrap();
    for field in ["authorized", "deadline", "path", "runtime", "worktree"] {
        for nested in [false, true] {
            let mut invalid = value.clone();
            let target = if nested {
                &mut invalid["action"]
            } else {
                &mut invalid
            };
            target[field] = json!(true);
            assert!(serde_json::from_value::<Request>(invalid).is_err());
        }
    }
    for (field, invalid) in [
        ("utf8Bytes", json!(4 * 1024 * 1024 + 1)),
        ("sha256", json!("A".repeat(64))),
    ] {
        let mut value = request();
        value["action"]["content"][field] = invalid;
        assert!(
            serde_json::from_value::<Request>(value)
                .unwrap()
                .validate()
                .is_err()
        );
    }
    let mut invalid = request();
    invalid["action"]["position"]["row"] = json!(5);
    assert!(
        serde_json::from_value::<Request>(invalid)
            .unwrap()
            .validate()
            .is_err()
    );
}

#[test]
fn navigation_references_are_disjoint_and_commands_require_protocol_21() {
    for id in [
        "0000000000000001",
        "nav:0000000000000001",
        "navigation:0000000000000000",
        "navigation:000000000000000A",
    ] {
        assert!(
            NavigationRef {
                instance: "a".repeat(32),
                id: id.into()
            }
            .validate()
            .is_err()
        );
    }
    let id = NavigationRef {
        instance: "a".repeat(32),
        id: "navigation:0000000000000001".into(),
    };
    id.validate().unwrap();
    let value = json!({"service_id":format!("svc-{}", "a".repeat(32)), "machine_id":"hawk",
        "action":{"kind":"query","navigation":id}});
    let request: Request = serde_json::from_value(value).unwrap();
    request.validate().unwrap();
    let command = crate::machine_protocol::MachineCommand::CodeBufferNavigation {
        request_id: "navigation".into(),
        request: Box::new(request),
    };
    assert_eq!(command.minimum_protocol(), 21);
    let bytes = serde_json::to_vec(&command).unwrap();
    assert!(serde_json::from_slice::<crate::machine_protocol::MachineCommand>(&bytes).is_ok());
}

#[test]
fn locations_are_complete_bounded_and_consistent_not_lookup_paths() {
    let location: Location = serde_json::from_value(json!({"path":"target.rs",
        "content":{"sha256":"a".repeat(64),"utf8Bytes":9},
        "start":{"row":0,"column":4},"end":{"row":0,"column":6}}))
    .unwrap();
    validate_locations(&[location.clone(), location.clone()]).unwrap();
    for path in [
        "",
        "/root",
        "../target",
        "a/../b",
        "./target",
        "a//b",
        "a\nb",
        "a/",
    ] {
        let mut invalid = location.clone();
        invalid.path = path.into();
        assert!(validate_locations(&[invalid]).is_err());
    }
    let mut invalid = location.clone();
    invalid.content.sha256 = "b".repeat(64);
    assert!(validate_locations(&[location.clone(), invalid]).is_err());
    assert!(validate_locations(&vec![location.clone(); MAX_LOCATIONS + 1]).is_err());
    let targets: Vec<_> = (0..=MAX_TARGETS)
        .map(|i| Location {
            path: format!("{i}.rs"),
            ..location.clone()
        })
        .collect();
    assert!(validate_locations(&targets).is_err());
    let mut invalid = location;
    invalid.end.column = 3;
    assert!(validate_locations(&[invalid]).is_err());
}

#[test]
fn navigation_receipts_are_closed_and_destinations_are_unique_original_indices() {
    let value = json!({"api_version":1,"navigation":{"instance":"a".repeat(32),"id":"navigation:0000000000000001"},"phase":"retained",
        "locations":[{"path":"target.rs","content":{"sha256":"b".repeat(64),"utf8Bytes":9},"start":{"row":0,"column":4},"end":{"row":0,"column":6}}],
        "destinations":[{"destination":0,"lease":{"instance":"c".repeat(32),"id":"0000000000000001"}}]});
    let snapshot: Snapshot = serde_json::from_value(value.clone()).unwrap();
    snapshot.validate().unwrap();
    let mut invalid = value.clone();
    invalid["grant"] = json!(true);
    assert!(serde_json::from_value::<Snapshot>(invalid).is_err());
    for phase in [Phase::Prepared, Phase::Unknown] {
        let mut invalid = snapshot.clone();
        invalid.phase = phase;
        assert!(invalid.validate().is_err());
    }
    let mut invalid = snapshot.clone();
    invalid.destinations[0].destination = 1;
    assert!(invalid.validate().is_err());
    let mut invalid = snapshot.clone();
    invalid.locations.push(invalid.locations[0].clone());
    invalid.destinations.push(invalid.destinations[0].clone());
    assert!(invalid.validate().is_err());
    invalid.destinations[1].destination = 1;
    assert!(invalid.validate().is_err());
    invalid.destinations[1].lease.id = "0000000000000002".into();
    invalid.validate().unwrap();
    invalid.destinations.reverse();
    assert!(invalid.validate().is_err());
}
