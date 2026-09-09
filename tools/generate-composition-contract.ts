// Core contract, not a separately installable Plugin SDK. Run via the pinned shell.
// Only this deliberately closed, non-recursive schema profile is supported.
import { strictJson } from "../contracts/strict-json.ts";
interface Shape {
  $ref?: string;
  type?: "object" | "string" | "array";
  additionalProperties?: false;
  required?: string[];
  properties?: Record<string, Shape>;
  items?: Shape;
  minItems?: number;
  maxItems?: number;
  minLength?: number;
  maxLength?: number;
  pattern?: string;
  enum?: string[];
  const?: string;
  oneOf?: Shape[];
  "x-max-decimal"?: string;
}

const source = "contracts/composition-v1.schema.json";
const schema = strictJson(await Deno.readTextFile(source), 1048576, 32) as {
  $schema: string;
  $id: string;
  title: string;
  $ref: string;
  $defs: Record<string, Shape>;
  "x-json-budget": { max_bytes: number; max_depth: number };
  "x-profile": string;
};
const defs: Record<string, Shape> = schema.$defs;
const budget = schema["x-json-budget"];
const allowed = new Set([
  "$ref",
  "type",
  "additionalProperties",
  "required",
  "properties",
  "items",
  "minItems",
  "maxItems",
  "minLength",
  "maxLength",
  "pattern",
  "enum",
  "const",
  "oneOf",
  "x-max-decimal",
]);
const quoted = JSON.stringify;
const variant = (name: string): string =>
  name.split(/[._-]/).map((part) => part[0].toUpperCase() + part.slice(1)).join(
    "",
  );
function onlyKeys(value: object, keys: string[]): void {
  if (Object.keys(value).some((key) => !keys.includes(key))) {
    throw new Error("unsupported schema keyword or keyword combination");
  }
}
onlyKeys(schema, [
  "$schema",
  "$id",
  "title",
  "$ref",
  "$defs",
  "x-profile",
  "x-json-budget",
]);
onlyKeys(budget, ["max_bytes", "max_depth"]);
// Explicit common ASCII subset: adding a refinement requires cross-runtime
// vectors and profile review, not silently executing arbitrary JS regexes.
const patterns = new Set([
  "^[a-z0-9][a-z0-9._-]*$",
  "^(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)$",
  "^sha256:[a-f0-9]{64}$",
  "^(0|[1-9][0-9]*)$",
]);
function reference(shape: Shape): string {
  if (!shape.$ref?.startsWith("#/$defs/")) {
    throw new Error("local named reference required");
  }
  const name = shape.$ref.slice(8);
  if (!(name in defs)) throw new Error(`unresolved definition: ${name}`);
  return name;
}
function check(shape: Shape, trail: string[] = []): void {
  for (const key of Object.keys(shape)) {
    if (!allowed.has(key)) {
      throw new Error(`unsupported schema keyword: ${key}`);
    }
  }
  if (shape.$ref) {
    onlyKeys(shape, ["$ref"]);
    const name = reference(shape);
    if (trail.includes(name)) {
      throw new Error("recursive contracts are not supported");
    }
    check(defs[name], [...trail, name]);
  } else if (shape.oneOf) {
    onlyKeys(shape, ["oneOf"]);
    const tags = shape.oneOf.map((branch) => branch.properties?.kind?.const);
    if (tags.some((tag) => !tag) || new Set(tags).size !== tags.length) {
      throw new Error("a union requires unique string kind tags");
    }
    for (const branch of shape.oneOf) check(branch, trail);
  } else if (shape.type === "object") {
    onlyKeys(shape, ["type", "additionalProperties", "required", "properties"]);
    const fields = Object.keys(shape.properties ?? {});
    if (
      shape.additionalProperties !== false || !shape.required ||
      fields.join() !== shape.required.join()
    ) throw new Error("closed required-field records only");
    for (const child of Object.values(shape.properties!)) check(child, trail);
  } else if (shape.type === "array") {
    onlyKeys(shape, ["type", "items", "minItems", "maxItems"]);
    if (
      !Number.isSafeInteger(shape.minItems) ||
      !Number.isSafeInteger(shape.maxItems) ||
      shape.minItems! < 0 || shape.maxItems! < shape.minItems! ||
      shape.maxItems! > 2048
    ) {
      throw new Error("bounded arrays required");
    }
    reference(shape.items!);
    check(shape.items!, trail);
  } else if (shape.type === "string") {
    if (shape.enum) {
      onlyKeys(shape, ["type", "enum"]);
      if (
        !shape.enum.length || new Set(shape.enum).size !== shape.enum.length
      ) {
        throw new Error("nonempty unique enums required");
      }
    } else {
      onlyKeys(shape, [
        "type",
        "minLength",
        "maxLength",
        "pattern",
        "x-max-decimal",
      ]);
      if (
        shape["x-max-decimal"] !== undefined &&
        (shape.pattern !== "^(0|[1-9][0-9]*)$" ||
          !/^[1-9][0-9]{0,19}$/.test(shape["x-max-decimal"]) ||
          shape.maxLength !== shape["x-max-decimal"].length)
      ) {
        throw new Error("invalid canonical decimal bound");
      }
      if (
        !Number.isSafeInteger(shape.maxLength) ||
        !Number.isSafeInteger(shape.minLength) ||
        !shape.maxLength || !shape.minLength || !shape.pattern ||
        shape.minLength < 1 ||
        shape.maxLength > 128 || shape.minLength > shape.maxLength ||
        !patterns.has(shape.pattern)
      ) {
        throw new Error("bounded anchored string refinements required");
      }
    }
  } else {
    onlyKeys(shape, ["const"]);
    if (typeof shape.const !== "string") {
      throw new Error("unsupported contract shape");
    }
  }
}
for (const shape of Object.values(defs)) check(shape);
if (
  schema.$schema !== "https://json-schema.org/draft/2020-12/schema" ||
  schema["x-profile"] !== "cowboy.closed-contract.v1" ||
  schema.$ref !== "#/$defs/Composition" || budget.max_bytes !== 1048576 ||
  budget.max_depth !== 32
) {
  throw new Error("unsupported profile, entry point or JSON budget");
}
function canonical(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (value !== null && typeof value === "object") {
    return `{${
      Object.entries(value).sort(([a], [b]) => a < b ? -1 : a > b ? 1 : 0)
        .map(([key, child]) => `${quoted(key)}:${canonical(child)}`).join(",")
    }}`;
  }
  return quoted(value);
}
const hash = await crypto.subtle.digest(
  "SHA-256",
  new TextEncoder().encode(
    `cowboy.closed-contract.v1\n${canonical(schema)}`,
  ),
);
const fingerprint = "sha256:" +
  [...new Uint8Array(hash)].map((b) => b.toString(16).padStart(2, "0")).join(
    "",
  );
