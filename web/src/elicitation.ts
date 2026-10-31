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

function fieldOf(
  id: string,
  schema: Record<string, unknown>,
  required: boolean,
): ElicitationField | null {
  const meta = record(schema["_meta"]);
  const codex = record(meta?.["codex"]);
  const base = {
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
    case "string": {
      const choices = choicesOf(schema["oneOf"], schema["enum"]);
      const fallback = typeof schema["default"] === "string" ? schema["default"] : undefined;
      return {
        ...base,
        kind: choices.length > 0 ? "single" : "text",
        choices,
        defaultValue: fallback,
      };
    }
    case "array": {
      const items = record(schema["items"]);
      const choices = choicesOf(items?.["anyOf"] ?? items?.["oneOf"], items?.["enum"]);
      if (choices.length === 0) return null;
      const fallback = Array.isArray(schema["default"])
        ? schema["default"].filter((value): value is string => typeof value === "string")
        : undefined;
      return { ...base, kind: "multi", choices, defaultValue: fallback };
    }
    case "boolean":
      return {
        ...base,
        kind: "boolean",
        choices: [],
        defaultValue: typeof schema["default"] === "boolean" ? schema["default"] : undefined,
      };
    case "number":
    case "integer":
      return {
        ...base,
        kind: schema["type"],
        choices: [],
        defaultValue: typeof schema["default"] === "number"
          ? String(schema["default"])
          : undefined,
      };
    default:
      return null;
  }
}

/** The elicitation a permission request carries, if it is one. */
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
  const properties = record(schema?.["properties"]) ?? {};
  const required = new Set(
    Array.isArray(schema?.["required"])
      ? schema["required"].filter((id): id is string => typeof id === "string")
      : [],
  );
  const fields: ElicitationField[] = [];
  for (const [id, property] of Object.entries(properties)) {
    const field = record(property);
    if (!field) continue;
    const parsed = fieldOf(id, field, required.has(id));
    // A required field Cowboy cannot render makes the form unanswerable here.
    if (!parsed) {
      if (required.has(id)) return null;
      continue;
    }
    fields.push(parsed);
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
    if (field.defaultValue !== undefined) values[field.id] = field.defaultValue;
    else if (field.kind === "multi") values[field.id] = [];
    else if (field.kind === "boolean") values[field.id] = false;
    else values[field.id] = "";
  }
  return values;
}

function answered(field: ElicitationField, value: ElicitationValue | undefined): boolean {
  if (field.kind === "boolean") return typeof value === "boolean";
  if (Array.isArray(value)) return value.length > 0;
  if (typeof value !== "string" || value.trim() === "") return false;
  if (field.kind === "number") return Number.isFinite(Number(value));
  if (field.kind === "integer") return Number.isInteger(Number(value));
  return true;
}

/** Whether every required field has a valid answer and no optional number is
 *  malformed. */
export function elicitationComplete(
  fields: readonly ElicitationField[],
  values: ElicitationValues,
): boolean {
  return fields.every((field) => {
    const value = values[field.id];
    if (field.required) return answered(field, value);
    if ((field.kind === "number" || field.kind === "integer") &&
      typeof value === "string" && value.trim() !== "") {
      return answered(field, value);
    }
    return true;
  });
}

/** The option id that accepts the form with these values. Empty optional
 *  answers are omitted, as the schema allows. */
export function elicitationAnswer(
  fields: readonly ElicitationField[],
  values: ElicitationValues,
): string {
  const content: Record<string, string | number | boolean | string[]> = {};
  for (const field of fields) {
    const value = values[field.id];
    if (!answered(field, value)) continue;
    if (field.kind === "number" || field.kind === "integer") {
      content[field.id] = Number(value);
    } else if (typeof value === "string") {
      content[field.id] = field.kind === "text" ? value : value.trim();
    } else if (value !== undefined) {
      content[field.id] = value;
    }
  }
  return Object.keys(content).length === 0 && fields.length === 0
    ? ELICITATION_ACCEPT
    : `${ELICITATION_ACCEPT}:${JSON.stringify(content)}`;
}
