import { assertThrows } from "jsr:@std/assert@1.0.19";
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
