use super::{CheckError, check, decode, wire};
use serde_json::{Value, json};

const FIXTURE: &str = include_str!("../../tests/fixtures/composition-v1.json");

fn fixture() -> Value {
    serde_json::from_str(FIXTURE).unwrap()
}
fn base() -> Value {
    fixture()["base"].clone()
}
fn run(value: &Value) -> Result<super::CheckedStructure, CheckError> {
    check(&serde_json::to_vec(value).unwrap())
}

#[test]
fn shared_codec_and_semantic_vectors() {
    let fixture = fixture();
    for vector in fixture["vectors"].as_array().unwrap() {
        let name = vector["name"].as_str().unwrap();
        let bytes = if let Some(raw) = vector["raw"].as_str() {
            raw.as_bytes().to_vec()
        } else {
            let mut proposal = fixture["base"].clone();
            for edit in vector["edits"].as_array().unwrap() {
                let (parent, key) = edit["path"].as_str().unwrap().rsplit_once('/').unwrap();
                let parent = proposal.pointer_mut(parent).unwrap();
                if let Some(array) = parent.as_array_mut() {
                    array[key.parse::<usize>().unwrap()] = edit["value"].clone();
                } else if edit["remove"] == true {
                    parent.as_object_mut().unwrap().remove(key);
                } else {
                    parent[key] = edit["value"].clone();
                }
            }
            serde_json::to_vec(&proposal).unwrap()
        };
        assert_eq!(
            decode(&bytes).is_ok(),
            vector["decode"].as_bool().unwrap(),
            "codec: {name}"
        );
        let result = check(&bytes);
        if let Some(error) = vector["error"].as_str() {
            assert_eq!(result.unwrap_err().to_string(), error, "semantic: {name}");
        } else {
            assert!(
                !result
                    .unwrap_or_else(|error| panic!("{name}: {error}"))
                    .authorized
            );
        }
    }
}

#[test]
fn deterministic_identity_and_local_projections() {
    let mut proposal = base();
    let report = run(&proposal).unwrap();
    assert_eq!(
        report
            .dependency_order
            .iter()
            .map(|id| id.0.as_str())
            .collect::<Vec<_>>(),
        ["victoria", "client"]
    );
    assert_eq!(
        report
            .reverse_dependency_order
            .iter()
            .map(|id| id.0.as_str())
            .collect::<Vec<_>>(),
        ["client", "victoria"]
    );
    assert_eq!(report.sites.len(), 2);
    assert_eq!(report.remote_bindings.len(), 1);
    proposal["nodes"].as_array_mut().unwrap().reverse();
    proposal["scopes"].as_array_mut().unwrap().reverse();
    assert_eq!(
        report.proposal_digest,
        run(&proposal).unwrap().proposal_digest
    );
    proposal["bindings"][0]["revision"] = json!("2");
    assert_ne!(
        report.proposal_digest,
        run(&proposal).unwrap().proposal_digest
    );
    let expected = wire::CONTRACT_FINGERPRINT;
    assert_eq!(report.contract_fingerprint, expected);
}

#[test]
fn generation_maps_to_exact_release_but_old_generation_may_coexist() {
    let mut proposal = base();
    let mut second = proposal["nodes"][1].clone();
    second["id"] = json!("victoria-new");
    second["identity"]["release"]["version"] = json!("2.0.0");
    proposal["nodes"].as_array_mut().unwrap().push(second);
    assert_eq!(run(&proposal).unwrap_err(), CheckError::GenerationConflict);
    proposal["nodes"][2]["identity"]["generation"] = json!("2");
    assert!(run(&proposal).is_ok());
    // The same release version with a different digest is still a conflict.
    proposal["nodes"][2]["identity"]["generation"] = json!("1");
    proposal["nodes"][2]["identity"]["release"]["version"] = json!("1.0.0");
    proposal["nodes"][2]["identity"]["release"]["digest"] =
        json!(format!("sha256:{}", "f".repeat(64)));
    assert_eq!(run(&proposal).unwrap_err(), CheckError::GenerationConflict);
}

#[test]
fn one_optional_many_have_explicit_selection_not_last_registration_wins() {
    let mut proposal = base();
    let mut second = proposal["nodes"][1].clone();
    second["id"] = json!("victoria-other");
    proposal["nodes"].as_array_mut().unwrap().push(second);
    let mut binding = proposal["bindings"][0].clone();
    binding["provider"]["node"] = json!("victoria-other");
    proposal["bindings"].as_array_mut().unwrap().push(binding);
    assert_eq!(run(&proposal).unwrap_err(), CheckError::CardinalityMismatch);
    proposal["nodes"][0]["requires"][0]["cardinality"] = json!("optional");
    assert_eq!(run(&proposal).unwrap_err(), CheckError::CardinalityMismatch);
    proposal["nodes"][0]["requires"][0]["cardinality"] = json!("many");
    let report = run(&proposal).unwrap();
    proposal["bindings"].as_array_mut().unwrap().reverse();
    assert_eq!(
        report.proposal_digest,
        run(&proposal).unwrap().proposal_digest
    );
}

#[test]
fn dependency_cycle_cannot_hide_behind_remote_ports() {
    let mut proposal = base();
    proposal["nodes"][0]["provides"] = proposal["nodes"][1]["provides"].clone();
    proposal["nodes"][1]["requires"] = proposal["nodes"][0]["requires"].clone();
    proposal["bindings"].as_array_mut().unwrap().push(json!({
        "consumer": { "node": "victoria", "port": "export" },
        "provider": { "node": "client", "port": "export" }, "revision": "1"
    }));
    assert_eq!(run(&proposal).unwrap_err(), CheckError::DependencyCycle);
}

#[test]
fn budgets_are_enforced_without_echoing_untrusted_input() {
    let mut proposal = base();
    let node = proposal["nodes"][1].clone();
    proposal["nodes"] = json!(vec![node; 257]);
    assert_eq!(run(&proposal).unwrap_err(), CheckError::InvalidContract);
    let mut bytes = serde_json::to_vec(&base()).unwrap();
    bytes.resize(wire::MAX_BYTES + 1, b' ');
    assert_eq!(check(&bytes).unwrap_err(), CheckError::InvalidJson);
    assert_eq!(
        check(b"{\"secret\":\"synthetic-private-token\"}")
            .unwrap_err()
            .to_string(),
        "invalid_contract"
    );
    assert_eq!(check(b"\xff").unwrap_err(), CheckError::InvalidJson);
}

#[test]
fn total_port_budget_is_not_multiplied_by_components() {
    let mut proposal = base();
    let mut nodes = Vec::new();
    for index in 0..65 {
        let mut node = proposal["nodes"][1].clone();
        node["id"] = json!(format!("node-{index}"));
        let mut ports = Vec::new();
        for index in 0..32 {
            let mut port = node["provides"][0].clone();
            port["id"] = json!(format!("port-{index}"));
            ports.push(port);
        }
        node["provides"] = json!(ports);
        nodes.push(node);
    }
    proposal["nodes"] = json!(nodes);
    proposal["bindings"] = json!([]);
    assert_eq!(run(&proposal).unwrap_err(), CheckError::PortBudget);
}
