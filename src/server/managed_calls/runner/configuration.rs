//! A managed prompt must wait for its actual preset, not optimistic UI state.
use super::*;

fn matches(options: Option<&Value>, expected: &Value) -> bool {
    let Some(options) = options.and_then(Value::as_array) else {
        return false;
    };
    expected.as_object().is_some_and(|expected| {
        expected.iter().all(|(key, value)| {
            options.iter().any(|option| {
                option["id"].as_str() == Some(key) && option.get("currentValue") == Some(value)
            })
        })
    })
}

pub(super) async fn ready(
    state: &AppState,
    store: &Store,
    record: &Record,
) -> Result<(), &'static str> {
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
            return Err(RETRY);
        }
        let current = store
            .managed_call(&record.placement.parent_session_id, &record.call_id)
            .await
            .map_err(|_| RETRY)?
            .ok_or("call_unavailable")?;
        if current.revision != record.revision {
            return Err(RETRY);
        }
        if state
            .hub
            .session_info(&record.placement.parent_session_id)
            .is_none_or(|parent| parent.meta.closing)
        {
            return Err("parent_unavailable");
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
            return Err("configuration_unavailable");
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("configuration_unconfirmed");
        }
        tokio::time::sleep(POLL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
