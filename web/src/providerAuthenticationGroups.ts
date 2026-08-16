export interface AuthenticationScoped {
  authentication_scope: string;
}

export interface ProviderAuthenticationGroup<T extends AuthenticationScoped> {
  authenticationScope: string;
  entries: [T, ...T[]];
}

/** Group Service credentials by their signed portable schema boundary.
 * Adding another Provider to a credential family only requires declaring the
 * same authentication_scope in its package; Cowboy UI does not need a
 * Provider-id-specific branch. */
export function groupProviderAuthentications<T extends AuthenticationScoped>(
  entries: readonly T[],
): ProviderAuthenticationGroup<T>[] {
  const groups = new Map<string, T[]>();
  for (const entry of entries) {
    const group = groups.get(entry.authentication_scope);
    if (group) group.push(entry);
    else groups.set(entry.authentication_scope, [entry]);
  }
  return [...groups].map(([authenticationScope, groupedEntries]) => {
    const [first, ...rest] = groupedEntries;
    if (!first) throw new Error("authentication group cannot be empty");
    return { authenticationScope, entries: [first, ...rest] };
  });
}
