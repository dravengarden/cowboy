//! The usage accounts the first-party host Plugins declare, for tests.

use std::sync::OnceLock;

use super::adapter_hosts;

#[must_use]
pub fn usage_accounts() -> Vec<&'static str> {
    usage_account_rows()
        .iter()
        .map(|(_, account)| *account)
        .collect()
}

fn usage_account_rows() -> &'static Vec<(u16, &'static str)> {
    static VALUE: OnceLock<Vec<(u16, &'static str)>> = OnceLock::new();
    VALUE.get_or_init(|| {
        let mut rows = Vec::new();
        for (plugin_id, source) in adapter_hosts() {
            let value: serde_json::Value = serde_json::from_str(source)
                .unwrap_or_else(|error| panic!("{plugin_id} host.json must parse: {error}"));
            let Some(usage) = value.get("usage") else {
                continue;
            };
            let Some(account) = usage.get("account").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let order = usage
                .get("order")
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| u16::try_from(value).ok())
                .unwrap_or(u16::MAX);
            let leaked: &'static str = Box::leak(account.to_owned().into_boxed_str());
            if !rows.iter().any(|(_, existing)| *existing == leaked) {
                rows.push((order, leaked));
            }
        }
        rows.sort_by_key(|(order, account)| (*order, *account));
        rows
    })
}
