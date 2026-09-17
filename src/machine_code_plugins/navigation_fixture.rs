//! Test-process-only saved navigation responses, never a product LSP.
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Default)]
pub(super) struct Fixture {
    last: u64,
    slots: BTreeMap<String, Value>,
    executes: u64,
    releases: u64,
    queries: u64,
    destinations: u64,
}

fn pause(home: &Path, name: &str) {
    for _ in 0..500 {
        if home.join(name).exists() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("fixture navigation never resumed");
}

impl Fixture {
    pub(super) fn reply(
        &mut self,
        request: &Value,
        home: &Path,
        generation: &str,
        owned: &mut BTreeMap<String, &'static str>,
        next_lease: &mut u64,
    ) -> Option<Option<Value>> {
        let kind = request["type"].as_str()?;
        if kind == "bufferNavigationSupport" {
            return Some(Some(if generation == "generation-navigation-old" {
                json!({"type":"bufferSyncOwnerSupport","api_version":1,"protocol":1})
            } else {
                json!({"type":"bufferNavigationSupport","api_version":1,"protocol":1})
            }));
        }
        if kind == "prepareNavigationBuffer" {
            self.destinations += 1;
            std::fs::write(home.join("nav-destinations"), self.destinations.to_string()).unwrap();
            *next_lease += 1;
            let id = format!("{next_lease:016x}");
            owned.insert(id.clone(), "prepared");
            if generation == "generation-navigation-pause-destination" {
                pause(home, "resume-destination");
            }
            return Some(Some(json!({"type":"bufferLease","api_version":1,
                "lease":{"instance":format!("{:032x}",std::process::id()),"id":id},"state":"prepared"})));
        }
        if !matches!(kind, "prepareBufferNavigation" | "bufferNavigation") {
            return None;
        }
        let navigation = if kind == "prepareBufferNavigation" {
            self.last += 1;
            std::fs::write(home.join("nav-prepares"), self.last.to_string()).unwrap();
            let id = format!("nav:{:016x}", self.last);
            self.slots.insert(id.clone(), json!({"kind":"prepared"}));
            json!({"instance":format!("{:032x}",std::process::id()),"id":id})
        } else {
            request["navigation"].clone()
        };
        let state = self
            .slots
            .get_mut(navigation["id"].as_str().unwrap())
            .unwrap();
        let action = request["action"].as_str().unwrap_or("");
        if action == "execute" {
            self.executes += 1;
            std::fs::write(home.join("nav-executes"), self.executes.to_string()).unwrap();
            *state = if generation == "generation-navigation-unknown" {
                json!({"kind":"unknown"})
            } else {
                json!({"kind":"retained","locations":[
                    {"path":"a.rs","content":{"sha256":"c".repeat(64),"utf8Bytes":4},
                        "start":{"row":0,"column":0},"end":{"row":0,"column":1}},
                    {"path":"b.rs","content":{"sha256":"c".repeat(64),"utf8Bytes":4},
                        "start":{"row":0,"column":0},"end":{"row":0,"column":1}}
                ]})
            };
            if generation == "generation-navigation-pause" {
                pause(home, "resume-navigation");
            }
            if generation == "generation-navigation-lost" {
                return Some(None);
            }
        }
        if action == "release" {
            self.releases += 1;
            std::fs::write(home.join("nav-releases"), self.releases.to_string()).unwrap();
            *state = json!({"kind":"released"});
            if generation == "generation-navigation-lost-release" {
                return Some(None);
            }
        }
        if action == "query" {
            self.queries += 1;
            std::fs::write(home.join("nav-queries"), self.queries.to_string()).unwrap();
        }
        let mut reply = json!({"type":"ownedBufferNavigation","api_version":1,"navigation":navigation,"state":state});
        if action == "execute" {
            match generation {
                "generation-navigation-bad-owner" => {
                    reply["navigation"]["instance"] = json!("f".repeat(32))
                }
                "generation-navigation-bad-path" => {
                    reply["state"]["locations"][0]["path"] = json!("../outside")
                }
                "generation-navigation-bad-range" => {
                    reply["state"]["locations"][0]["end"]["column"] = json!(9)
                }
                "generation-navigation-prepared" => reply["state"] = json!({"kind":"prepared"}),
                "generation-navigation-released" => reply["state"] = json!({"kind":"released"}),
                "generation-navigation-extra" => reply["state"]["authorized"] = json!(true),
                _ => {}
            }
        }
        Some(Some(reply))
    }
}
