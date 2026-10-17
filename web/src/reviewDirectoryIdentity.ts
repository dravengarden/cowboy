/** Stable code-scope identities, never paths or permission grants. */
export function validReviewDirectory(value: unknown): value is string {
  if (
    typeof value !== "string" ||
    new TextEncoder().encode(value).length > 1024 || /\p{Cc}/u.test(value)
  ) return false;
  if (value === "project") return true;
  const parts = value.split("::");
  return parts.every(Boolean) &&
    ((parts[0] === "workspace" && parts.length === 3) ||
      (parts[0] === "session" && parts.length === 2));
}
