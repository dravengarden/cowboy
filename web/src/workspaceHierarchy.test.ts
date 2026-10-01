import { assertEquals } from "jsr:@std/assert";
import { workspaceBranch, workspaceTree } from "./workspaceHierarchy";

Deno.test("hierarchy preserves selectable parents and opaque duplicate identities", () => {
  const root = workspaceTree([
    { value: "a", label: "hawk/columbus", help: "/root/a" },
    { value: "b", label: "hawk/columbus/cowboy", help: "/root/b" },
    { value: "c", label: "hawk/columbus/cowboy", help: "/root/c" },
    { value: "d", label: "falcon/suger", help: "/root/d" },
  ]);
  const parent = workspaceBranch(root, ["hawk", "columbus"]);
  assertEquals(parent.entries.map((entry) => entry.value), ["a"]);
  assertEquals(
    parent.children.get("cowboy")?.entries.map((entry) => entry.value),
    ["b", "c"],
  );
  assertEquals(workspaceBranch(root, ["removed"]), root);
});
