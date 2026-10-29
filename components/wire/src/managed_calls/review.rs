//! Explicit, snapshot-bound review reuse. Labels carry this opt-in contract so
//! older gateways can transport it without a Machine/worker protocol change.
//! They never grant filesystem access, select an account or authorize a call.

use serde_json::{Value, json};

use super::{Conversation, Purpose, Request, lifecycle::Record};

pub const TASK: &str = "review_task";
pub const BASE: &str = "review_base";
pub const REUSE: &str = "review_reuse";
/// Label value `parent` asks the Controller to queue one completion notice
/// into the parent session instead of the parent polling `calls.wait`.
pub const NOTIFY: &str = "notify";

pub fn valid_base(base: &str) -> bool {
    matches!(base.len(), 40 | 64) && base.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub fn validate(request: &Request) -> bool {
    let labels = &request.labels;
    if ![TASK, BASE, REUSE]
        .iter()
        .any(|key| labels.contains_key(*key))
    {
        return true;
    }
    matches!(request.purpose, Purpose::Review | Purpose::DesignReview)
        && request.context.root.is_some()
        && matches!(&request.output, super::OutputFormat::JsonSchema { schema } if *schema == output_schema())
        && labels.get(TASK).is_some_and(|task| super::valid_id(task))
        && labels.get(BASE).is_some_and(|base| valid_base(base))
        && labels
            .get(REUSE)
            .is_some_and(|reuse| matches!(reuse.as_str(), "true" | "false"))
}

/// Standard wrappers use an explicit completeness signal. A native end-turn
/// alone cannot certify a review that was blocked by unavailable evidence.
pub fn output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "review_complete": {"type":"boolean"},
            "verdict": {"type":"string","enum":["approve","needs-attention","incomplete"]},
            "summary": {"type":"string"},
            "findings": {"type":"array","items":{
                "type":"object",
                "properties":{
                    "severity":{"type":"string","enum":["critical","high","medium","low"]},
                    "title":{"type":"string"},
                    "body":{"type":"string"},
                    "file":{"type":"string"},
                    "line_start":{"type":"integer"},
                    "line_end":{"type":"integer"}
                },
                "required":["severity","title","body","file","line_start","line_end"],
                "additionalProperties":false
            }}
        },
        "required":["review_complete","verdict","summary","findings"],
        "additionalProperties":false
    })
}

pub fn prompt(request: &Request) -> Option<String> {
    let base = request.labels.get(BASE)?;
    let contract = input_contract(base);
    // Wrappers persist the scope in the instruction too: rolling back the
    // Controller must not silently reinterpret an accepted base-bound review.
    Some(if request.instruction.ends_with(&contract) {
        request.instruction.clone()
    } else {
        format!("{}\n{contract}", request.instruction)
    })
}

pub fn input_contract(base: &str) -> String {
    format!(
        "Review input contract: compare the current immutable snapshot against base commit {base}. First confirm that commit exists in the snapshot repository; if it does not, stop without reviewing and report review_complete=false and verdict=incomplete naming the missing base. Read files from the current snapshot, not the source checkout. All requirements and external evidence must be supplied in this request or captured files; do not consult mutable external paths or branch refs. Return JSON matching the supplied output schema. Report review_complete=false and verdict=incomplete only if a required input, such as a referenced requirement, design or file, is missing from this request and the captured files, or you could not read the changed code. This is a static review: project toolchains, installed dependencies and browsers may be unavailable in the snapshot, and a check you cannot execute does not make the review incomplete; name it in the summary instead. A completed review may still have findings and verdict=needs-attention."
    )
}

/// Native conversation history is another mutable input: continued calls are
/// never content-deduplicated. Model/account/generation changes also invalidate
/// reuse. Titles, selection explanations and round display labels do not.
pub fn identity(
    request: &Request,
    record: &Record,
    input_revision: &str,
    account: &crate::provider_account::ProviderAccountId,
    authentication_generation: Option<u64>,
) -> Option<String> {
    if request.labels.get(REUSE).map(String::as_str) != Some("true")
        || !matches!(request.conversation, Conversation::Fresh {})
        || !validate(request)
    {
        return None;
    }
    Some(super::canonical_digest(&json!({
        "schema":1,
        "task":request.labels.get(TASK),
        "base":request.labels.get(BASE),
        "placement":record.placement,
        "root":request.context.root,
        "input_revision":input_revision,
        "purpose":request.purpose,
        "instruction":request.prompt(),
        "output":request.output,
        "provider":record.provider,
        "generation":record.provider_generation_digest,
        "runtime":record.runtime_machine_id,
        "preset":record.preset,
        "account":account,
        "authentication_generation":authentication_generation,
    })))
}

