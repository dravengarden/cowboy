import {
  mcpIdentity,
  providerName,
  toolCopyText,
  toolHeading,
  toolTypeName,
  toolUsesRawOnly,
  toolVariantLabel,
} from "./presentation";

function assertEquals(actual: unknown, expected: unknown): void {
  if (actual !== expected) throw new Error(`Expected ${String(expected)}, got ${String(actual)}`);
}

Deno.test("tool presentation names the built-in agents", () => {
  assertEquals(providerName("codex"), "Codex");
  assertEquals(providerName("claude-code"), "Claude Code");
  assertEquals(providerName("grok"), "Grok Build");
});

Deno.test("MCP identity normalizes Codex fields and Claude tool names", () => {
  assertEquals(
    JSON.stringify(mcpIdentity("", {
      server: "chrome-devtools",
      tool: "evaluate_script",
      arguments: { function: "() => 1" },
    })),
    JSON.stringify({ server: "chrome-devtools", tool: "evaluate_script", arguments: { function: "() => 1" } }),
  );
  assertEquals(
    JSON.stringify(mcpIdentity("mcp__openaiDeveloperDocs__search_openai_docs", { query: "ACP" })),
    JSON.stringify({ server: "openaiDeveloperDocs", tool: "search_openai_docs", arguments: { query: "ACP" } }),
  );
  assertEquals(toolVariantLabel({
    provider: "codex",
    toolName: "",
    kind: "execute",
    title: "mcp.chrome-devtools.navigate_page",
    rawInput: { server: "chrome-devtools", tool: "navigate_page", arguments: {} },
  }), "Codex · MCP · chrome-devtools");
});

Deno.test("tool headings adapt Codex and Claude execute variants", () => {
  assertEquals(toolHeading({
    provider: "codex",
    toolName: "",
    kind: "execute",
    title: "/bin/bash -lc test",
    rawInput: { cwd: "/tmp", command: "test" },
  }), "Shell command");
  assertEquals(toolHeading({
    provider: "claude-code",
    toolName: "Bash",
    kind: "execute",
    title: "test",
    rawInput: { command: "test", description: "Verify build" },
  }), "Verify build");
  assertEquals(toolTypeName("claude-code", "Bash", "execute"), "Bash");
});

Deno.test("search headings do not repeat the full query", () => {
  assertEquals(toolHeading({
    provider: "codex",
    toolName: "",
    kind: "search",
    title: "Web search: a very long query",
    rawInput: { query: "a very long query" },
  }), "Web search");
  assertEquals(toolUsesRawOnly({ kind: "search" }), true);
  assertEquals(toolUsesRawOnly({ kind: "other", toolName: "web_search" }), true);
  assertEquals(toolUsesRawOnly({ kind: "other", toolName: "search_query" }), true);
  assertEquals(toolUsesRawOnly({ kind: "other", title: "Web search: query" }), true);
  assertEquals(toolUsesRawOnly({ kind: "execute", title: "Search local files" }), false);
});

Deno.test("tool copy uses command input and normalized output", () => {
  assertEquals(toolCopyText({
    title: "fallback",
    rawInput: { command: "just check", description: "Verify" },
    content: [{ type: "content", content: { text: "ok" } }],
  }), "just check\n\nok");
});

Deno.test("tool copy selects an MCP query as its primary content", () => {
  assertEquals(toolCopyText({
    title: "mcp.openaiDeveloperDocs.search_openai_docs",
    rawInput: { server: "openaiDeveloperDocs", tool: "search_openai_docs", arguments: { query: "ACP SDK" } },
    content: [{ type: "content", content: { text: "result" } }],
  }), "ACP SDK\n\nresult");
});

Deno.test("tool copy preserves edit diff evidence", () => {
  assertEquals(toolCopyText({
    title: "Editing files",
    rawInput: {},
    content: [{ type: "diff", oldText: "old", newText: "new" }],
  }), "Editing files\n\n--- before\nold\n+++ after\nnew");
});
