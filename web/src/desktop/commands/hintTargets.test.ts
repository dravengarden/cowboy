import { assertEquals } from "jsr:@std/assert";
import { assignMnemonics } from "./hintTargets.ts";

Deno.test("mnemonics prefer explicit keys, then word initials, then letters", () => {
  assertEquals(
    assignMnemonics([
      { name: "Cancel" },
      { name: "Create session" },
      { name: "Session", preferred: "s" },
      { name: "Copy" },
    ]),
    ["c", "r", "s", "o"],
  );
});

Deno.test("mnemonics stay stable and unique across many controls", () => {
  const names = Array.from({ length: 40 }, (_, index) => ({ name: `Item ${index}` }));
  const first = assignMnemonics(names);
  assertEquals(first, assignMnemonics(names));
  const assigned = first.filter((key) => key !== null);
  assertEquals(new Set(assigned).size, assigned.length);
  assertEquals(assigned.length, 36);
});

Deno.test("CJK names fall back to free letters", () => {
  assertEquals(assignMnemonics([{ name: "创建" }, { name: "取消" }]), ["a", "s"]);
});
