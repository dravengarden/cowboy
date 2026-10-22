/** People-facing text for login readiness codes. Codes stay in the API for
 *  agents and diagnostics; unknown future codes fall back to a neutral text. */
const REASONS: Record<string, string> = {
  account_disabled: "This subscription is disabled.",
  account_unavailable: "This subscription is unavailable.",
  machine_not_permitted: "This Provider cannot run on this Machine.",
  machine_unavailable: "This Machine is offline.",
  machine_account_protocol_required:
    "This Machine needs a Cowboy update before it can hold separate subscriptions.",
  machine_recipient_unavailable:
    "This Machine is not ready to receive a sign-in. Reconnect it and try again.",
  provider_not_installed: "This Provider is not installed on this Machine.",
  release_unavailable:
    "The Provider release on this Machine is not trusted by this Service.",
  native_identity_verification_unavailable:
    "Not available for separate subscriptions on this Machine.",
  native_login_lifecycle_not_enabled:
    "Separate sign-in is not enabled for this Provider.",
};

export function loginReasonText(code: string | null | undefined): string {
  return (code && REASONS[code]) || "Sign-in is unavailable on this Machine.";
}
