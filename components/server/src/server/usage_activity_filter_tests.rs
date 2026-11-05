
use super::{UsageActivityQuery, parse_usage_activity_filter};

const MODELS: &[&str] = &["flash", "pro"];
const AGENTS: &[&str] = &["codex", "claude"];

#[test]
fn defaults_to_bounded_unfiltered_activity() {
    let filter = parse_usage_activity_filter(&UsageActivityQuery::default(), MODELS, AGENTS)
        .expect("default filter");
    assert_eq!(filter.window, "24h");
    assert_eq!(filter.to_ms - filter.from_ms, 86_400_000);
    assert!(filter.models.is_empty());
    assert!(filter.agents.is_empty());
}

#[test]
fn accepts_agent_and_model_family_filters() {
    let filter = parse_usage_activity_filter(
        &UsageActivityQuery {
            window: Some("30d".to_owned()),
            model: Some("pro,flash".to_owned()),
            agent: Some("codex,claude".to_owned()),
            ..UsageActivityQuery::default()
        },
        MODELS,
        AGENTS,
    )
    .expect("multi filter");
    assert_eq!(filter.to_ms - filter.from_ms, 30 * 86_400_000);
    assert_eq!(filter.models, ["pro", "flash"]);
    assert_eq!(filter.agents, ["codex", "claude"]);
}

#[test]
fn accepts_short_rolling_windows() {
    for (window, window_seconds) in [
        ("2h", 2 * 3_600),
        ("4h", 4 * 3_600),
        ("8h", 8 * 3_600),
        ("12h", 12 * 3_600),
    ] {
        let filter = parse_usage_activity_filter(
            &UsageActivityQuery {
                window: Some(window.to_owned()),
                ..UsageActivityQuery::default()
            },
            MODELS,
            AGENTS,
        )
        .expect("rolling window");
        assert_eq!(filter.window, window);
        assert_eq!(
            filter.to_ms - filter.from_ms,
            i64::from(window_seconds) * 1_000
        );
    }
}

#[test]
fn accepts_an_exact_bounded_time_range() {
    let to_ms = super::now_ms();
    let filter = parse_usage_activity_filter(
        &UsageActivityQuery {
            from_ms: Some(to_ms - 2 * 3_600_000),
            to_ms: Some(to_ms),
            ..UsageActivityQuery::default()
        },
        MODELS,
        AGENTS,
    )
    .expect("custom range");
    assert_eq!(filter.window, "custom");
    assert_eq!(filter.from_ms, to_ms - 2 * 3_600_000);
    assert_eq!(filter.to_ms, to_ms);
}

#[test]
fn rejects_open_ended_dimensions() {
    for query in [
        UsageActivityQuery {
            window: Some("forever".to_owned()),
            ..UsageActivityQuery::default()
        },
        UsageActivityQuery {
            model: Some("deepseek-v4-pro[1m]".to_owned()),
            ..UsageActivityQuery::default()
        },
        UsageActivityQuery {
            agent: Some("unknown-runtime".to_owned()),
            ..UsageActivityQuery::default()
        },
    ] {
        assert!(parse_usage_activity_filter(&query, MODELS, AGENTS).is_err());
    }
}
