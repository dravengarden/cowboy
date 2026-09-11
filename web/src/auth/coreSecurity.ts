/** Local authentication presentation belongs to Cowboy, never a Catalog host.
 * These decisions project Service policy; the server still authorizes requests.
 */
export const PASSWORD_LOGIN_METHOD = "password";

export function passwordLoginFields(): {
  account: string;
  secret: string;
  confirm: string;
  setup: string;
} {
  return {
    account: "Account",
    secret: "Password",
    confirm: "Confirm password",
    setup: "Setup code",
  };
}

export type CorePasswordMode = "setup" | "register" | "login";

export function corePasswordMode(policy: {
  setupRequired: boolean;
  setupPending: boolean;
  passwordEnabled: boolean;
  selectedMethod: string;
}): CorePasswordMode | null {
  // Creating the sole local account is a core bootstrap ceremony, even when
  // ordinary password login is disabled in favor of an external identity.
  if (policy.setupRequired) return policy.setupPending ? "register" : "setup";
  return policy.passwordEnabled &&
      policy.selectedMethod === PASSWORD_LOGIN_METHOD
    ? "login"
    : null;
}

export function corePasskeysEnabled(
  policy: { enabled: boolean } | undefined,
): boolean {
  return policy?.enabled === true;
}
