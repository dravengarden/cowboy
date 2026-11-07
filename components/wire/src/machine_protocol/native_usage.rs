/// Closed diagnostics for native observations; arbitrary subprocess output is
/// never an account API error or a log field.
pub fn native_usage_error_code(value: &str) -> &'static str {
    match value {
        "native_usage_busy" => "native_usage_busy",
        "native_usage_admission_expired" => "native_usage_admission_expired",
        "native_usage_profile_changed" => "native_usage_profile_changed",
        "native_usage_collector_unavailable" => "native_usage_collector_unavailable",
        "native_usage_home_unsupported" => "native_usage_home_unsupported",
        "native_usage_installation_changed" => "native_usage_installation_changed",
        "native_usage_authentication_required" => "native_usage_authentication_required",
        "native_usage_collector_failed" => "native_usage_collector_failed",
        "machine_usage_update_required" => "machine_usage_update_required",
        "machine_connection_changed" => "machine_connection_changed",
        _ => "native_usage_unavailable",
    }
}
