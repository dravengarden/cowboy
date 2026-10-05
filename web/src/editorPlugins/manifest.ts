/** Installable editor plugin manifest and package format (docs/editor-plugins.md).
 *
 * A package is one JSON file: the manifest, the plugin's single JavaScript
 * entry and a SHA-256 digest over both. Validation is closed: unknown fields,
 * permissions, setting types or icons are rejected rather than ignored, so a
 * newer package can never silently gain authority on an older host. */

/** Host API major. A plugin declares the major it targets and the lowest minor
 * it needs; a host accepts the same major with an equal or newer minor. */
export const EDITOR_PLUGIN_API = { major: 1, minor: 0 } as const;
export const EDITOR_PLUGIN_PACKAGE_FORMAT = "cowboy-editor-plugin/1";

export const EDITOR_PLUGIN_PERMISSIONS = [
  /** Read the full text and selection of the editor a command runs in. */
  "editor:read",
  /** Replace the selection of that editor as one undoable edit. */
  "editor:write",
] as const;
export type EditorPluginPermission = typeof EDITOR_PLUGIN_PERMISSIONS[number];

/** Closed icon vocabulary; packages never ship SVG, CSS or HTML. */
export const EDITOR_PLUGIN_ICONS = [
  "text",
  "sort",
  "list",
  "wand",
  "clock",
  "calc",
  "tag",
  "link",
] as const;
export type EditorPluginIcon = typeof EDITOR_PLUGIN_ICONS[number];

export type EditorPluginSetting =
  | {
    readonly id: string;
    readonly type: "boolean";
    readonly title: string;
    readonly description?: string;
    readonly default: boolean;
  }
  | {
    readonly id: string;
    readonly type: "string";
    readonly title: string;
    readonly description?: string;
    readonly default: string;
    readonly maxLength: number;
  }
  | {
    readonly id: string;
    readonly type: "number";
    readonly title: string;
    readonly description?: string;
    readonly default: number;
    readonly min: number;
    readonly max: number;
  }
  | {
    readonly id: string;
    readonly type: "select";
    readonly title: string;
    readonly description?: string;
    readonly default: string;
    readonly options: readonly {
      readonly value: string;
      readonly label: string;
    }[];
  };

export interface EditorPluginManifest {
  readonly id: string;
  readonly name: string;
  readonly version: string;
  readonly description: string;
  readonly author: string;
  readonly api: { readonly major: number; readonly minor: number };
  readonly permissions: readonly EditorPluginPermission[];
  readonly contexts: readonly ("document" | "session")[];
  readonly surfaces: readonly ("desktop" | "touch")[];
  readonly settings: readonly EditorPluginSetting[];
}

export interface EditorPluginPackage {
  readonly format: typeof EDITOR_PLUGIN_PACKAGE_FORMAT;
  readonly manifest: EditorPluginManifest;
  readonly main: string;
  /** `sha256-<hex>` of canonicalManifest + "\n" + main. */
  readonly digest: string;
}

export const EDITOR_PLUGIN_LIMITS = {
  packageBytes: 512 * 1024,
  mainBytes: 384 * 1024,
  settings: 32,
  options: 32,
  dataBytes: 64 * 1024,
  installed: 32,
} as const;

const ID = /^[a-z][a-z0-9-]{1,47}$/;
const SETTING_ID = /^[a-z][a-zA-Z0-9_-]{0,47}$/;
const VERSION = /^(0|[1-9]\d{0,5})\.(0|[1-9]\d{0,5})\.(0|[1-9]\d{0,5})$/;

export class EditorPluginPackageError extends Error {}

function fail(message: string): never {
  throw new EditorPluginPackageError(message);
}

function record(value: unknown, where: string): Record<string, unknown> {
  if (
    typeof value !== "object" || value === null || Array.isArray(value) ||
    Object.getPrototypeOf(value) !== Object.prototype
  ) fail(`${where} must be an object`);
  return value as Record<string, unknown>;
}

function only(
  value: Record<string, unknown>,
  keys: readonly string[],
  where: string,
): void {
  for (const key of Object.keys(value)) {
    if (!keys.includes(key)) fail(`${where} has unknown field "${key}"`);
  }
}

function text(value: unknown, where: string, max: number, min = 1): string {
  if (typeof value !== "string" || value.length < min || value.length > max) {
    fail(`${where} must be text of ${min}–${max} characters`);
  }
  return value;
}

function closedList<T extends string>(
  value: unknown,
  allowed: readonly T[],
  where: string,
  min = 1,
): readonly T[] {
  if (!Array.isArray(value) || value.length < min) {
    fail(`${where} must be a list`);
  }
  const seen = new Set<string>();
  for (const item of value) {
    if (typeof item !== "string" || !allowed.includes(item as T)) {
      fail(`${where} contains unsupported "${String(item)}"`);
    }
    if (seen.has(item)) fail(`${where} repeats "${item}"`);
    seen.add(item);
  }
  return value as T[];
}

