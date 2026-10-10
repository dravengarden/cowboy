import { test } from "bun:test";
import { assertEquals } from "@std/assert";
import { presentHoverBlock } from "./symbolPresentation";

test("symbol docs convert HTML breaks and hide rustdoc setup lines", () => {
  const block = presentHoverBlock({
    markdown: true,
    text:
      "Summary<br><br>```rust\n# use std::io;\n#\nlet value = load()?;\n```\n<br>Details",
  });
  assertEquals(
    block.text,
    "Summary\n\n```rust\nlet value = load()?;\n```\n\nDetails",
  );
});

test("symbol docs preserve headings outside Rust fences", () => {
  const block = presentHoverBlock({
    markdown: true,
    text: "## Details\n\n```text\n# visible output\n```",
  });
  assertEquals(block.text, "## Details\n\n```text\n# visible output\n```");
});
