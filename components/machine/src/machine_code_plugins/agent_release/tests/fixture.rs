use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs;
use std::io::Write as _;
use std::path::Path;
use std::time::Duration;

#[derive(Default)]
pub(crate) struct Fixture {
    next: u64,
    entries: HashMap<String, Value>,
}

impl Fixture {
    pub(crate) fn reply(
        &mut self,
        request: &Value,
        home: &Path,
        generation: &str,
    ) -> Option<Option<Value>> {
        let kind = request["type"].as_str()?;
        if !generation.starts_with("generation-recover-agent-")
            || !matches!(
                kind,
                "legacyReleaseSupport" | "prepareLegacyRelease" | "legacyRelease"
            )
        {
            return None;
        }
        let mut log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(home.join("release-log"))
            .unwrap();
        writeln!(log, "{request}").unwrap();
        if kind == "legacyReleaseSupport" {
            return Some(Some(json!({"type":kind,"api_version":1,"protocol":1})));
        }
        let mut reference = if kind == "prepareLegacyRelease" {
            self.next += 1;
            let id = format!("{:016x}", self.next);
            self.entries.insert(id.clone(), json!({"type":"buffer","api_version":1,"worktree":request["worktree"],"path":request["path"],"leases":0,"buffer_id":self.next,"version":[]}));
            json!({"instance":format!("{:032x}",std::process::id()),"id":id})
        } else {
            request["release"].clone()
        };
        let id = reference["id"].as_str().unwrap();
        if request["action"] == "execute" {
            return Some(None);
        }
        let state = if kind == "prepareLegacyRelease" {
            "prepared"
        } else if request["action"] == "retire" {
            if generation.ends_with("pause-retire") {
                fs::write(home.join("retire-started"), "yes").unwrap();
                for _ in 0..500 {
                    if home.join("resume-retire").exists() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                assert!(home.join("resume-retire").exists());
            }
            self.entries.remove(id);
            "retired"
        } else if generation.ends_with("pending") {
            "unknown"
        } else {
            "released"
        };
        let buffer = if state == "released" {
            self.entries.get(id).cloned()
        } else {
            None
        };
        if request["action"] == "query" && home.join("foreign-query").exists() {
            reference["instance"] = json!("f".repeat(32));
        }
        Some(Some(
            json!({"type":"legacyRelease","api_version":1,"release":reference,"state":state,"buffer":buffer}),
        ))
    }
}
