/** The display fields a valid contract carries. */
export interface ProviderDisplayFields {
  name: string;
  vendor: string;
  summary: string;
  accent: string;
  secondary_accent: string;
  logo_asset: string;
  icon_asset: string;
}

/** The Provider display contract: name, vendor, summary, accents, assets. */
export function validDisplay(
  display: unknown,
  checks: {
    isColor: (value: string) => boolean;
    isIdentifier: (value: string) => boolean;
  },
): display is ProviderDisplayFields {
  if (display === null || typeof display !== "object" || Array.isArray(display)) return false;
  const value = display as Record<string, unknown>;
  const text = (key: string): string | undefined =>
    typeof value[key] === "string" ? value[key] : undefined;
  const filled = (key: string) => Boolean(text(key)?.trim());
  const color = (key: string) => text(key) !== undefined && checks.isColor(text(key)!);
  const id = (key: string) => text(key) !== undefined && checks.isIdentifier(text(key)!);
  return (
    filled("name") &&
    filled("vendor") &&
    filled("summary") &&
    color("accent") &&
    color("secondary_accent") &&
    id("logo_asset") &&
    id("icon_asset")
  );
}

/**
 * Whether the levitating activity (Provider SDK 3.7.0) appears below that
 * SDK: such a package is refused, as the Rust SDK refuses it.
 */
export function levitationBelowFloor(
  surfaces: Record<string, unknown>,
  sdkVersion: string,
  compare: (left: string, right: string) => number,
): boolean {
  const walk = (node: unknown): boolean => {
    if (node === null || typeof node !== "object" || Array.isArray(node)) return false;
    const value = node as {
      component?: unknown;
      indicator?: { kind?: unknown };
      children?: unknown;
    };
    if (value.component === "activity") return value.indicator?.kind === "asset_levitate";
    return (
      value.component === "stack" && Array.isArray(value.children) && value.children.some(walk)
    );
  };
  return Object.values(surfaces).some(walk) && compare(sdkVersion, "3.7.0") < 0;
}
