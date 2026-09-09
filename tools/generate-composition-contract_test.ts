// Bad schemas must fail before any generator output or external reference is used.
const source = JSON.parse(
  await Deno.readTextFile("contracts/composition-v1.schema.json"),
);
const generator =
  new URL("./generate-composition-contract.ts", import.meta.url).pathname;
const expected: Record<string, string> = {
  "unknown root keyword": "unsupported schema keyword",
  "unknown scalar keyword": "unsupported schema keyword",
  "ignored ref sibling": "unsupported schema keyword",
  "unsupported regex dialect": "bounded anchored string refinements required",
  "incompatible decimal refinement": "invalid canonical decimal bound",
  "remote reference": "local named reference required",
  "recursive reference": "recursive contracts are not supported",
  "open record": "closed required-field records only",
  "unbounded array": "bounded arrays required",
  "fractional length": "bounded anchored string refinements required",
  "unsupported dialect": "unsupported profile, entry point or JSON budget",
};

for (
  const [name, path, replacement] of [
    ["unknown root keyword", "/allOf", []],
    ["unknown scalar keyword", "/$defs/ScopeId/format", "uuid"],
    ["ignored ref sibling", "/$defs/Scope/properties/id/maxLength", 1],
    ["unsupported regex dialect", "/$defs/ScopeId/pattern", "^(?=x)x$"],
    ["incompatible decimal refinement", "/$defs/ScopeId/x-max-decimal", "99"],
    [
      "remote reference",
      "/$defs/Scope/properties/id/$ref",
      "https://invalid.example/schema",
    ],
    ["recursive reference", "/$defs/Scope/properties/id/$ref", "#/$defs/Scope"],
    ["open record", "/$defs/Scope/additionalProperties", true],
    ["unbounded array", "/$defs/Composition/properties/nodes/maxItems", null],
    ["fractional length", "/$defs/ScopeId/minLength", 1.5],
    [
      "unsupported dialect",
      "/$schema",
      "https://json-schema.org/draft-07/schema",
    ],
  ] as const
) {
  Deno.test(`composition generator rejects ${name}`, async () => {
    const temporary = await Deno.makeTempDir({
      prefix: "cowboy-contract-test-",
    });
    try {
      await Deno.mkdir(`${temporary}/contracts`);
      const schema = structuredClone(source);
      const segments = path.slice(1).split("/");
      const key = segments.pop()!;
      let parent = schema;
      for (const segment of segments) parent = parent[segment];
      parent[key] = replacement;
      await Deno.writeTextFile(
        `${temporary}/contracts/composition-v1.schema.json`,
        JSON.stringify(schema),
      );
      const output = await new Deno.Command(Deno.execPath(), {
        args: ["run", "--allow-read", generator],
        cwd: temporary,
        stdout: "piped",
        stderr: "piped",
      }).output();
      if (
        output.success ||
        new TextDecoder().decode(output.stdout).includes("checked")
      ) {
        throw new Error(`invalid schema emitted output: ${name}`);
      }
      const error = new TextDecoder().decode(output.stderr);
      if (!error.includes(expected[name])) {
        throw new Error(`wrong rejection for ${name}: ${error}`);
      }
      if (error.includes("Requires run access") || error.includes("NotFound")) {
        throw new Error(
          `schema reached generation instead of profile rejection: ${name}`,
        );
      }
    } finally {
      await Deno.remove(temporary, { recursive: true });
    }
  });
}
