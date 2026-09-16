//! Test-process-only adapter protocol fixture. No product dispatch imports it.
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Default)]
pub(super) struct Fixture {
    last: u64,
    slots: BTreeMap<String, (Value, Value)>,
    applies: u64,
    retires: u64,
}

impl Fixture {
    pub(super) fn reply(
        &mut self,
        request: &Value,
        home: &Path,
        generation: &str,
    ) -> Option<Option<Value>> {
        let kind = request["type"].as_str()?;
        if kind == "bufferSyncOwnerSupport" {
            let response = if generation == "generation-sync-old" {
                json!({"type":"nativeSyncSupport","api_version":1,"protocol":1,"instance":"a".repeat(32)})
            } else {
                json!({"type":"bufferSyncOwnerSupport","api_version":1,"protocol":1})
            };
            return Some(Some(response));
        }
        if kind != "prepareBufferSync" && kind != "bufferSync" {
            return None;
        }
        let operation = if kind == "prepareBufferSync" {
            self.last += 1;
            std::fs::write(home.join("sync-prepares"), self.last.to_string()).unwrap();
            let id = format!("{:016x}", self.last);
            self.slots.insert(
                id.clone(),
                (request["content"].clone(), json!({"kind":"prepared"})),
            );
            json!({"instance":format!("{:032x}",std::process::id()),"id":id})
        } else {
            request["operation"].clone()
        };
        let id = operation["id"].as_str().unwrap();
        let (content, state) = self.slots.get_mut(id).unwrap();
        if request["action"] == "apply" {
            self.applies += 1;
            std::fs::write(home.join("sync-applies"), self.applies.to_string()).unwrap();
            *state = json!({"kind":"applied","content":content,"version":[{"replicaId":0,"timestamp":1}]});
            if generation == "generation-sync-pause" {
                for _ in 0..500 {
                    if home.join("resume-sync").exists() {
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                assert!(
                    home.join("resume-sync").exists(),
                    "fixture synchronization never resumed"
                );
            }
            if generation == "generation-sync-lost" {
                return Some(None);
            }
        } else if request["action"] == "retire" {
            self.retires += 1;
            std::fs::write(home.join("sync-retires"), self.retires.to_string()).unwrap();
            *state = json!({"kind":"retired"});
            if generation == "generation-sync-lost-retire" {
                return Some(None);
            }
        }
        let mut reply =
            json!({"type":"bufferSync","api_version":1,"operation":operation,"state":state});
        if request["action"] == "apply" {
            match generation {
                "generation-sync-bad-content" => {
                    reply["state"]["content"]["sha256"] = json!("f".repeat(64))
                }
                "generation-sync-bad-owner" => {
                    reply["operation"]["instance"] = json!("f".repeat(32))
                }
                "generation-sync-retired" => reply["state"] = json!({"kind":"retired"}),
                "generation-sync-pending" => reply["state"] = json!({"kind":"pending"}),
                _ => {}
            }
        }
        Some(Some(reply))
    }
}
