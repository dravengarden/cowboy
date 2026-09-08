import { assert, assertThrows } from "jsr:@std/assert@1.0.19";
import {
  validateIndependentPluginVersion,
  validateReleaseHistory,
} from "./check-plugin-components.ts";

const component = {
  id: "cowboy.plugin-contract",
  version: "1.0.0",
  publisher: "cowboy",
  sources: ["components/plugin-contract"],
  digest: "sha256:fixture",
};

Deno.test("a component release requires every plugin version to increase", () => {
  const first = {
    version: "1.0.0",
    components: [component],
    plugins: { codex: "1.0.0", zed: "1.0.0" },
  };
  assertThrows(
    () =>
      validateReleaseHistory([
        first,
        {
          version: "1.1.0",
          components: [{ ...component, version: "1.1.0" }],
          plugins: { codex: "1.1.0", zed: "1.0.0" },
        },
      ]),
    Error,
    "zed must increase version",
  );

  validateReleaseHistory([
    first,
    {
      version: "1.1.0",
      components: [{ ...component, version: "1.1.0" }],
      plugins: { codex: "1.1.0", zed: "2.0.0" },
    },
  ]);
});

Deno.test("a plugin can release independently above its component baseline", () => {
  validateIndependentPluginVersion("codex", "1.2.0", "1.1.0");
  assertThrows(
    () => validateIndependentPluginVersion("codex", "1.0.9", "1.1.0"),
    Error,
    "predates the active component release",
  );
});

Deno.test("published capability schemas are independently resolvable", async () => {
  for (
    const name of [
      "authentication-provider",
      "authentication-provider-v2",
      "telemetry-backend",
    ]
  ) {
    const schema = JSON.parse(
      await Deno.readTextFile(
        `components/plugin-contract/${name}.schema.json`,
      ),
    );
    const inspect = (value: unknown): void => {
      if (!value || typeof value !== "object") return;
      if ("$ref" in value) {
        assert(
          typeof value.$ref === "string" && value.$ref.startsWith("#/"),
          `${name}: a published schema must not depend on external resolution`,
        );
        let target: unknown = schema;
        for (const token of value.$ref.slice(2).split("/")) {
          assert(target !== null && typeof target === "object");
          target = (target as Record<string, unknown>)[
            token.replaceAll("~1", "/").replaceAll("~0", "~")
          ];
        }
        assert(
          target !== undefined,
          `${name}: dangling reference ${value.$ref}`,
        );
      }
      for (const child of Object.values(value)) inspect(child);
    };
    inspect(schema);
  }
});
