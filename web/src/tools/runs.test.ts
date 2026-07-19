import { assertEquals } from "jsr:@std/assert";
import type { RenderItem } from "../derive";
import { toolRuns } from "./runs";

function mcp(key: string, server: string, tool: string): RenderItem {
  return {
    kind: "tool",
    key,
    id: key,
    title: `mcp.${server}.${tool}`,
    toolKind: "execute",
    toolName: "",
    status: "completed",
    rawInput: { server, tool, arguments: {} },
  };
}

Deno.test("continuous calls to one MCP server form a run across thought rows", () => {
  const items: RenderItem[] = [
    mcp("1", "chrome-devtools", "navigate_page"),
    { kind: "thought", key: "2", sections: ["Inspecting page"] },
    mcp("3", "chrome-devtools", "evaluate_script"),
    mcp("4", "chrome-devtools", "take_screenshot"),
  ];
  assertEquals(toolRuns(items).map((run) => [run.server, run.tools.map((tool) => tool.key)]), [
    ["chrome-devtools", ["1", "3", "4"]],
  ]);
});

Deno.test("messages, ordinary tools, and another MCP server end a run", () => {
  const ordinary: RenderItem = {
    kind: "tool",
    key: "4",
    id: "4",
    title: "Shell command",
    toolKind: "execute",
    toolName: "shell",
    status: "completed",
    rawInput: { command: "true" },
  };
  const items: RenderItem[] = [
    mcp("1", "chrome-devtools", "navigate_page"),
    { kind: "message", key: "2", role: "assistant", chunks: [{ type: "text", text: "Done" }] },
    mcp("3", "chrome-devtools", "evaluate_script"),
    ordinary,
    mcp("5", "github", "search_code"),
  ];
  assertEquals(toolRuns(items).map((run) => [run.server, run.tools.map((tool) => tool.key)]), [
    ["chrome-devtools", ["1"]],
    ["chrome-devtools", ["3"]],
    [null, ["4"]],
    ["github", ["5"]],
  ]);
});
