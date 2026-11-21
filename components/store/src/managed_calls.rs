//! Closed input contract for managed cross-Provider calls. A validated request
//! is caller input, never a parent identity or permission to execute.

#[cfg(unix)]
pub mod cli;
#[cfg(unix)]
pub mod gateway;
#[cfg(all(unix, feature = "machine-host"))]
pub mod host;
#[cfg(unix)]
pub mod round;
#[cfg(all(unix, feature = "machine-host"))]
pub mod snapshot;

#[cfg(feature = "full")]
pub mod authority;
pub use cowboy_wire::managed_calls::lifecycle;
pub mod media;
pub use cowboy_wire::managed_calls::protocol;
pub use cowboy_wire::managed_calls::review;
#[cfg(feature = "full")]
pub mod service;

pub use cowboy_wire::managed_calls::*;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn example() -> Value {
        json!({"schema":1,"request_id":"review-r2-security-01","purpose":"review",
            "instruction":"Review this round.","context":{"scope":"current-worktree","files":["review/input.md"]},
            "access":"read-only","conversation":{"mode":"fresh"}})
    }

    fn parse(value: &Value) -> Result<Request, InputError> {
        Request::parse(&serde_json::to_vec(value).unwrap())
    }

    #[test]
    fn image_calls_are_explicit_bounded_and_keep_review_constraints() {
        let mut value = example();
        value["purpose"] = json!("image_generate");
        value["context"]["files"] = json!([]);
        let image = parse(&value).unwrap();
        assert!(image.purpose.is_image());
        assert_eq!(
            image.purpose.profile(),
            cowboy_provider_sdk::ManagedRuntimeProfile::ImageV1
        );
        value["purpose"] = json!("image_edit");
        assert!(parse(&value).is_err());
        value["conversation"] = json!({"mode":"continue","child_session_id":"image-child"});
        assert!(parse(&value).is_ok());
        value["output"] = json!({"format":"json_schema","schema":{"type":"object"}});
        assert!(parse(&value).is_err());
        value["output"] = json!({"format":"text"});
        value["context"]["files"] = json!(["1.png", "2.png", "3.png", "4.png", "5.png"]);
        assert!(parse(&value).is_err());
        let review = parse(&example()).unwrap();
        assert_eq!(review.prompt(), review.instruction);
        assert_eq!(
            review.purpose.profile(),
            cowboy_provider_sdk::ManagedRuntimeProfile::ReadOnlyV1
        );
    }

    #[test]
    fn native_output_schema_is_preserved_and_bound_to_request_identity() {
        let mut value = example();
        let text = parse(&value).unwrap();
        value["output"] = json!({"format":"text"});
        assert_eq!(parse(&value).unwrap().digest(), text.digest());
        value["output"]["ignored"] = json!(true);
        assert!(parse(&value).is_err());
        let schema = json!({"type":"object","additionalProperties":false,
            "properties":{"verdict":{"type":"string","enum":["approve","needs-attention"]}},
            "required":["verdict"]});
        value["output"] = json!({"format":"json_schema","schema":schema});
        let structured = parse(&value).unwrap();
        assert_eq!(structured.instruction, text.instruction);
        assert_ne!(structured.digest(), text.digest());
        assert_eq!(
            serde_json::to_value(&structured).unwrap()["output"]["schema"],
            schema
        );
        value["output"]["schema"]["properties"]["verdict"]["enum"] = json!(["approve"]);
        assert_ne!(parse(&value).unwrap().digest(), structured.digest());
        for schema in [
            json!({"$ref":"https://example.invalid/schema"}),
            json!({"$ref":"file:///private/schema"}),
            json!({"$dynamicRef":"https://example.invalid/schema"}),
            json!({"$id":"https://example.invalid/root","$ref":"#/$defs/result"}),
            json!({"description":"x".repeat(65536)}),
            json!(true),
        ] {
            value["output"]["schema"] = schema;
            assert_eq!(parse(&value).err(), Some(InputError::InvalidOutput));
        }
        let mut nested = json!({"type":"string"});
        for _ in 0..34 {
            nested = json!({"items":nested});
        }
        value["output"]["schema"] = nested;
        assert_eq!(parse(&value).err(), Some(InputError::InvalidOutput));
    }

    #[test]
    fn requests_cannot_select_their_parent_or_escalate_access() {
        for field in [
            "parent_session_id",
            "machine_id",
            "capability",
            "auth",
            "command",
        ] {
            let mut value = example();
            value[field] = json!("spoof");
            assert_eq!(parse(&value).unwrap_err(), InputError::InvalidContract);
        }
        let mut value = example();
        value["access"] = json!("full-access");
        assert_eq!(parse(&value).unwrap_err(), InputError::InvalidContract);
    }

    #[test]
    fn context_rejects_traversal_absolute_paths_and_ambiguous_names() {
        for path in [
            "../secret",
            "a/../../secret",
            "/tmp/secret",
            "a\\secret",
            "file:///secret",
            "a//b",
            "./input",
            "a/./b",
            "x\ny",
        ] {
            let mut value = example();
            value["context"]["files"] = json!([path]);
            assert_eq!(
                parse(&value).unwrap_err(),
                InputError::InvalidContext,
                "{path}"
            );
        }
    }

    #[test]
    fn context_root_is_an_exact_absolute_target_path() {
        let mut value = example();
        value["context"]["root"] = json!("/srv/worktrees/marketplace-service/task");
        assert!(parse(&value).is_ok());
        for root in [
            "relative/path",
            "/",
            "/a/../b",
            "/a/./b",
            "/a//b",
            "/a/b/",
            "/a\nb",
        ] {
            value["context"]["root"] = json!(root);
            assert_eq!(
                parse(&value).unwrap_err(),
                InputError::InvalidContext,
                "{root}"
            );
        }
    }

    #[test]
    fn exact_conversation_identity_is_required_for_continuation() {
        let mut value = example();
        value["conversation"] = json!({"mode":"continue"});
        assert!(parse(&value).is_err());
        value["conversation"] = json!({"mode":"continue","child_session_id":"child-1"});
        assert!(parse(&value).is_ok());
        value["conversation"] = json!({"mode":"fresh","child_session_id":"child-1"});
        assert!(parse(&value).is_err());
    }

    #[test]
    fn digest_is_order_independent_but_prompt_sensitive() {
        let mut value = example();
        value["labels"] = json!({"round":"2","aspect":"security"});
        let a = parse(&value).unwrap();
        let pretty = serde_json::to_vec_pretty(&value).unwrap();
        assert_eq!(a.digest(), Request::parse(&pretty).unwrap().digest());
        value["instruction"] = json!("Different task");
        assert_ne!(a.digest(), parse(&value).unwrap().digest());
        // Native schemas keep their caller order, but identity ignores it.
        value["output"] =
            json!({"format":"json_schema","schema":{"type":"object","required":["a"]}});
        let ordered = parse(&value).unwrap();
        let text = serde_json::to_string(&value).unwrap().replace(
            r#"{"type":"object","required":["a"]}"#,
            r#"{"required":["a"],"type":"object"}"#,
        );
        let reordered = Request::parse(text.as_bytes()).unwrap();
        assert_ne!(
            serde_json::to_string(&ordered).unwrap(),
            serde_json::to_string(&reordered).unwrap()
        );
        assert_eq!(ordered.digest(), reordered.digest());
    }

    #[test]
    fn duplicate_fields_and_large_inputs_fail_without_echoing_source() {
        let text = serde_json::to_string(&example()).unwrap();
        let duplicated = text.replacen("{", "{\"schema\":1,", 1);
        assert_eq!(
            Request::parse(duplicated.as_bytes()).unwrap_err(),
            InputError::InvalidContract
        );
        assert_eq!(
            Request::parse(&vec![b' '; MAX_REQUEST_BYTES + 1]).unwrap_err(),
            InputError::RequestTooLarge
        );
        assert_eq!(InputError::InvalidContract.to_string(), "invalid_contract");
        assert_eq!(
            format!("{:?}", parse(&example()).unwrap()),
            "ManagedCallRequest([private])"
        );
    }
}
