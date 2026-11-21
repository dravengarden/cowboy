// Structured output for Antigravity managed calls. `agy` has no native
// schema constraint, so the proxy states the round's output schema as a
// contract appended to the prompt, holds the reply back as thought, checks
// it against the schema and asks for a corrected reply at most
// `MAX_CORRECTIONS` times inside the same turn. Only a reply that passes is
// sent as the turn's final message; otherwise none is, and the Controller
// records the round's structured output as invalid.
//
// The Controller sends Antigravity only Cowboy's own review schema. The
// check here is exact for the small keyword set that schema uses, and a
// schema with any other keyword is refused before the round starts
// (`schemaRefusal`), so no constraint is silently skipped: no references,
// combinators, `not`, `pattern` or dialect switches are evaluated here.

export const MAX_CORRECTIONS = 2;
/** Prompt blocks the proxy adds; `agy` echoes them as the owner's. */
export const CONTRACT_OPEN = "<cowboy_output_contract>";
/** The longest reply checked; a longer one is asked to be corrected. */
export const MAX_REPLY_CHARS = 1024 * 1024;
/** Nesting bound for replies and schemas. */
const MAX_DEPTH = 64;

export function contractBlock(schema) {
  return {
    type: "text",
    text: [
      CONTRACT_OPEN,
      "This is a Cowboy managed call. Your final message must be exactly one JSON value that",
      "matches the JSON Schema below: no prose, no Markdown fence, nothing before or after it.",
      JSON.stringify(schema),
      "</cowboy_output_contract>",
    ].join("\n"),
  };
}

/** The follow-up when a reply does not match the schema. */
export function correctionBlock(schema, problem, nonce) {
  return {
    type: "text",
    text: [
      CONTRACT_OPEN,
      `Your last reply is not valid for the required output (${problem}). Reply again: first`,
      `the exact line ${nonceLine(nonce)}`,
      "then only the corrected JSON value matching this JSON Schema, and nothing else:",
      JSON.stringify(schema),
      "</cowboy_output_contract>",
    ].join("\n"),
  };
}

/** The line binding a corrected reply to its correction prompt. */
export function nonceLine(nonce) {
  return `cowboy-nonce: ${nonce}`;
}

/**
 * A corrected reply's JSON text once its first line is the correction's
 * nonce line, else `undefined`. Nothing produced before the correction was
 * sent can carry the nonce, so a delayed message of the failed attempt can
 * never become the corrected answer.
 */
export function afterNonce(reply, nonce) {
  const text = String(reply ?? "").trimStart();
  const line = nonceLine(nonce);
  if (!text.startsWith(line)) return undefined;
  const rest = text.slice(line.length);
  return rest === "" || rest.startsWith("\n") ? rest : undefined;
}

const TYPES = Object.freeze({
  __proto__: null,
  object: (value) => value !== null && typeof value === "object" && !Array.isArray(value),
  array: Array.isArray,
  string: (value) => typeof value === "string",
  integer: Number.isInteger,
  number: (value) => typeof value === "number" && Number.isFinite(value),
  boolean: (value) => typeof value === "boolean",
  null: (value) => value === null,
});
const isObject = TYPES.object;

/** Keywords that constrain nothing: accepted and not checked. */
const ANNOTATIONS = new Set(["title", "description", "$comment"]);
/** Assertions `schemaProblem` checks exactly. */
const ASSERTIONS = new Set([
  "type",
  "enum",
  "properties",
  "required",
  "additionalProperties",
  "items",
  "minItems",
  "maxItems",
  "minLength",
  "maxLength",
  "minimum",
  "maximum",
]);
const COUNTS = new Set(["minItems", "maxItems", "minLength", "maxLength"]);

/**
 * Why `schema` cannot be enforced here, or `undefined`: a keyword outside
 * the supported set or a malformed value.
 */
export function schemaRefusal(schema) {
  if (!isObject(schema)) return "outputSchema must be a JSON Schema object";
  const problem = walk(schema, "#", 0);
  return problem && `outputSchema is not supported by Antigravity: ${problem}`;
}

function walk(schema, at, depth) {
  if (typeof schema === "boolean") return undefined;
  if (!isObject(schema)) return `${at} is not a schema`;
  if (depth > MAX_DEPTH) return `${at} is nested too deeply`;
  for (const [key, value] of Object.entries(schema)) {
    if (ANNOTATIONS.has(key)) continue;
    if (!ASSERTIONS.has(key)) return `${at} uses ${key}`;
    if (malformed(key, value)) return `${at}/${key} is malformed`;
  }
  const children = [];
  for (const [name, child] of Object.entries(schema.properties ?? {})) {
    children.push([child, `${at}/properties/${name}`]);
  }
  for (const key of ["additionalProperties", "items"]) {
    if (key in schema) children.push([schema[key], `${at}/${key}`]);
  }
  for (const [child, path] of children) {
    const problem = walk(child, path, depth + 1);
    if (problem) return problem;
  }
  return undefined;
}

/** Whether a supported keyword's value is malformed. */
function malformed(key, value) {
  switch (key) {
    case "type": {
      const types = Array.isArray(value) ? value : [value];
      return !(types.length && types.every((type) => typeof type === "string" && type in TYPES));
    }
    case "enum":
      // Scalars only: membership needs no structural comparison.
      return !(
        Array.isArray(value) && value.every((item) => item === null || typeof item !== "object")
      );
    case "required":
      return !(Array.isArray(value) && value.every((name) => typeof name === "string"));
    case "properties":
      return !isObject(value);
    case "items":
    case "additionalProperties":
      return !(isObject(value) || typeof value === "boolean");
    default:
      if (COUNTS.has(key)) return !(Number.isSafeInteger(value) && value >= 0);
      return !Number.isSafeInteger(value);
  }
}