function setting(value: unknown, index: number): EditorPluginSetting {
  const where = `settings[${index}]`;
  const s = record(value, where);
  const id = text(s.id, `${where}.id`, 48);
  if (!SETTING_ID.test(id)) fail(`${where}.id is invalid`);
  const title = text(s.title, `${where}.title`, 80);
  const description = s.description === undefined
    ? undefined
    : text(s.description, `${where}.description`, 300);
  const base = {
    id,
    title,
    ...(description === undefined ? {} : { description }),
  };
  switch (s.type) {
    case "boolean":
      only(s, ["id", "type", "title", "description", "default"], where);
      if (typeof s.default !== "boolean") {
        fail(`${where}.default must be a boolean`);
      }
      return { ...base, type: "boolean", default: s.default };
    case "string": {
      only(
        s,
        ["id", "type", "title", "description", "default", "maxLength"],
        where,
      );
      if (
        typeof s.maxLength !== "number" || !Number.isInteger(s.maxLength) ||
        s.maxLength < 1 || s.maxLength > 4096
      ) fail(`${where}.maxLength must be 1–4096`);
      return {
        ...base,
        type: "string",
        default: text(s.default, `${where}.default`, s.maxLength, 0),
        maxLength: s.maxLength,
      };
    }
    case "number": {
      only(
        s,
        ["id", "type", "title", "description", "default", "min", "max"],
        where,
      );
      const numbers = [s.min, s.max, s.default];
      if (!numbers.every((n) => typeof n === "number" && Number.isFinite(n))) {
        fail(`${where} needs finite min, max and default`);
      }
      const [min, max, fallback] = numbers as number[];
      if (min! > max! || fallback! < min! || fallback! > max!) {
        fail(`${where}.default must be within min and max`);
      }
      return {
        ...base,
        type: "number",
        default: fallback!,
        min: min!,
        max: max!,
      };
    }
    case "select": {
      only(
        s,
        ["id", "type", "title", "description", "default", "options"],
        where,
      );
      if (
        !Array.isArray(s.options) || s.options.length === 0 ||
        s.options.length > EDITOR_PLUGIN_LIMITS.options
      ) {
        fail(
          `${where}.options must list 1–${EDITOR_PLUGIN_LIMITS.options} choices`,
        );
      }
      const options = s.options.map((option, i) => {
        const o = record(option, `${where}.options[${i}]`);
        only(o, ["value", "label"], `${where}.options[${i}]`);
        return {
          value: text(o.value, `${where}.options[${i}].value`, 80),
          label: text(o.label, `${where}.options[${i}].label`, 80),
        };
      });
      if (new Set(options.map((o) => o.value)).size !== options.length) {
        fail(`${where}.options repeat a value`);
      }
      if (!options.some((o) => o.value === s.default)) {
        fail(`${where}.default must be one of its options`);
      }
      return { ...base, type: "select", default: s.default as string, options };
    }
    default:
      return fail(`${where}.type is unsupported`);
  }
}

export function parseEditorPluginManifest(
  value: unknown,
): EditorPluginManifest {
  const m = record(value, "manifest");
  only(m, [
    "id",
    "name",
    "version",
    "description",
    "author",
    "api",
    "permissions",
    "contexts",
    "surfaces",
    "settings",
  ], "manifest");
  const id = text(m.id, "id", 48);
  if (!ID.test(id)) fail("id must be lowercase letters, digits and dashes");
  if (id.startsWith("cowboy-")) fail('id prefix "cowboy-" is reserved');
  const version = text(m.version, "version", 20);
  if (!VERSION.test(version)) fail("version must be MAJOR.MINOR.PATCH");
  const api = record(m.api, "api");
  only(api, ["major", "minor"], "api");
  if (
    !Number.isInteger(api.major) || !Number.isInteger(api.minor) ||
    (api.major as number) < 0 || (api.minor as number) < 0
  ) fail("api must declare integer major and minor");
  const settings = m.settings === undefined ? [] : m.settings;
  if (
    !Array.isArray(settings) || settings.length > EDITOR_PLUGIN_LIMITS.settings
  ) {
    fail(`settings must list at most ${EDITOR_PLUGIN_LIMITS.settings} entries`);
  }
  const parsedSettings = settings.map(setting);
  if (new Set(parsedSettings.map((s) => s.id)).size !== parsedSettings.length) {
    fail("settings repeat an id");
  }
  return {
    id,
    name: text(m.name, "name", 60),
    version,
    description: text(m.description, "description", 300),
    author: text(m.author, "author", 80),
    api: { major: api.major as number, minor: api.minor as number },
    permissions: closedList(
      m.permissions,
      EDITOR_PLUGIN_PERMISSIONS,
      "permissions",
      0,
    ),
    contexts: closedList(
      m.contexts,
      ["document", "session"] as const,
      "contexts",
    ),
    surfaces: closedList(m.surfaces, ["desktop", "touch"] as const, "surfaces"),
    settings: parsedSettings,
  };
}

