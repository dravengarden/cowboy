export function validSubscriptionLabel(value: string): boolean {
  return Boolean(value.trim()) && new TextEncoder().encode(value).length <= 128 &&
    !Array.from(value).some((character) => {
      const code = character.codePointAt(0)!;
      return code < 32 || (code >= 127 && code <= 159);
    });
}
