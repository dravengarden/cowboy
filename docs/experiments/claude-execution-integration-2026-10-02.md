# Claude execution integration candidate

Claude Code 2.1.287 exposes the supported Mods prompt construction hooks needed
to separate its runtime context from an execution environment. The native CLI,
account authentication and conversation remain on the Agent Machine. The
existing Claude Agent Plugin owns a private SDK bridge, context Mod and bounded
file/process facade over Cowboy's authenticated worker execution endpoint.

## Release plan

- Claude Code Plugin: 3.1.38 → 3.2.0; native CLI: 2.1.286 → 2.1.287. Keep the
  tested ACP adapter 0.84.0 and its exact Agent SDK 0.3.284. Add exact `ws`
  8.22.0 to that adapter's private npm lock for authenticated loopback
  WebSockets.
- Source: `plugins/claude-code/runtime/`, the Claude Provider execution
  declaration, `components/provider-runtime/lock.json` and the adapter npm lock.
  The native upstream executable is unmodified. No dependency-owned updater or
  runtime package download is enabled.
- Shared runtime component: 1.1.10 → 1.1.11, component release 3.35.0. The
  conservative component closure requires independent patch releases of the
  other five Agent Plugins: Codex 3.2.1, Claude DeepSeek 3.1.29, Codex DeepSeek
  3.1.29, Gemini 3.1.29 and Grok 3.1.30. Only standard Claude receives the new
  execution integration; the shared native Claude pin also advances its
  separately isolated DeepSeek variant.
- Build the declared Linux x86_64 and macOS arm64 Claude artifacts, retain exact
  upstream archive integrity and private source digests, and run the full
  Plugin/Provider/Cowboy gates. Run the actual packaged Linux adapter through
  the worker/keeper conformance gate. Bind every signed candidate to the actual
  active, recovery and cold Catalog readers before publication.
- Install the exact accepted Claude release on OVH through the delegated
  Operator transaction. Retain one operation identity and inspect its receipt.
  Existing sessions keep their original generation and environment. No Machine
  or Controller activation is needed for this Provider-only implementation.

## Native behavior that the integration must preserve or avoid

`prompt.attachment`, `prompt.context` and `prompt.section` project target cwd,
Bash, OS, Git status and target guidance. They run before native request
construction, including native resume and compaction. Module failures normally
fall through in Claude, so the bridge withholds initialization success until a
native `/cost` command and repeated SDK initialize observe a unique module
registration. The probe uses no model request; an unknown custom slash command
would be unsafe because Claude may send it to the model.

Reserved native `Read`, `Edit` and `Write` aliases are insufficient. After real
compaction Claude restores file snapshots by directly rereading paths against
the runtime cwd, bypassing those aliases. `CLAUDE_CODE_DISABLE_ATTACHMENTS` does
not stop that path. The accepted design uses `ReadFile`, `EditFile`,
`WriteFile`, `GlobFiles`, `GrepFiles` and `EditNotebook` with familiar schemas.
Ordinary file operations still cost one model-visible tool call. No generated
SSH command, heredoc or transport path is needed. Native local file tools remain
disabled.

Native MCP results have a separate persistence path: large text spills and image
cache captions can point into OVH's session storage. The facade declares the
documented `anthropic/maxResultSizeChars` threshold above its own bounded
output. For images it provides the original target filename; a provenance-scoped
`session.append` Mod removes only Claude's generated cache locator from that
tool result while retaining its media and target caption. This occurs before
transcript persistence, so resume and compaction keep the same representation.
The engine-owned compaction reminder also suggests reading an OVH transcript
file. Its exact generated locator line is removed at that same supported
boundary; the actual summary and native durable history remain intact.

The SDK adapter also probes `claude auth status --json` asynchronously. That
read-only native account operation stays on the runtime and must not acquire the
sole target connection. The same applies to its native version probe. The
implementation never uses `--bare`, which disables subscription authentication.
Existing native credential projection is retained.

Read stamps and process handles persist under the exact execution binding. Stale
edits are refused; this is read-before-write protection, not an atomic
filesystem lock against unrelated processes. Cowboy retains admitted operation
identities through transport loss. The Provider never retries an uncertain
mutation as a new call or falls back to the runtime filesystem.

Native subagents, project hooks/skills, plan files, external MCP configuration
and implicit file attachments are not enabled in this first lane. The model is
told the available capabilities. Context-affecting configuration and local file
control methods are refused; model/effort selection remains available. Project
guidance is bounded and duplicate content, such as `CLAUDE.md -> AGENTS.md`, is
included once. Ancestor `AGENTS.md`, `CLAUDE.md` and `.claude/CLAUDE.md` are
literal startup snapshots. Automatic `@` imports, `.claude/rules` and nested
directory discovery are not implemented; further target guidance can be read
explicitly. PDFs need a target extraction utility. Notebook edits require cell
IDs for replacement/deletion and preserve the other notebook metadata.

Foreground Bash waits for up to two minutes by default (ten minutes when
requested), so ordinary builds do not require repeated model polling. An
explicit background call or a command that outlives that wait returns a retained
handle. Completion is observed with `TaskOutput`; it is not a native Claude
background-task notification. Interrupt cancels the foreground target command;
closing or reconnecting the native process retains background handles.

## Evidence boundary

The development worker gate has passed actual native turns, target edits,
Unicode/quotes/CRLF preservation, stale-edit rejection, target image input,
background cancellation, a lost start receipt and 35-second transport outage,
cold resume with retained process handles, real compaction, packaged ACP new and
load, live effort changes, and rejection of a broken Mod and `--bare` before
inference. It uses a loopback scripted API and disposable state. It does not
establish a signed production installation, real subscription inference,
cross-host latency or unsupported native project capabilities. Final immutable
package receipts and the deployment record must be added after their gates.

Sources:
[official Mods overview](https://code.claude.com/docs/en/plugins/mods/overview),
[prompt events](https://code.claude.com/docs/en/plugins/mods/events),
[exact-version types generated by the native CLI](https://code.claude.com/docs/en/plugins/mods/create#get-the-types-for-your-build),
[native headless authentication behavior](https://code.claude.com/docs/en/headless),
and
[MCP output/image persistence](https://code.claude.com/docs/en/mcp#mcp-output-limits-and-warnings).
