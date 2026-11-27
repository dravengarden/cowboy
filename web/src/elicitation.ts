// ACP elicitations (`elicitation/create`) reach the Web as permission requests:
// the worker presents them through the permission channel every surface
// already answers, carrying the original request under
// `toolCall._cowboy.elicitation` (src/acp.rs `elicitation_permission`). This
// module turns that payload into form fields and the answer back into the
// option id the worker understands: `accept:<content JSON>`, `accept` (a URL
// elicitation or an empty form) or `decline`.

export interface ElicitationChoice {
  value: string;
  title: string;
  description: string | null;
  /** Plain-text preview of the option (a mockup, a code snippet), shown on
   *  request. Never markup. */
  preview: string | null;
}

export type ElicitationFieldKind =
  | "single"
  | "multi"
  | "text"
  | "boolean"
  | "number"
  | "integer";

export interface ElicitationField {
  id: string;
  kind: ElicitationFieldKind;
  title: string;
  description: string | null;
  required: boolean;
  choices: ElicitationChoice[];
  /** A credential or other value that must not be shown while typed. */
  secret: boolean;
  /** A free-text note that only matters when its question picked "Other". */
  noteFor: string | null;
  defaultValue: ElicitationValue | undefined;
  minLength?: number | undefined;
  maxLength?: number | undefined;
  minimum?: number | undefined;
  maximum?: number | undefined;
  minItems?: number | undefined;
  maxItems?: number | undefined;
}

export type ElicitationValue = string | boolean | string[];

export type Elicitation =
  | {
    mode: "form";
    message: string;
    fields: ElicitationField[];
    /** The agent answers on its own after this long without input (Codex
     *  `request_user_input`); null when it waits for the user. */
    autoResolutionMs: number | null;
  }
  | { mode: "url"; message: string; url: string };

export type ElicitationValues = Record<string, ElicitationValue>;

export const ELICITATION_ACCEPT = "accept";
/** claude-agent-acp's `_meta` key for an AskUserQuestion option's extras
 *  (dist/elicitation.js `OPTION_META_KEY`). */
export const CLAUDE_OPTION_META_KEY = "_claude/askUserQuestionOption";
/** A preview is plain text for a bounded box; keep a runaway one bounded too. */
const PREVIEW_MAX_CHARS = 20_000;
export const ELICITATION_DECLINE = "decline";

