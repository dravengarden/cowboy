//! Model evidence for choosing a managed-call preset, served on demand as the
//! `models` help topic and never in a resident tool description. The
//! model-intel skill generates the summary from versioned evidence; the
//! Controller only embeds and serves it, so a catalog refresh needs a
//! Controller release but never a Machine release.

use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

const SUMMARY: &str = include_str!("../../../../models/summary.md");

const USE: &str = "Explains preset trade-offs for managed calls; it never authorizes a model. An explicit user choice, session preference or Settings target wins. For Review/Design Review follow calls.capabilities().review_selection and declare risk. An absent tier means the evidence cannot decide: keep the current selection. Keep this revision and do not reread while you hold it.";

pub fn help() -> Value {
    json!({
        "api": crate::agent_api::API_VERSION,
        "revision": revision(),
        "use": USE,
        "summary": SUMMARY,
    })
}

fn revision() -> &'static str {
    static REVISION: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        let digest = Sha256::digest(SUMMARY.as_bytes());
        format!("{digest:x}")[..16].to_owned()
    });
    &REVISION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_models_topic_is_small_and_self_describing() {
        let value = help();
        let bytes = serde_json::to_vec(&value).unwrap().len();
        // One on-demand read; keep it well under a single result page.
        assert!(bytes < 4096, "models help: {bytes} bytes");
        assert!(
            value["summary"]
                .as_str()
                .unwrap()
                .starts_with("# Model routing summary")
        );
        assert_eq!(value["revision"].as_str().unwrap().len(), 16);
    }
}
