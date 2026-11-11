/** The address a pasted text names, when it is exactly one web link: an
 * `http`/`https` URL on a single line, surrounding whitespace ignored. Only
 * such a text is offered "Open as address" in the paste sheet. */
export function pastedAddress(text: string): string | undefined {
  const candidate = text.trim();
  if (!candidate || /\s/.test(candidate)) return undefined;
  try {
    const url = new URL(candidate);
    return url.protocol === "https:" || url.protocol === "http:" ? candidate : undefined;
  } catch {
    return undefined;
  }
}
