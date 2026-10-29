use super::*;

fn fixture() -> (UsageSnapshot, Vec<PluginUsageSpec>) {
    let bindings = ["future-account", "not-collected"]
        .into_iter()
        .map(|account| {
            serde_json::from_value(json!({"account":account,"product":"Future Provider"})).unwrap()
        })
        .collect();
    let usage = ProviderUsage {
        provider: "future-account",
        status: "available",
        source: "Provider API",
        observed_at_ms: 1234,
        account: Some(json!({"plan":"subscription"})),
        rate_limits: Some(
            json!({"rateLimits":{"primary":{"usedPercent":0,"windowDurationMins":10080,"resetsAt":2000}},"usageCredits":{"balanceMinor":100}}),
        ),
        activity: Some(
            json!({"history": "x".repeat(100_000),"last24Hours":{"summary":{"inputTokens":42}}}),
        ),
        error: Some("last refresh failed".into()),
        refresh: Some(crate::usage::ProviderRefreshState {
            last_attempt_at_ms: 2345,
            manual_refresh_after_ms: 3456,
            next_auto_refresh_at_ms: 3456,
            stale: true,
        }),
    };
    (
        UsageSnapshot {
            refreshed_at_ms: 1234,
            next_refresh_at_ms: 3456,
            refresh_interval_ms: 300_000,
            providers: vec![usage],
            reset_schedules: Default::default(),
            refreshing: vec![],
        },
        bindings,
    )
}

#[test]
fn accounts_are_compact_paged_and_missing_observations_stay_unknown() {
    let (mut snapshot, bindings) = fixture();
    let mut removed = snapshot.providers[0].clone();
    removed.provider = "removed-plugin";
    snapshot.providers.push(removed);
    let refreshing = BTreeSet::from(["future-account".into()]);
    let first = project(
        &snapshot,
        &bindings,
        &refreshing,
        "usage.list",
        &json!({"limit":1}),
    )
    .unwrap();
    assert_eq!(first["next"], "future-account");
    assert_eq!(first["accounts"][0]["refreshing"], true);
    assert_eq!(first["accounts"][0]["refresh"]["stale"], true);
    assert_eq!(first["accounts"][0]["sections"]["quota"], true);
    assert!(serde_json::to_string(&first).unwrap().len() < 1000);
    let last = project(
        &snapshot,
        &bindings,
        &refreshing,
        "usage.list",
        &json!({"after":first["next"],"limit":1}),
    )
    .unwrap();
    assert_eq!(last["next"], Value::Null);
    assert_eq!(last["accounts"][0]["status"], "unavailable");
    assert_eq!(last["accounts"][0]["observed_at_ms"], Value::Null);
    assert_eq!(last["accounts"][0]["sections"]["quota"], false);
    assert_eq!(last["accounts"].as_array().unwrap().len(), 1);
    let empty = project(
        &snapshot,
        &bindings,
        &refreshing,
        "usage.read",
        &json!({"account":"not-collected"}),
    )
    .unwrap();
    assert_eq!(empty["available"], false);
    assert_eq!(empty["page"]["data"], Value::Null);
    assert_eq!(
        project(
            &snapshot,
            &bindings,
            &refreshing,
            "usage.read",
            &json!({"account":"removed-plugin"})
        ),
        Err("not_found".into())
    );
}

#[test]
fn quota_preserves_provider_semantics_and_freshness_separately_from_activity() {
    let (snapshot, bindings) = fixture();
    let refreshing = BTreeSet::new();
    let quota = project(
        &snapshot,
        &bindings,
        &refreshing,
        "usage.read",
        &json!({"account":"future-account"}),
    )
    .unwrap();
    assert_eq!(
        quota["page"]["data"],
        *snapshot.providers[0].rate_limits.as_ref().unwrap()
    );
    assert_eq!(quota["observed_at_ms"], 1234);
    assert_eq!(quota["refresh"]["stale"], true);
    assert_eq!(quota["error"], "last refresh failed");
    assert!(quota["page"]["data"].get("activity").is_none());
    let activity = project(
        &snapshot,
        &bindings,
        &refreshing,
        "usage.read",
        &json!({
            "account":"future-account","section":"activity","path":"/last24Hours/summary"
        }),
    )
    .unwrap();
    assert_eq!(activity["page"]["data"], json!({"inputTokens":42}));
    assert_eq!(
        project(
            &snapshot,
            &bindings,
            &refreshing,
            "usage.read",
            &json!({"account":"future-account","section":"invalid"})
        ),
        Err("invalid_argument".into())
    );
}

#[test]
fn json_pages_bound_large_history_and_allow_discovery_without_copying_it() {
    let data = json!({"a/b": {"~key": [0,1,2,3,4]}, "history":"x".repeat(100_000)});
    assert_eq!(
        json_page(Some(&data), &json!({})),
        Err("selection_too_large".into())
    );
    let shape = json_page(Some(&data), &json!({"describe":true})).unwrap();
    assert_eq!(
        shape["data"]["history"],
        json!({"type":"string","bytes":100_000})
    );
    let first = json_page(Some(&data), &json!({"path":"/a~1b/~0key","limit":2})).unwrap();
    assert_eq!(first["data"], json!([0, 1]));
    assert_eq!(first["range"], json!({"offset":0,"next":2,"total":5}));
    let last = json_page(
        Some(&data),
        &json!({"path":"/a~1b/~0key","offset":4,"limit":2}),
    )
    .unwrap();
    assert_eq!(last["data"], json!([4]));
    assert_eq!(last["range"]["next"], Value::Null);
    let keys = json_page(
        Some(&json!({"c":3,"a":1,"b":2})),
        &json!({"offset":1,"limit":1}),
    )
    .unwrap();
    assert_eq!(keys["data"], json!({"b":2}));
    assert_eq!(keys["range"]["next"], 2);
    assert_eq!(
        json_page(Some(&data), &json!({"path":"invalid"})),
        Err("not_found".into())
    );
    for args in [
        json!({"path":"/~2"}),
        json!({"limit":0}),
        json!({"limit":201}),
        json!({"offset":-1}),
        json!({"describe":"true"}),
    ] {
        assert!(
            json_page(Some(&data), &args)
                .unwrap_err()
                .starts_with("invalid_argument")
        );
    }
    assert_eq!(
        json_page(Some(&data), &json!({"path":"/missing"})),
        Err("not_found".into())
    );
    assert_eq!(
        json_page(Some(&data), &json!({"offset":3})),
        Err("invalid_cursor".into())
    );
}

#[test]
fn account_reads_require_a_product_identity_when_product_auth_is_enabled() {
    assert!(!account_access(true, None));
    assert!(account_access(true, Some("active-user")));
    assert!(account_access(false, None));
}
