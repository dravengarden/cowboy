use serde_json::{Value, json};
use std::{collections::HashSet, fs, io::Write as _, path::Path};

#[derive(Default)]
pub(crate) struct Fixture {
    state: Option<&'static str>,
    dropped: HashSet<String>,
}

impl Fixture {
    pub(crate) fn reply(
        &mut self,
        request: &Value,
        home: &Path,
        generation: &str,
    ) -> Option<Option<Value>> {
        if !generation.starts_with("generation-acquisition-") {
            return None;
        }
        let kind = request["type"].as_str()?;
        if !matches!(
            kind,
            "agentAcquisitionSupport" | "prepareAgentAcquisition" | "agentAcquisition"
        ) {
            return None;
        }
        let mut log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(home.join("acquisition-log"))
            .unwrap();
        writeln!(log, "{request}").unwrap();
        if kind == "agentAcquisitionSupport" {
            return Some(if generation.ends_with("unsupported") {
                Some(json!({"type":"error","message":"native journal unavailable"}))
            } else if generation.ends_with("malformed") {
                Some(json!({"type":kind,"api_version":1,"protocol":2}))
            } else {
                Some(json!({"type":kind,"api_version":1,"protocol":1}))
            });
        }
        let action = if kind == "prepareAgentAcquisition" {
            "prepare"
        } else {
            request["action"].as_str().unwrap()
        };
        match action {
            "prepare" => {
                self.state.get_or_insert("prepared");
            }
            "execute" => {
                assert_eq!(self.state, Some("prepared"));
                self.state = Some(if generation.ends_with("pending") {
                    "acquiring"
                } else {
                    "ready"
                });
            }
            "release" => {
                assert_ne!(self.state, Some("released"));
                self.state = Some("released");
            }
            "retire" => self.state = Some("retired"),
            "query" => {}
            _ => panic!("unexpected action"),
        }
        if generation.ends_with("lost") && action != "query" && self.dropped.insert(action.into()) {
            return Some(None);
        }
        let mut reference = json!({"instance":"a".repeat(32),"id":"0000000000000001"});
        if action == "query" && home.join("foreign-query").exists() {
            reference["instance"] = json!("b".repeat(32));
        }
        let buffer = if self.state == Some("ready") {
            Some(json!({"type":"buffer","api_version":1,"buffer_id":77,"leases":1,"version":[]}))
        } else {
            None
        };
        Some(Some(
            json!({"type":"agentAcquisition","api_version":1,"acquisition":reference,"state":self.state,"buffer":buffer}),
        ))
    }
}