const banner = `// Generated from ${source}; do not edit.\n`;
let rust = banner + `use serde::{Deserialize, Serialize};\n
pub(super) const CONTRACT_FINGERPRINT: &str = ${quoted(fingerprint)};
pub(super) const MAX_BYTES: usize = ${budget.max_bytes};
pub(super) const MAX_DEPTH: usize = ${budget.max_depth};
pub(super) trait Validate { fn valid(&self) -> bool; }\n`;
let ts = banner + `import { strictJson } from "./strict-json.ts";\n
export const CONTRACT_FINGERPRINT = ${quoted(fingerprint)};
export const MAX_BYTES = ${budget.max_bytes};
export const MAX_DEPTH = ${budget.max_depth};
declare const identity: unique symbol;\n
function record(value: unknown, keys: readonly string[]): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value) &&
    Object.keys(value).length === keys.length && keys.every((key) => Object.hasOwn(value, key));
}\n`;
const derive =
  "#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]\n";
function fieldType(shape: Shape, lang: "rust" | "ts"): string {
  if (shape.type === "array") {
    const name = reference(shape.items!);
    return lang === "rust" ? `Vec<${name}>` : `readonly ${name}[]`;
  }
  return reference(shape);
}
function validField(shape: Shape, access: string, lang: "rust" | "ts"): string {
  if (shape.type === "array") {
    const name = reference(shape.items!);
    return lang === "rust"
      ? `${
        shape.minItems === 1
          ? `!${access}.is_empty() && `
          : shape.minItems
          ? `${access}.len() >= ${shape.minItems} && `
          : ""
      }${access}.len() <= ${shape.maxItems} && ${access}.iter().all(Validate::valid)`
      : `Array.isArray(${access}) && ${access}.length >= ${shape.minItems} && ${access}.length <= ${shape.maxItems} && ${access}.every(is${name})`;
  }
  return lang === "rust"
    ? `${access}.valid()`
    : `is${reference(shape)}(${access})`;
}
for (const [name, shape] of Object.entries(defs)) {
  if (shape.type === "string" && !shape.enum) {
    // This profile deliberately admits ASCII only. Both runtimes count the
    // same units and never rely on differing Unicode regex semantics.
    const decimal = shape["x-max-decimal"];
    rust +=
      `${derive}#[serde(transparent)]\npub(super) struct ${name}(pub(super) String);
impl Validate for ${name} { fn valid(&self) -> bool {
  static PATTERN: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(${
        quoted(shape.pattern)
      }).expect("generated contract regex"));
  self.0.is_ascii() && ${
        shape.minLength === 1
          ? "!self.0.is_empty()"
          : `self.0.len() >= ${shape.minLength}`
      } && self.0.len() <= ${shape.maxLength} && PATTERN.is_match(&self.0)
  ${
        decimal
          ? `&& (self.0.len() < ${decimal.length} || self.0.as_str() <= ${
            quoted(decimal)
          })`
          : ""
      }
}}\n`;
    ts += `export type ${name} = string & { readonly [identity]: ${
      quoted(name)
    } };
function is${name}(value: unknown): value is ${name} {
  return typeof value === "string" && /^[\\x00-\\x7f]*$/.test(value) && value.length >= ${shape.minLength} && value.length <= ${shape.maxLength} && new RegExp(${
      quoted(shape.pattern)
    }).exec(value)?.[0] === value
  ${
      decimal
        ? `&& (value.length < ${decimal.length} || value <= ${quoted(decimal)})`
        : ""
    };
}\n`;
  } else if (shape.enum) {
    rust += `${derive}pub(super) enum ${name} { ${
      shape.enum.map((v) => `#[serde(rename = ${quoted(v)})] ${variant(v)},`)
        .join("\n")
    } }
