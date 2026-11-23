import { test } from "bun:test";
import { assertEquals } from "../../../../tools/test-assert.ts";
import {
  connectVimRegisterController,
  readVimRegister,
  writeVimRegister,
} from "./vimRegister.ts";

test("vimRegister falls back to local state before CodeMirror Vim loads", () => {
  connectVimRegisterController(null);
  writeVimRegister("hello world", false);
  assertEquals(readVimRegister(), { text: "hello world", linewise: false });
});

test("vimRegister bridges two-way with CodeMirror RegisterController and strips linewise trailing newline", () => {
  let cmText = "";
  let cmLinewise = false;
  connectVimRegisterController(null);
  writeVimRegister("preloaded", false);

  connectVimRegisterController({
    read: () => ({ text: cmText, linewise: cmLinewise }),
    write: (text, linewise) => {
      cmText = linewise && !text.endsWith("\n") ? `${text}\n` : text;
      cmLinewise = linewise;
    },
  });
  // Fallback written before CodeMirror loaded seeds an empty controller.
  assertEquals(cmText, "preloaded");
  assertEquals(readVimRegister(), { text: "preloaded", linewise: false });

  // Linewise yank inside CodeMirror stores a trailing '\n'; readVimRegister
  // strips that single trailing newline for clean native textarea/input paste.
  cmText = "const answer = 42;\n";
  cmLinewise = true;
  assertEquals(readVimRegister(), { text: "const answer = 42;", linewise: true });

  // Native input yank writes back into CodeMirror's RegisterController.
  writeVimRegister("renamed title", false);
  assertEquals(cmText, "renamed title");
  assertEquals(cmLinewise, false);

  // Native textarea linewise yank forwards linewise flag cleanly.
  writeVimRegister("full line", true);
  assertEquals(cmText, "full line\n");
  assertEquals(cmLinewise, true);

  connectVimRegisterController(null);
  writeVimRegister("", false);
});
