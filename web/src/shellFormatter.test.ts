import { assertEquals } from "jsr:@std/assert";
import {
  addRegexSoftBreaks,
  addShellPathBreaks,
  compactNixStoreExecutables,
  stripStructuralMarkerReference,
} from "./shellFormatter.ts";

Deno.test("shell path wrapping prefers separators without changing visible text", () => {
  const source = "BRIDGE=/home/draven/chrome-debug-bridge/helpers/bridge.sh";
  const display = addShellPathBreaks(source);

  assertEquals(display.replaceAll("\u200b", ""), source);
  assertEquals(display, "BRIDGE=/\u200bhome/\u200bdraven/\u200bchrome-debug-bridge/\u200bhelpers/\u200bbridge.sh");
});

Deno.test("nested structural markers stay outside highlighted command text", () => {
  assertEquals(stripStructuralMarkerReference("Nix bin › bash \\\n  -lc '🍊'", "🍊"), "Nix bin › bash");
  assertEquals(stripStructuralMarkerReference("ssh macbook-air '🌸'", "🌸"), "ssh macbook-air");
  assertEquals(stripStructuralMarkerReference("echo '🍊' later", "🍊"), "echo '🍊' later");
});

Deno.test("regex soft wrapping preserves escaped pipes and character classes", () => {
  const source = String.raw`alpha|beta\|literal|[a|b]`;
  const display = addRegexSoftBreaks(source);
  if (display !== `alpha|\u200bbeta\\|literal|\u200b[a|b]`) {
    throw new Error(`unexpected regex display: ${JSON.stringify(display)}`);
  }
  if (display.replaceAll("\u200b", "") !== source) {
    throw new Error("display-only regex breaks changed source bytes");
  }
});

Deno.test("readable shell compacts canonical Nix executable paths", () => {
  const hash = "0641h8qfqaxnwrsw2nzrz6i1wbzyx921";
  assertEquals(
    compactNixStoreExecutables(`/nix/store/${hash}-bash-interactive-5.3p9/bin/bash -lc 'echo ok'`),
    "Nix bin › bash -lc 'echo ok'",
  );
  assertEquals(
    compactNixStoreExecutables(`/nix/store/${hash}-python-env/libexec/tools/python3 script.py`),
    "Nix bin › python3 script.py",
  );
});

Deno.test("complex embedded JSON is formatted while invalid JSON falls back intact", async () => {
  const { formatEmbeddedFrame } = await import("./shellFormatter.ts");
  assertEquals(
    (await formatEmbeddedFrame({ launcher: "json", language: "json", text: '{"name":"cowboy","items":[1,2]}' }, 46)).text,
    '{\n  "name": "cowboy",\n  "items": [\n    1,\n    2\n  ]\n}',
  );
  assertEquals(
    (await formatEmbeddedFrame({ launcher: "json", language: "json", text: "{broken" }, 46)).text,
    "{broken",
  );
});

Deno.test("Nix compaction leaves data paths and noncanonical hashes exact", () => {
  const canonical = "/nix/store/0641h8qfqaxnwrsw2nzrz6i1wbzyx921-source/share/schema.json";
  const lookalike = "/nix/store/not-a-real-store-hash-bash/bin/bash";
  assertEquals(compactNixStoreExecutables(canonical), canonical);
  assertEquals(compactNixStoreExecutables(lookalike), lookalike);
});
