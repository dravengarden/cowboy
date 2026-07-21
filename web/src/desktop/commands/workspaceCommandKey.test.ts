import { strict as assert } from "node:assert";
import { workspaceCommandKey } from "./workspaceCommandKey";

Deno.test("workspace Vim motions use physical keys under an IME input source", () => {
  assert.equal(
    workspaceCommandKey({ code: "KeyJ", key: "Process", shiftKey: false }),
    "j",
  );
  assert.equal(
    workspaceCommandKey({ code: "KeyK", key: "に", shiftKey: false }),
    "k",
  );
});

Deno.test("workspace Vim motions preserve shifted G and native non-letter keys", () => {
  assert.equal(
    workspaceCommandKey({ code: "KeyG", key: "Process", shiftKey: true }),
    "G",
  );
  assert.equal(
    workspaceCommandKey({ code: "Enter", key: "Enter", shiftKey: false }),
    "Enter",
  );
  assert.equal(
    workspaceCommandKey({
      code: "BracketLeft",
      key: "Process",
      shiftKey: false,
    }),
    "[",
  );
  assert.equal(
    workspaceCommandKey({ code: "Escape", key: "Process", shiftKey: false }),
    "Escape",
  );
});