impl Validate for ${name} { fn valid(&self) -> bool { true } }\n`;
    ts += `export type ${name} = ${
      shape.enum.map((v) => quoted(v)).join(" | ")
    };
function is${name}(value: unknown): value is ${name} { return ${
      shape.enum.map((v) => `value === ${quoted(v)}`).join(" || ")
    }; }\n`;
  } else {
    const branches = shape.oneOf ?? [shape];
    const rustVariants: string[] = [];
    const rustChecks: string[] = [];
    const tsVariants: string[] = [];
    const tsChecks: string[] = [];
    for (const branch of branches) {
      const properties = branch.properties!;
      const tag = properties.kind?.const;
      const fields = Object.entries(properties).filter(([key]) =>
        key !== "kind"
      );
      rustVariants.push(
        `${
          tag
            ? `#[serde(rename = ${quoted(tag)})] ${variant(tag)}`
            : `pub(super) struct ${name}`
        } { ${
          fields.map(([key, child]) =>
            `${tag ? "" : "pub(super) "}${key}: ${fieldType(child, "rust")},`
          ).join("\n")
        } }`,
      );
      const values = fields.map(([key, child]) =>
        validField(child, tag ? key : `self.${key}`, "rust")
      ).join(" && ") || "true";
      rustChecks.push(
        tag
          ? `Self::${variant(tag)} { ${
            fields.map(([key]) =>
              key
            ).join(",")
          } } => ${values},`
          : values,
      );
      tsVariants.push(
        `{ ${tag ? `readonly kind: ${quoted(tag)};` : ""} ${
          fields.map(([key, child]) =>
            `readonly ${key}: ${fieldType(child, "ts")};`
          ).join(" ")
        } }`,
      );
      tsChecks.push(
        `(record(value, ${quoted(Object.keys(properties))}) ${
          tag ? `&& value.kind === ${quoted(tag)}` : ""
        } ${
          fields.map(([key, child]) =>
            `&& (${validField(child, `value.${key}`, "ts")})`
          ).join(" ")
        })`,
      );
    }
    rust += `${derive}#[serde(${
      shape.oneOf ? 'tag = "kind", ' : ""
    }deny_unknown_fields)]\n${
      shape.oneOf
        ? `pub(super) enum ${name} { ${rustVariants.join(",\n")} }`
        : rustVariants[0]
    }
impl Validate for ${name} { fn valid(&self) -> bool { ${
      shape.oneOf ? `match self { ${rustChecks.join("\n")} }` : rustChecks[0]
    } } }\n`;
    ts += `export type ${name} = ${tsVariants.join(" | ")};
function is${name}(value: unknown): value is ${name} { return ${
      tsChecks.join(" || ")
    }; }\n`;
  }
}
ts += `
/** Decodes untrusted data, never an authorization or a verified release. */
export function decodeComposition(raw: string): Composition {
  const value: unknown = strictJson(raw, MAX_BYTES, MAX_DEPTH);
  if (!isComposition(value)) throw new Error("invalid_contract");
  return value;
}\n`;
async function formatted(
  command: string,
  args: string[],
  value: string,
): Promise<string> {
  const child = new Deno.Command(command, {
    args,
    stdin: "piped",
    stdout: "piped",
    stderr: "piped",
  }).spawn();
  const writer = child.stdin.getWriter();
  await writer.write(new TextEncoder().encode(value));
  await writer.close();
  const result = await child.output();
  if (!result.success) throw new Error(new TextDecoder().decode(result.stderr));
  return new TextDecoder().decode(result.stdout);
}
for (
  const [path, value] of [
    [
      "src/composition/wire.rs",
      await formatted("rustfmt", ["--edition", "2024"], rust),
    ],
    [
      "contracts/composition.generated.ts",
      await formatted(Deno.execPath(), ["fmt", "--ext=ts", "-"], ts),
    ],
  ]
) {
  if (Deno.args.includes("--write")) await Deno.writeTextFile(path, value);
  else if (await Deno.readTextFile(path) !== value) {
    throw new Error(`${path}: stale contract generation`);
  }
}
console.log(
  `composition contract ${fingerprint}: ${
    Deno.args.includes("--write") ? "generated" : "checked"
  }`,
);