function record(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

function text(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function previewOf(option: Record<string, unknown> | null): string | null {
  const preview = record(record(option?.["_meta"])?.[CLAUDE_OPTION_META_KEY])?.["preview"];
  if (typeof preview !== "string" || !preview.trim()) return null;
  return preview.length > PREVIEW_MAX_CHARS
    ? `${preview.slice(0, PREVIEW_MAX_CHARS)}\n…`
    : preview;
}

function choicesOf(source: unknown, values: unknown): ElicitationChoice[] {
  if (Array.isArray(source)) {
    return source.flatMap((option) => {
      const entry = record(option);
      const value = entry?.["const"];
      if (typeof value !== "string") return [];
      return [{
        value,
        title: text(entry?.["title"]) ?? value,
        description: text(entry?.["description"]),
        preview: previewOf(entry),
      }];
    });
  }
  return Array.isArray(values)
    ? values.filter((value): value is string => typeof value === "string")
      .map((value) => ({ value, title: value, description: null, preview: null }))
    : [];
}

/** Codex `request_user_input`: `_meta.codex.autoResolutionMs`, a number of
 *  milliseconds or null. */
function autoResolutionOf(request: Record<string, unknown>): number | null {
  const value = record(record(request["_meta"])?.["codex"])?.["autoResolutionMs"];
  return typeof value === "number" && Number.isFinite(value) && value >= 0 ? value : null;
}

function codePointLength(value: string): number {
  let count = 0;
  for (const _ of value) count += 1;
  return count;
}

/** Whether `toolCall` carries a Cowboy elicitation payload (valid or invalid). */
export function hasElicitationPayload(toolCall: unknown): boolean {
  return record(record(record(toolCall)?.["_cowboy"])?.["elicitation"]) !== null;
}

/** Parse an optional numeric bound; `null` indicates a malformed declared value. */
function parseBound(value: unknown, integer: boolean, min = -Infinity): number | undefined | null {
  if (value === undefined) return undefined;
  if (typeof value !== "number" || !Number.isFinite(value) || value < min) return null;
  if (integer && !Number.isInteger(value)) return null;
  return value;
}

function lengthWithin(
  value: string,
  minLength: number | undefined,
  maxLength: number | undefined,
): boolean {
  const len = codePointLength(value);
  return (minLength === undefined || len >= minLength) &&
    (maxLength === undefined || len <= maxLength);
}

function stringFitsBounds(field: ElicitationField, value: string): boolean {
  const len = codePointLength(value);
  if (field.minLength !== undefined && len < field.minLength) return false;
  if (field.maxLength !== undefined && len > field.maxLength) return false;
  return true;
}

function defineOwn(target: Record<string, unknown>, key: string, value: unknown): void {
  Object.defineProperty(target, key, {
    value,
    enumerable: true,
    writable: true,
    configurable: true,
  });
}

type FieldParseResult =
  | { status: "ok"; field: ElicitationField }
  | { status: "omit" }
  | { status: "invalid" };

interface FieldBase {
  id: string;
  title: string;
  description: string | null;
  required: boolean;
  secret: boolean;
  noteFor: string | null;
}

function stringFieldOf(
  base: FieldBase,
  schema: Record<string, unknown>,
  required: boolean,
): FieldParseResult {
  const minLength = parseBound(schema["minLength"], true, 0);
  const maxLength = parseBound(schema["maxLength"], true, 0);
  if (minLength === null || maxLength === null) return { status: "invalid" };
  if (minLength !== undefined && maxLength !== undefined && minLength > maxLength) {
    return { status: "invalid" };
  }
  const rawDefault = typeof schema["default"] === "string" ? schema["default"] : undefined;
  const hasChoices = schema["oneOf"] !== undefined || schema["enum"] !== undefined;
  if (hasChoices) {
    const rawChoices = choicesOf(schema["oneOf"], schema["enum"]);
    if (rawChoices.length === 0) return { status: "invalid" };
    // Filter out choices that violate declared length bounds so an optional
    // choice can never trap the user with a permanently disabled Submit.
    const validChoices = rawChoices.filter((c) => lengthWithin(c.value, minLength, maxLength));
    if (validChoices.length === 0) return required ? { status: "invalid" } : { status: "omit" };
    const fallback = rawDefault !== undefined && validChoices.some((c) => c.value === rawDefault)
      ? rawDefault
      : undefined;
    return {
      status: "ok",
      field: {
        ...base,
        kind: "single",
        choices: validChoices,
        defaultValue: fallback,
        ...(minLength !== undefined ? { minLength } : {}),
        ...(maxLength !== undefined ? { maxLength } : {}),
      },
    };
  }
  if (required && maxLength === 0) return { status: "invalid" };
  const fallback = rawDefault !== undefined && lengthWithin(rawDefault, minLength, maxLength)
    ? rawDefault
    : undefined;
  return {
    status: "ok",
    field: {
      ...base,
      kind: "text",
      choices: [],
      defaultValue: fallback,
      ...(minLength !== undefined ? { minLength } : {}),
      ...(maxLength !== undefined ? { maxLength } : {}),
    },
  };
}

function arrayFieldOf(
  base: FieldBase,
  schema: Record<string, unknown>,
  required: boolean,
): FieldParseResult {
  const items = record(schema["items"]);
  const choices = choicesOf(items?.["anyOf"] ?? items?.["oneOf"], items?.["enum"]);
  if (choices.length === 0) return { status: "invalid" };
  const minItems = parseBound(schema["minItems"], true, 0);
  const maxItems = parseBound(schema["maxItems"], true, 0);
  if (minItems === null || maxItems === null) return { status: "invalid" };
  if (minItems !== undefined && maxItems !== undefined && minItems > maxItems) {
    return { status: "invalid" };
  }
  const distinct = new Set(choices.map((c) => c.value));
  if (minItems !== undefined && minItems > distinct.size) return { status: "invalid" };
  if (required && maxItems !== undefined && maxItems < Math.max(1, minItems ?? 0)) {
    return { status: "invalid" };
  }
  const rawDefault = Array.isArray(schema["default"])
    ? schema["default"].filter((v): v is string => typeof v === "string" && distinct.has(v))
    : undefined;
  const uniqueDefault = rawDefault ? [...new Set(rawDefault)] : undefined;
  const fallback = uniqueDefault !== undefined &&
      (minItems === undefined || uniqueDefault.length >= minItems) &&
      (maxItems === undefined || uniqueDefault.length <= maxItems)
    ? uniqueDefault
    : undefined;
  return {
    status: "ok",
    field: {
      ...base,
      kind: "multi",
      choices,
      defaultValue: fallback,
      ...(minItems !== undefined ? { minItems } : {}),
      ...(maxItems !== undefined ? { maxItems } : {}),
    },
  };
}

function numberFieldOf(
  base: FieldBase,
  kind: "number" | "integer",
  schema: Record<string, unknown>,
): FieldParseResult {
  const minimum = parseBound(schema["minimum"], false);
  const maximum = parseBound(schema["maximum"], false);
  if (minimum === null || maximum === null) return { status: "invalid" };
  if (minimum !== undefined && maximum !== undefined && minimum > maximum) {
    return { status: "invalid" };
  }
  if (
    kind === "integer" &&
    minimum !== undefined &&
    maximum !== undefined &&
    Math.ceil(minimum) > Math.floor(maximum)
  ) {
    return { status: "invalid" };
  }
  const rawDefault = typeof schema["default"] === "number" &&
      Number.isFinite(schema["default"]) &&
      (kind !== "integer" || Number.isInteger(schema["default"])) &&
      (minimum === undefined || schema["default"] >= minimum) &&
      (maximum === undefined || schema["default"] <= maximum)
    ? String(schema["default"])
    : undefined;
  return {
    status: "ok",
    field: {
      ...base,
      kind,
      choices: [],
      defaultValue: rawDefault,
      ...(minimum !== undefined ? { minimum } : {}),
      ...(maximum !== undefined ? { maximum } : {}),
    },
  };
}

function fieldOf(
  id: string,
  schema: Record<string, unknown>,
  required: boolean,
): FieldParseResult {
  const meta = record(schema["_meta"]);
  const codex = record(meta?.["codex"]);
  const base: FieldBase = {
    id,
    title: text(schema["title"]) ?? id,
    description: text(schema["description"]),
    required,
    secret: schema["format"] === "password" || codex?.["isSecret"] === true,
    noteFor: codex?.["role"] === "user_note" && typeof codex["questionId"] === "string"
      ? codex["questionId"]
      : null,
  };
  switch (schema["type"]) {
    case "string":
      return stringFieldOf(base, schema, required);
    case "array":
      return arrayFieldOf(base, schema, required);
    case "boolean":
      return {
        status: "ok",
        field: {
          ...base,
          kind: "boolean",
          choices: [],
          defaultValue: typeof schema["default"] === "boolean" ? schema["default"] : undefined,
        },
      };
    case "number":
    case "integer":
      return numberFieldOf(base, schema["type"], schema);
    default:
      return required ? { status: "invalid" } : { status: "omit" };
  }
}

/** The elicitation a permission request carries, if it is one and is valid. */
export function elicitationOf(toolCall: unknown): Elicitation | null {
  const request = record(record(record(toolCall)?.["_cowboy"])?.["elicitation"]);
  if (!request) return null;
  const message = text(request["message"]) ?? "The agent needs your input.";
  if (request["mode"] === "url") {
    const url = text(request["url"]);
    return url && /^https?:\/\//i.test(url) ? { mode: "url", message, url } : null;
  }
  if (request["mode"] !== "form") return null;
  const schema = record(request["requestedSchema"]);
  if (!schema) return null;
  if (schema["type"] !== undefined && schema["type"] !== "object") return null;
  const properties = record(schema["properties"]);
  if (!properties) return null;
  const required = new Set<string>();
  if (schema["required"] !== undefined) {
    if (!Array.isArray(schema["required"])) return null;
    for (const id of schema["required"]) {
      if (typeof id !== "string" || id.length === 0) return null;
      required.add(id);
    }
  }
  const fields: ElicitationField[] = [];
  const fieldIds = new Set<string>();
  for (const [id, property] of Object.entries(properties)) {
    const field = record(property);
    if (!field) return null;
    const parsed = fieldOf(id, field, required.has(id));
    if (parsed.status === "invalid") return null;
    if (parsed.status === "omit") continue;
    fields.push(parsed.field);
    fieldIds.add(id);
  }
  for (const id of required) {
    if (!fieldIds.has(id)) return null;
  }
  return { mode: "form", message, fields, autoResolutionMs: autoResolutionOf(request) };
}

/** Remaining whole seconds before an auto-resolving request answers itself,
 *  given when this client first saw it; 0 once due. */
export function autoResolutionSecondsLeft(
  autoResolutionMs: number,
  firstSeenAtMs: number,
  nowMs: number,
): number {
  return Math.max(0, Math.ceil((firstSeenAtMs + autoResolutionMs - nowMs) / 1000));
}

/** "42s" / "1:05" for the countdown. */
export function formatCountdown(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  return `${minutes}:${String(seconds % 60).padStart(2, "0")}`;
}

export function initialElicitationValues(
  fields: readonly ElicitationField[],
): ElicitationValues {
  const values: ElicitationValues = {};
  for (const field of fields) {
    if (field.defaultValue !== undefined) defineOwn(values, field.id, field.defaultValue);
    else if (field.kind === "single") continue;
    else if (field.kind === "multi") defineOwn(values, field.id, []);
    else if (field.kind === "boolean") defineOwn(values, field.id, false);
    else defineOwn(values, field.id, "");
  }
  return values;
}

function ownValue(values: ElicitationValues, id: string): ElicitationValue | undefined {
  return Object.hasOwn(values, id) ? values[id] : undefined;
}

function answered(field: ElicitationField, value: ElicitationValue | undefined): boolean {
  if (field.kind === "boolean") return typeof value === "boolean";
  if (field.kind === "multi") {
    if (!Array.isArray(value) || value.length === 0) return false;
    if (field.minItems !== undefined && value.length < field.minItems) return false;
    if (field.maxItems !== undefined && value.length > field.maxItems) return false;
    const allowed = new Set(field.choices.map((c) => c.value));
    const seen = new Set<string>();
    for (const item of value) {
      if (typeof item !== "string" || !allowed.has(item) || seen.has(item)) return false;
      seen.add(item);
    }
    return true;
  }
  if (field.kind === "single") {
    if (typeof value !== "string") return false;
    if (!field.choices.some((c) => c.value === value)) return false;
    return stringFitsBounds(field, value);
  }
  if (typeof value !== "string" || value.trim() === "") return false;
  if (field.kind === "number" || field.kind === "integer") {
    const n = Number(value);
    if (!Number.isFinite(n)) return false;
    if (field.kind === "integer" && !Number.isInteger(n)) return false;
    if (field.minimum !== undefined && n < field.minimum) return false;
    if (field.maximum !== undefined && n > field.maximum) return false;
    return true;
  }
  return stringFitsBounds(field, value);
}

/** Whether every required field has a valid answer and no provided optional
 *  answer violates its schema bounds. */
export function elicitationComplete(
  fields: readonly ElicitationField[],
  values: ElicitationValues,
): boolean {
  return fields.every((field) => {
    const value = ownValue(values, field.id);
    if (field.required) return answered(field, value);
    if (field.kind === "multi") {
      return !Array.isArray(value) || value.length === 0 || answered(field, value);
    }
    if (field.kind === "single") {
      return value === undefined || answered(field, value);
    }
    if (typeof value === "string" && value.trim() !== "") {
      return answered(field, value);
    }
    return true;
  });
}

/** The option id that accepts the form with these values. Empty optional
 *  answers are omitted, as the schema allows. Choices are serialized verbatim. */
export function elicitationAnswer(
  fields: readonly ElicitationField[],
  values: ElicitationValues,
): string {
  const content: Record<string, string | number | boolean | string[]> = {};
  for (const field of fields) {
    const value = ownValue(values, field.id);
    if (!answered(field, value)) continue;
    if (field.kind === "number" || field.kind === "integer") {
      defineOwn(content, field.id, Number(value));
    } else if (typeof value === "string") {
      defineOwn(content, field.id, value);
    } else if (value !== undefined) {
      defineOwn(content, field.id, value);
    }
  }
  return Object.keys(content).length === 0 && fields.length === 0
    ? ELICITATION_ACCEPT
    : `${ELICITATION_ACCEPT}:${JSON.stringify(content)}`;
}