pub fn complete(record: &Record) -> bool {
    record.state == super::lifecycle::State::Completed
        && record.result.as_ref().is_some_and(|result| {
            result.pointer("/structured/review_complete") == Some(&Value::Bool(true))
                && matches!(
                    result
                        .pointer("/structured/verdict")
                        .and_then(Value::as_str),
                    Some("approve" | "needs-attention")
                )
                && result.get("truncated") != Some(&Value::Bool(true))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> Request {
        Request::parse(&serde_json::to_vec(&json!({
            "schema":1,"request_id":"op","purpose":"review","instruction":"Check the ownership invariant.",
            "context":{"scope":"current-worktree","root":"/tasks/project"},
            "access":"read-only","conversation":{"mode":"fresh"},
            "output":{"format":"json_schema","schema":output_schema()},
            "labels":{"review_task":"task-1","review_base":"a".repeat(40),"review_reuse":"true"}
        })).unwrap()).unwrap()
    }

    #[test]
    fn scoped_review_cannot_bypass_completeness_schema() {
        let mut request = request();
        assert!(request.validate().is_ok());
        request.output = super::super::OutputFormat::Text {};
        assert!(request.validate().is_err());
        request.output = super::super::OutputFormat::JsonSchema {
            schema: json!({"type":"object"}),
        };
        assert!(request.validate().is_err());
    }

    fn record() -> Record {
        serde_json::from_value(json!({
            "schema":1,"call_id":"call-1","request_id":"op","request_digest":"digest",
            "provider":"codex","purpose":"review","access":"read-only","labels":{},
            "placement":{"service_id":"s","parent_session_id":"p","parent_revision":"r",
                "machine_id":"m","workspace_id":"w","cwd":"/workspace"},
            "child_session_id":"child","state":"running","revision":1,
            "created_at_ms":1,"updated_at_ms":1,"preset":"astra",
            "provider_generation_digest":"generation","runtime_machine_id":"runtime"
        }))
        .unwrap()
    }

    #[test]
    fn standard_review_persists_its_exact_scope_without_duplicate_guidance() {
        let mut request = request();
        request.instruction = super::super::review_instruction(
            false,
            "Check persistence.",
            request.labels.get(BASE).map(String::as_str),
        );
        assert_eq!(request.prompt(), request.instruction);
        assert!(!request.instruction.contains("default branch"));
        assert!(!request.instruction.contains("reply exactly: No findings"));
        assert!(request.instruction.contains(&"a".repeat(40)));
    }

    #[test]
    fn reuse_requires_complete_input_and_ignores_only_display_metadata() {
        let request = request();
        let record = record();
        let account = crate::provider_account::ProviderAccountId::default();
        let original = identity(&request, &record, "snapshot", &account, None).unwrap();
        assert_ne!(
            identity(&request, &record, "snapshot", &account, Some(1)).unwrap(),
            original,
        );
        assert_ne!(
            identity(&request, &record, "snapshot", &account, Some(1)),
            identity(&request, &record, "snapshot", &account, Some(2)),
        );
        let mut renamed = request.clone();
        renamed.request_id = "different-operation".into();
        renamed
            .labels
            .insert("title".into(), "Another title".into());
        renamed.labels.insert("round".into(), "9".into());
        assert_eq!(
            identity(&renamed, &record, "snapshot", &account, None).unwrap(),
            original
        );
        for key in [TASK, BASE, REUSE] {
            let mut missing = request.clone();
            missing.labels.remove(key);
            assert!(!validate(&missing));
            assert!(identity(&missing, &record, "snapshot", &account, None).is_none());
        }
        let mut missing = request.clone();
        missing.context.root = None;
        assert!(!validate(&missing));
        for (key, value) in [(TASK, "other"), (BASE, &"b".repeat(40))] {
            let mut different = request.clone();
            different.labels.insert(key.into(), value.into());
            assert_ne!(
                identity(&different, &record, "snapshot", &account, None).unwrap(),
                original
            );
        }
        let mut different = request.clone();
        different.context.root = Some("/tasks/another".into());
        assert_ne!(
            identity(&different, &record, "snapshot", &account, None).unwrap(),
            original
        );
        different = request.clone();
        different.instruction.push_str(" Check cancellation too.");
        assert_ne!(
            identity(&different, &record, "snapshot", &account, None).unwrap(),
            original
        );
        assert_ne!(
            identity(&request, &record, "changed-snapshot", &account, None).unwrap(),
            original
        );
        let mut changed_model = record.clone();
        changed_model.preset = Some("opus".into());
        assert_ne!(
            identity(&request, &changed_model, "snapshot", &account, None).unwrap(),
            original
        );
        changed_model = record.clone();
        changed_model.placement.parent_session_id = "another-parent".into();
        assert_ne!(
            identity(&request, &changed_model, "snapshot", &account, None).unwrap(),
            original
        );
        different = request.clone();
        different.labels.insert(REUSE.into(), "false".into());
        assert!(identity(&different, &record, "snapshot", &account, None).is_none());
        different = request;
        different.conversation = Conversation::Continue {
            child_session_id: "child".into(),
        };
        assert!(identity(&different, &record, "snapshot", &account, None).is_none());
    }

    #[test]
    fn completion_is_coverage_evidence_not_an_approval() {
        let mut record = record();
        record.state = super::super::lifecycle::State::Completed;
        for verdict in ["approve", "needs-attention"] {
            record.result = Some(json!({"structured":{"review_complete":true,"verdict":verdict}}));
            assert!(complete(&record));
        }
        for result in [
            json!({"text":"No findings."}),
            json!({"structured":{"review_complete":false,"verdict":"approve"}}),
            json!({"structured":{"review_complete":true,"verdict":"incomplete"}}),
            json!({"truncated":true,"structured":{"review_complete":true,"verdict":"approve"}}),
        ] {
            record.result = Some(result);
            assert!(!complete(&record));
        }
        record.result = Some(json!({"structured":{"review_complete":true,"verdict":"approve"}}));
        record.state = super::super::lifecycle::State::Failed;
        assert!(!complete(&record));
    }
}
