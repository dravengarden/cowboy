//! Native read-only enforcement for a managed child's environment.
//!
//! The keeper owns this boundary rather than the Provider: every admitted
//! process runs inside the pinned executor's own sandbox with read-only
//! filesystem access and no network, and requests that would change the
//! target are answered with an ordinary error instead of reaching it.

use std::path::Path;

use serde_json::{Value, json};

/// Methods that only observe the target. Anything else, including methods a
/// newer executor adds, is refused rather than forwarded.
const OBSERVING: &[&str] = &[
    "environment/info",
    "environment/status",
    "environmentConfig/read",
    "capabilityRoots/discoverV1",
    "fs/canonicalize",
    "fs/getMetadata",
    "fs/open",
    "fs/readBlock",
    "fs/close",
    "fs/readDirectory",
    "fs/readFile",
    "fs/walk",
    "process/read",
    "process/write",
    "process/signal",
    "process/terminate",
];

pub(super) enum Decision {
    /// Forward these native params instead of the admitted ones.
    Forward(Value),
    /// Answer without contacting the executor.
    Refuse(Value),
}

/// The executor sandbox intent of a read-only call: read the whole host, write
/// nothing (including temporary directories) and reach no network.
#[must_use]
pub(super) fn sandbox(cwd: &Path) -> Option<Value> {
    let cwd = url::Url::from_file_path(cwd).ok()?;
    Some(json!({
        "permissions": {
            "type": "managed",
            "file_system": {
                "type": "restricted",
                "entries": [{"path": {"type": "special", "value": {"kind": "root"}}, "access": "read"}],
            },
            "network": "restricted",
        },
        "cwd": cwd.as_str(),
        "windowsSandboxLevel": "disabled",
    }))
}

fn refusal(method: &str) -> Value {
    json!({"error": {"code": -32000, "message": format!(
        "This is a read-only managed call: {method} is unavailable. Inspect the snapshot and report findings instead of changing files."
    )}})
}

/// Constrain one admitted invocation of a read-only environment.
pub(super) fn decide(method: &str, params: &Value, cwd: &Path) -> Decision {
    if method == "process/start" {
        let (Some(object), Some(sandbox)) = (params.as_object(), sandbox(cwd)) else {
            return Decision::Refuse(refusal(method));
        };
        let mut object = object.clone();
        // The caller's own sandbox, network proxy or managed-network request
        // could widen access; the keeper's intent replaces all of them.
        for key in ["networkProxy", "managedNetwork"] {
            object.remove(key);
        }
        object.insert("enforceManagedNetwork".into(), Value::Bool(false));
        object.insert("sandbox".into(), sandbox);
        return Decision::Forward(Value::Object(object));
    }
    if OBSERVING.contains(&method) {
        Decision::Forward(params.clone())
    } else {
        Decision::Refuse(refusal(method))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn processes_always_run_in_the_keeper_sandbox() {
        let cwd = Path::new("/state/managed/child/workspace");
        let params = json!({
            "processId": "p1", "argv": ["sh", "-c", "touch x"], "cwd": "file:///tmp",
            "env": {}, "tty": false, "arg0": null,
            "sandbox": {"permissions": {"type": "disabled"}, "cwd": "file:///", "windowsSandboxLevel": "disabled"},
            "enforceManagedNetwork": true,
            "networkProxy": {"anything": true},
            "managedNetwork": {"anything": true},
        });
        let Decision::Forward(forwarded) = decide("process/start", &params, cwd) else {
            panic!("process start must be forwarded sandboxed");
        };
        assert_eq!(forwarded["sandbox"], sandbox(cwd).unwrap());
        assert_eq!(
            forwarded["sandbox"]["permissions"]["file_system"]["entries"][0]["access"],
            "read"
        );
        assert_eq!(forwarded["sandbox"]["permissions"]["network"], "restricted");
        assert_eq!(forwarded["enforceManagedNetwork"], false);
        assert!(
            forwarded.get("networkProxy").is_none() && forwarded.get("managedNetwork").is_none()
        );
        assert_eq!(forwarded["argv"], params["argv"]);
    }

    #[test]
    fn mutations_network_and_unknown_methods_never_reach_the_executor() {
        let cwd = Path::new("/w");
        for method in [
            "fs/writeFile",
            "fs/createDirectory",
            "fs/remove",
            "fs/copy",
            "http/request",
            "network/policyDecision",
            "fs/futureWrite",
        ] {
            let Decision::Refuse(reply) = decide(method, &json!({}), cwd) else {
                panic!("{method} must be refused");
            };
            assert!(
                reply["error"]["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("read-only"))
            );
        }
        let read = json!({"path": "file:///w/a"});
        assert!(matches!(
            decide("fs/readFile", &read, cwd),
            Decision::Forward(params) if params == read
        ));
    }
}