/** Why this host cannot run the manifest, or null when it can. */
export function editorPluginIncompatibility(
  manifest: EditorPluginManifest,
): string | null {
  if (manifest.api.major !== EDITOR_PLUGIN_API.major) {
    return `needs editor plugin API ${manifest.api.major}.x; this Cowboy provides ${EDITOR_PLUGIN_API.major}.${EDITOR_PLUGIN_API.minor}`;
  }
  if (manifest.api.minor > EDITOR_PLUGIN_API.minor) {
    return `needs editor plugin API ${manifest.api.major}.${manifest.api.minor}; update Cowboy first`;
  }
  return null;
}

/** Stable key order, so the same manifest always hashes identically. */
export function canonicalManifest(manifest: EditorPluginManifest): string {
  const sort = (value: unknown): unknown =>
    Array.isArray(value)
      ? value.map(sort)
      : value && typeof value === "object"
      ? Object.fromEntries(
        Object.keys(value).sort().map((
          key,
        ) => [key, sort((value as Record<string, unknown>)[key])]),
      )
      : value;
  return JSON.stringify(sort(manifest));
}

export async function editorPluginDigest(
  manifest: EditorPluginManifest,
  main: string,
): Promise<string> {
  const bytes = new TextEncoder().encode(
    `${canonicalManifest(manifest)}\n${main}`,
  );
  const hash = new Uint8Array(await crypto.subtle.digest("SHA-256", bytes));
  return `sha256-${
    Array.from(hash, (b) => b.toString(16).padStart(2, "0")).join("")
  }`;
}

/** Parse and verify one package file's text. Integrity only: an installed
 * package is trusted because the user chose to install it, as with an
 * Obsidian manual install, and it runs without network or App access. */
export async function parseEditorPluginPackage(
  source: string,
): Promise<EditorPluginPackage> {
  if (
    new TextEncoder().encode(source).length > EDITOR_PLUGIN_LIMITS.packageBytes
  ) {
    fail("package is larger than 512 KiB");
  }
  let json: unknown;
  try {
    json = JSON.parse(source);
  } catch {
    fail("package is not JSON");
  }
  const p = record(json, "package");
  only(p, ["format", "manifest", "main", "digest"], "package");
  if (p.format !== EDITOR_PLUGIN_PACKAGE_FORMAT) {
    fail(`package format must be ${EDITOR_PLUGIN_PACKAGE_FORMAT}`);
  }
  const manifest = parseEditorPluginManifest(p.manifest);
  const main = text(p.main, "main", EDITOR_PLUGIN_LIMITS.mainBytes);
  const digest = text(p.digest, "digest", 80);
  if (digest !== await editorPluginDigest(manifest, main)) {
    fail("package digest does not match its contents");
  }
  return { format: EDITOR_PLUGIN_PACKAGE_FORMAT, manifest, main, digest };
}

export function compareEditorPluginVersions(a: string, b: string): number {
  const pa = a.split(".").map(Number);
  const pb = b.split(".").map(Number);
  for (let i = 0; i < 3; i++) {
    if (pa[i] !== pb[i]) return pa[i]! < pb[i]! ? -1 : 1;
  }
  return 0;
}

export type EditorPluginSettingValues = Readonly<
  Record<string, boolean | string | number>
>;

/** Stored values filtered and clamped to the current manifest's schema;
 * an upgrade keeps compatible values and resets the rest to defaults. */
export function normalizeEditorPluginSettings(
  manifest: EditorPluginManifest,
  stored: unknown,
): EditorPluginSettingValues {
  const values = stored && typeof stored === "object" && !Array.isArray(stored)
    ? stored as Record<string, unknown>
    : {};
  const out: Record<string, boolean | string | number> = {};
  for (const s of manifest.settings) {
    const v = values[s.id];
    switch (s.type) {
      case "boolean":
        out[s.id] = typeof v === "boolean" ? v : s.default;
        break;
      case "string":
        out[s.id] = typeof v === "string" && v.length <= s.maxLength
          ? v
          : s.default;
        break;
      case "number":
        out[s.id] = typeof v === "number" && Number.isFinite(v) && v >= s.min &&
            v <= s.max
          ? v
          : s.default;
        break;
      case "select":
        out[s.id] = typeof v === "string" && s.options.some((o) =>
            o.value === v
          )
          ? v
          : s.default;
        break;
    }
  }
  return out;
}
