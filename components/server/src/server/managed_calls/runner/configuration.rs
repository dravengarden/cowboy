//! A managed prompt must wait for its actual preset, not optimistic UI state.
use super::*;
#[cfg(test)]
use crate::server::native_configuration::confirmed;
use crate::server::native_configuration::matches;

pub(super) struct Failure {
    pub code: &'static str,
    pub detail: Option<String>,
}

impl From<&'static str> for Failure {
    fn from(code: &'static str) -> Self {
        Self { code, detail: None }
    }
}

pub(super) async fn ready(state: &AppState, store: &Store, record: &Record) -> Result<(), Failure> {
    let child = &record.child_session_id;
    // Reapply the immutable call intent on recovery too: option reconciliation
    // may have retired an unsupported value since the original registration.
    if record.preset.is_some() {
        apply_preset(
            state,
            record,
            record
                .provider_version
                .as_deref()
                .ok_or("preset_unavailable")?,
            record
                .provider_generation_digest
                .as_deref()
                .ok_or("preset_unavailable")?,
        )?;
    }
    // Capture before native startup: later option reconciliation may retire an
    // unsupported preference. That must not turn an explicit choice into a
    // successful default-model call.
    let expected = state
        .hub
        .config_preferences(child)
        .ok_or("configuration_unavailable")?;
    state
        .supervisor
        .start_registered_session(child)
        .map_err(|_| "provider_unavailable")?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        if *state.shutdown.borrow() {
            return Err(RETRY.into());
        }
        let current = store
            .managed_call(&record.placement.parent_session_id, &record.call_id)
            .await
            .map_err(|_| RETRY)?
            .ok_or("call_unavailable")?;
        if current.revision != record.revision {
            return Err(RETRY.into());
        }
        if state
            .hub
            .session_info(&record.placement.parent_session_id)
            .is_none_or(|parent| parent.meta.closing)
        {
            return Err("parent_unavailable".into());
        }
        if matches(
            state.supervisor.observed_configuration(child).as_ref(),
            &expected,
        ) {
            return Ok(());
        }
        if matches!(
            state.hub.status(child),
            Some(Status::Crashed | Status::Interrupted)
        ) {
            // Startup refusal (for example memory admission or authentication)
            // is not evidence that the requested preset is unsupported.
            return Err(Failure {
                code: "provider_startup_failed",
                detail: state.hub.latest_crash_detail(child),
            });
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(Failure {
                code: "configuration_unconfirmed",
                detail: Some(mismatch_detail(
                    state.supervisor.observed_configuration(child).as_ref(),
                    &expected,
                )),
            });
        }
        tokio::time::sleep(POLL).await;
    }
}

fn mismatch_detail(options: Option<&Value>, expected: &Value) -> String {
    let observed: serde_json::Map<String, Value> = options
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|option| {
            let key = option["id"].as_str()?;
            expected.get(key)?;
            Some((key.to_owned(), option["currentValue"].clone()))
        })
        .collect();
    format!("expected={expected}; observed={}", Value::Object(observed))
        .chars()
        .take(512)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_equivalence_survives_the_worker_acp_codec() {
        let value = json!({
            "id":"model", "name":"Model", "type":"select",
            "currentValue":"claude-fable-5-1", "options":[],
            "_meta":{"cowboy.dev/configuration-equivalence":{
                "schema":1,"value":"claude-fable-5-1",
                "equivalent_values":["claude-fable-5-1[1m]"]
            }}
        });
        let decoded: agent_client_protocol::schema::v1::SessionConfigOption =
            serde_json::from_value(value).unwrap();
        let observed = serde_json::to_value(decoded).unwrap();
        assert!(confirmed(&observed, &json!("claude-fable-5-1[1m]")));
        assert_eq!(observed["currentValue"], "claude-fable-5-1");
    }

    #[test]
    fn provider_equivalence_is_bound_to_the_actual_value_and_preserves_effort() {
        let expected = json!({"model":"claude-fable-5-1[1m]", "effort":"max"});
        let mut options = json!([
            {"id":"model", "currentValue":"claude-fable-5-1", "_meta":{
                "cowboy.dev/configuration-equivalence":{
                    "schema":1,"value":"claude-fable-5-1",
                    "equivalent_values":["claude-fable-5-1[1m]"]
                }
            }},
            {"id":"effort", "currentValue":"max"}
        ]);
        assert!(matches(Some(&options), &expected));
        options[1]["currentValue"] = json!("high");
        assert!(!matches(Some(&options), &expected));
        options[1]["currentValue"] = json!("max");
        options[0]["currentValue"] = json!("opus");
        assert!(!matches(Some(&options), &expected));
        options[0]["currentValue"] = json!("claude-fable-5-1");
        options[0]["_meta"]["cowboy.dev/configuration-equivalence"]["schema"] = json!(2);
        assert!(!matches(Some(&options), &expected));
        options[0].as_object_mut().unwrap().remove("_meta");
        assert!(!matches(Some(&options), &expected));
        assert!(mismatch_detail(Some(&options), &expected).contains("observed="));
        assert!(mismatch_detail(None, &expected).contains("observed={}"));
    }

    #[test]
    fn explicit_preset_needs_every_native_value_and_never_accepts_hidden_options() {
        let expected = json!({"model":"fable","effort":"max"});
        assert!(!matches(None, &expected));
        assert!(!matches(Some(&json!([])), &expected));
        assert!(!matches(
            Some(&json!([
                {"id":"model","currentValue":"opus"}, {"id":"effort","currentValue":"max"}
            ])),
            &expected
        ));
        assert!(!matches(
            Some(&json!([
                {"id":"model","currentValue":"fable"}, {"id":"effort","currentValue":"high"}
            ])),
            &expected
        ));
        assert!(matches(
            Some(&json!([
                {"id":"model","currentValue":"fable"}, {"id":"effort","currentValue":"max"}
            ])),
            &expected
        ));
        assert!(!matches(None, &json!({})));
        assert!(matches(Some(&json!([])), &json!({})));
    }
}
