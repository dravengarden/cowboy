# Tool results across Providers

Cowboy uses the same result model in the transcript, Desktop inspector and
Mobile tool sheet. MCP identity comes from the qualified tool name or the
explicit ACP server/tool/arguments envelope, independently of the Agent
Provider. Native `name` and older Provider metadata are both accepted. New
Providers and MCP servers retain the generic arguments and result view.

## Result retention

The Controller compacts a result before live fan-out and durable reduction.
When ACP `content` already carries a result, duplicate `rawOutput` is removed.
When content is absent, null or an empty array, the raw result is moved once
into a `content` block of type `tool_result` with the original value under
`value`. The existing `formatted_output` file/terminal projection remains a
literal `raw_output` block. Images inside results still pass through artifact
externalization. Token streams and tool completions remain incremental;
opening tool details does not refetch the transcript.

This preserves Codex ACP's opaque MCP `{result, error}` response, MCP
`structuredContent`, and raw-only responses from any Provider. The reducer
applies the same normalization to replayed completions before merging them.
The client also accepts legacy uncompacted events already in browser storage.
No database migration or Provider runtime change is required.

Old records whose only result was discarded by the previous compactor cannot
be reconstructed. They say **No result was recorded for this call**. This is
different from an explicit empty response, pending execution, interruption or
failure. A completed transport with MCP `isError: true` is displayed as failed.

## Presentation

- Cowboy help names its topic and renders type definitions as TypeScript.
  Actions expose their exact action, arguments and operation identity. CodeAct
  reads/executions show the program with language highlighting and copy.
- Matrix search/read/write/forget/receipt operations have specific headings.
  Search results show titles, snippets and revision references. Zero matches
  are explicit. Structured and text copies of the same result paint once.
- Text, structured JSON, images, embedded resources, resource references and
  audio are readable without knowing the Provider. Unknown result types fall
  back to JSON. Content is displayed as data, never evaluated as a program.
- Formatted and Raw remain available for MCP calls even when a Provider tags
  them as `search`. Raw keeps the original result envelope. Native web-search
  presentation retains its existing policy.
- Long code/data retain copy, expand and line-layout controls. Fields wrap
  within phone/tablet panes; Desktop keeps its inspector history and keyboard
  navigation. Touch controls have at least a 44px target where adapted here.

The unit fixtures cover transport shapes, explicit empty values, errors,
identity parsing and live/replayed completion folding. The `tool-inspector`
browser suite uses the real product theme in light/dark modes and exercises
help, programs, receipts, memory results, Raw switching and 320–1100px panes.
Browser emulation does not establish physical iOS acceptance.