/**
 * The JSON value of a reply: the whole text, or one fenced block. A value
 * the check cannot hold exactly (an integer past the safe range, a number
 * that overflows, a lone surrogate) is refused, so the checked value and
 * its serialization are the same JSON.
 */
export function replyJson(text) {
  const trimmed = String(text ?? "").trim();
  if (trimmed.length > MAX_REPLY_CHARS) return { problem: "the reply is too long" };
  const body = unfenced(trimmed);
  let inexact;
  let value;
  try {
    value = JSON.parse(body, (key, item) => {
      if (typeof item === "number" && !Number.isFinite(item)) inexact = "a number out of range";
      if (typeof item === "number" && Number.isInteger(item) && !Number.isSafeInteger(item)) {
        inexact = "an integer too large to check exactly";
      }
      if (typeof item === "string" && !item.isWellFormed()) inexact = "an unpaired surrogate";
      if (!key.isWellFormed()) inexact = "an unpaired surrogate";
      return item;
    });
  } catch {
    return { problem: "the reply is not a single JSON value" };
  }
  if (inexact) return { problem: `the reply has ${inexact}` };
  if (depthOf(value) > MAX_DEPTH) return { problem: "the reply is nested too deeply" };
  return { value };
}

/**
 * The body of one Markdown fence spanning the whole reply (an opening line
 * of three backticks and an optional `json`, a closing line of three
 * backticks), else the reply itself. Linear string operations only.
 */
function unfenced(text) {
  if (!text.startsWith("```") || !text.endsWith("```") || text.length < 7) return text;
  const newline = text.indexOf("\n");
  if (newline < 0) return text;
  const opening = text.slice(3, newline).trim();
  if (opening !== "" && opening !== "json") return text;
  const inner = text.slice(newline + 1, text.length - 3);
  return inner.endsWith("\n") ? inner.slice(0, -1) : inner;
}

function depthOf(value) {
  let deepest = 0;
  const stack = [[value, 0]];
  while (stack.length) {
    const [item, depth] = stack.pop();
    if (depth > deepest) deepest = depth;
    if (depth > MAX_DEPTH) return depth;
    if (item !== null && typeof item === "object") {
      for (const child of Object.values(item)) stack.push([child, depth + 1]);
    }
  }
  return deepest;
}

/**
 * The first violation of `schema` by `value` at `path`, or `undefined`.
 * Only a schema `schemaRefusal` accepts is checked; every keyword it admits
 * is checked here, in time linear in the reply.
 */
export function schemaProblem(schema, value, path = "$") {
  if (schema === true || schema === undefined) return undefined;
  if (schema === false) return `${path} is not allowed`;
  if (!isObject(schema)) return undefined;
  if (schema.type !== undefined) {
    const types = Array.isArray(schema.type) ? schema.type : [schema.type];
    if (!types.some((type) => TYPES[type]?.(value))) return `${path} must be ${types.join(" or ")}`;
  }
  if (Array.isArray(schema.enum) && !schema.enum.includes(value)) {
    return `${path} must be one of ${JSON.stringify(schema.enum)}`;
  }
  if (typeof value === "string") {
    // JSON Schema counts code points, not UTF-16 units.
    const length = [...value].length;
    if (schema.minLength !== undefined && length < schema.minLength) return `${path} is too short`;
    if (schema.maxLength !== undefined && length > schema.maxLength) return `${path} is too long`;
  }
  if (typeof value === "number") {
    if (schema.minimum !== undefined && value < schema.minimum) return `${path} is too small`;
    if (schema.maximum !== undefined && value > schema.maximum) return `${path} is too large`;
  }
  if (Array.isArray(value)) {
    if (schema.minItems !== undefined && value.length < schema.minItems)
      return `${path} has too few items`;
    if (schema.maxItems !== undefined && value.length > schema.maxItems)
      return `${path} has too many items`;
    if (schema.items !== undefined) {
      for (let index = 0; index < value.length; index++) {
        const problem = schemaProblem(schema.items, value[index], `${path}[${index}]`);
        if (problem) return problem;
      }
    }
  }
  if (isObject(value)) {
    for (const key of Array.isArray(schema.required) ? schema.required : []) {
      if (!Object.hasOwn(value, key)) return `${path}.${key} is required`;
    }
    const properties = isObject(schema.properties) ? schema.properties : {};
    for (const [key, item] of Object.entries(value)) {
      if (Object.hasOwn(properties, key)) {
        const problem = schemaProblem(properties[key], item, `${path}.${key}`);
        if (problem) return problem;
      } else if (schema.additionalProperties === false) {
        // The name is the model's text: never replayed into a prompt.
        return `${path} has a property its schema does not allow`;
      } else if (schema.additionalProperties !== undefined) {
        const problem = schemaProblem(schema.additionalProperties, item, `${path}.*`);
        if (problem) return problem;
      }
    }
  }
  return undefined;
}

/**
 * The reply checked against `schema`: `{ text }` (the checked value's JSON)
 * when valid, else `{ problem }`. Never throws.
 */
export function checkReply(schema, text) {
  try {
    const parsed = replyJson(text);
    if (parsed.problem) return { problem: parsed.problem };
    const problem = schemaProblem(schema, parsed.value);
    return problem ? { problem: problem.slice(0, 300) } : { text: JSON.stringify(parsed.value) };
  } catch {
    return { problem: "the reply could not be checked" };
  }
}
