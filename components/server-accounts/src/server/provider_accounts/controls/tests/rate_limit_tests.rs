//! Rate limits scoped to a model quota group: Antigravity meters Gemini and
//! third-party (Claude and GPT) models separately, so one group's refusal
//! parks only that group's sessions, until the group's quota resets.
use super::*;

const CLAUDE: &str = "claude-opus-5-5";
const GEMINI: &str = "gemini-4-neon";

fn rate_limited() -> String {
    proxy_error(
        "antigravity_backend_refusal:rate_limited",
        &json!({"antigravity_backend": {"class": "rate_limited"}}),
    )
}

/// A usage observation whose Claude and GPT five-hour window is used up
/// until `reset_ms` while Gemini has quota left.
fn claude_exhausted(reset_ms: i64) -> Value {
    json!({"rateLimitsByLimitId": {
        "0001-gemini-models": {"limitName": "Gemini Models",
            "primary": {"usedPercent": 0, "resetsAt": reset_ms / 1000 + 600, "windowDurationMins": 300}},
        "0100-claude-and-gpt-models": {"limitName": "Claude and GPT models",
            "primary": {"usedPercent": 50, "resetsAt": reset_ms / 1000 + 86_400, "windowDurationMins": 10080}},
        "0101-claude-and-gpt-models": {"limitName": "Claude and GPT models",
            "primary": {"usedPercent": 100, "resetsAt": reset_ms / 1000, "windowDurationMins": 300}},
    }})
}

#[tokio::test]
async fn a_claude_refusal_parks_only_claude_sessions_until_its_quota_resets() {
    let (fake, controls, _root) = fixture().await;
    fake.models.lock().insert("s1".into(), CLAUDE.into());
    fake.models.lock().insert("s2".into(), GEMINI.into());
    let reset = (now_ms() / 1000 + 4 * 3600) * 1000;
    *fake.rate_limits.lock() = Some(claude_exhausted(reset));
    end(&controls, &fake, "s1", &rate_limited()).await;
    let limited = control(&controls).await;
    assert_eq!(limited.state, AccountState::RateLimited);
    assert_eq!(limited.reason, "rate_limited:claude_gpt");
    assert_eq!(limited.until_ms, Some(reset));
    parked(controls.before_dispatch("s1").await, "Claude and GPT quota");
    assert!(controls.revival_gate(&meta(&fake, "s1")).is_err());
    // The Gemini session keeps running and leaves the limit in place.
    controls.before_dispatch("s2").await.unwrap();
    assert!(controls.parked_reason(&meta(&fake, "s2")).is_none());
    assert!(controls.revival_gate(&meta(&fake, "s2")).is_ok());
    assert_eq!(control(&controls).await.state, AccountState::RateLimited);
    // A session that switches to Claude parks too.
    fake.models.lock().insert("s2".into(), CLAUDE.into());
    parked(controls.before_dispatch("s2").await, "until");
}

#[tokio::test]
async fn without_a_usage_reset_a_scoped_limit_holds_the_default_hour() {
    let (fake, controls, _root) = fixture().await;
    fake.models.lock().insert("s1".into(), GEMINI.into());
    let before = now_ms();
    end(&controls, &fake, "s1", &rate_limited()).await;
    let limited = control(&controls).await;
    assert_eq!(limited.reason, "rate_limited:gemini");
    let until = limited.until_ms.unwrap();
    assert!(
        until >= before + policy::DEFAULT_RATE_LIMIT_MS
            && until <= now_ms() + policy::DEFAULT_RATE_LIMIT_MS
    );
}

#[tokio::test]
async fn an_unknown_model_parks_every_session_of_the_account() {
    let (fake, controls, _root) = fixture().await;
    fake.models.lock().insert("s2".into(), GEMINI.into());
    end(&controls, &fake, "s1", &rate_limited()).await;
    assert_eq!(control(&controls).await.reason, "rate_limited");
    parked(controls.before_dispatch("s2").await, "until");
    // A scoped limit parks a session whose model is unknown as well.
    let (fake, controls, _root) = fixture().await;
    fake.models.lock().insert("s1".into(), CLAUDE.into());
    end(&controls, &fake, "s1", &rate_limited()).await;
    parked(controls.before_dispatch("s2").await, "until");
}

#[tokio::test]
async fn a_second_groups_refusal_widens_the_limit_to_the_account() {
    let (fake, controls, _root) = fixture().await;
    fake.models.lock().insert("s1".into(), CLAUDE.into());
    fake.models.lock().insert("s2".into(), GEMINI.into());
    let reset = (now_ms() / 1000 + 4 * 3600) * 1000;
    *fake.rate_limits.lock() = Some(claude_exhausted(reset));
    end(&controls, &fake, "s1", &rate_limited()).await;
    // The same group again keeps the scope.
    end(&controls, &fake, "s1", &rate_limited()).await;
    assert_eq!(control(&controls).await.reason, "rate_limited:claude_gpt");
    end(&controls, &fake, "s2", &rate_limited()).await;
    let widened = control(&controls).await;
    assert_eq!(widened.reason, "rate_limited");
    assert_eq!(widened.until_ms, Some(reset));
    parked(controls.before_dispatch("s2").await, "until");
}

#[tokio::test]
async fn an_ended_limit_whose_quota_is_still_used_up_is_extended_not_cleared() {
    let (fake, controls, _root) = fixture().await;
    fake.models.lock().insert("s1".into(), CLAUDE.into());
    fake.models.lock().insert("s2".into(), GEMINI.into());
    let current = control(&controls).await;
    controls
        .enter(
            &current,
            AccountState::RateLimited,
            "rate_limited:claude_gpt",
            Some(now_ms() - 1),
            None,
            false,
        )
        .await
        .unwrap();
    let reset = (now_ms() / 1000 + 3600) * 1000;
    *fake.rate_limits.lock() = Some(claude_exhausted(reset));
    // The Gemini dispatch proceeds; the Claude limit runs on to its reset.
    controls.before_dispatch("s2").await.unwrap();
    let extended = control(&controls).await;
    assert_eq!(extended.state, AccountState::RateLimited);
    assert_eq!(extended.reason, "rate_limited:claude_gpt");
    assert_eq!(extended.until_ms, Some(reset));
    parked(controls.before_dispatch("s1").await, "until");
    // Once usage shows the quota back, the next dispatch resumes the account.
    let current = control(&controls).await;
    controls
        .enter(
            &current,
            AccountState::RateLimited,
            "rate_limited:claude_gpt",
            Some(now_ms() - 1),
            None,
            false,
        )
        .await
        .unwrap();
    *fake.rate_limits.lock() = Some(claude_exhausted(now_ms() - 60_000));
    controls.before_dispatch("s1").await.unwrap();
    let resumed = control(&controls).await;
    assert_eq!(resumed.state, AccountState::Ok);
    assert_eq!(resumed.reason, "rate_limit_ended");
}
