export const ADMIN_PASSWORD_MIN_LEN = 15;
export const ADMIN_PASSWORD_MAX_LEN = 128;

export type AdminPasswordLevel = "empty" | "weak" | "strong";

export interface AdminPasswordChecks {
  length: boolean;
  lower: boolean;
  upper: boolean;
  digit: boolean;
  generated: boolean;
}

export interface AdminPasswordAssessment {
  length: number;
  checks: AdminPasswordChecks;
  acceptable: boolean;
  level: AdminPasswordLevel;
  label: string;
}

export function hasRequiredClasses(password: string): boolean {
  return /[a-z]/.test(password) && /[A-Z]/.test(password) && /\d/.test(password);
}

export function looksLikePasswordManagerSecret(password: string): boolean {
  const groups = password.split("-");
  return groups.length >= 3 &&
    groups.every((group) =>
      group.length >= 3 && group.length <= 8 && /^[A-Za-z0-9]+$/.test(group)
    );
}

export function adminPasswordAcceptable(password: string, account: string): boolean {
  const length = [...password].length;
  if (length < ADMIN_PASSWORD_MIN_LEN || length > ADMIN_PASSWORD_MAX_LEN) {
    return false;
  }
  if (password === account && account !== "") return false;
  return hasRequiredClasses(password) || looksLikePasswordManagerSecret(password);
}

export function assessAdminPassword(
  password: string,
  account: string,
): AdminPasswordAssessment {
  const length = [...password].length;
  const checks: AdminPasswordChecks = {
    length: length >= ADMIN_PASSWORD_MIN_LEN,
    lower: /[a-z]/.test(password),
    upper: /[A-Z]/.test(password),
    digit: /\d/.test(password),
    generated: looksLikePasswordManagerSecret(password),
  };
  const acceptable = adminPasswordAcceptable(password, account);
  let level: AdminPasswordLevel = "empty";
  if (password.length > 0) {
    level = acceptable ? "strong" : "weak";
  }
  const label = level === "empty"
    ? "Enter a password"
    : level === "weak"
    ? "Too weak"
    : "Strong";
  return { length, checks, acceptable, level, label };
}
