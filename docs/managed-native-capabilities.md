# Managed native capabilities

The first capability profile supports `image.generate` and `image.edit` through
the existing four Cowboy MCP tools. The calling agent decides when it needs an
image. CLI calls remain supported. Installation and production acceptance are
separate from the source contract described here.

## Ownership and discovery

Codex owns the image tool, model, authentication, native conversation and native
image history. Its signed Provider package declares `image_v1` and
`provider.managed-image.v1` (Provider SDK 3.1.14). Cowboy owns parent-scoped
admission, placement, cancellation, durable receipts and verified file delivery.
The profile is discovered from the installed package, not inferred from a
Provider's name. Unsupported Providers fail closed.

This uses native Codex turns through ACP, not another image API client or hidden
agent CLI. Claude is adapter-backed through the shared Calls/MCP contract. If
the target cannot provide the capability, report it as unavailable; do not
silently switch credentials or manufacture a successful artifact. The residual
Cowboy-owned gap is authenticated cross-Provider routing and cross-machine
delivery. Replace this adapter glue when the native/ACP surface provides those
same scoped capabilities and delivery guarantees.

```ts
return await cowboy.calls.capabilities({ capability: "image.generate" });
// Later: pass if_revision with the returned revision to avoid the catalog again.
```

Discovery reads installed state and makes no model or quota-refresh request. The
exact native runtime checks `modelProvider/capabilities/read` before an image
turn, and requires a supported ChatGPT account (the pinned native version does
not expose this tool to API-key or Free accounts). Installed support is not a
guarantee that image quota is available. The native Provider owns the final
availability/limit decision.

```json
{
  "action": "calls.start",
  "operation": "hero-image-1",
  "arguments": {
    "provider": "codex",
    "capability": "image.generate",
    "instruction": "Create a transparent watercolor cowboy hat for the header.",
    "title": "Header illustration"
  }
}
```

Inspect the receipt under domain `calls` after an uncertain submission. Read
`cowboy.calls.result(call_id)` for the terminal result, including
`result.artifacts`. An artifact includes its content digest, verified PNG size
and dimensions, HTTP reference, execution Machine id and absolute path on that
Machine. No base64 is returned to the calling model. An admitted or completed
text-only turn does not count as successful image generation. Quota failures
retain the native limit id and reset time in `native_failure`, without another
usage probe or a dump of the child transcript.

For editing, use `capability:"image.edit"` and either `files` with up to four
relative PNG/JPEG/WebP input paths or `child_session_id` from a previous image
call. The latter reuses the native conversation. A continuation cannot switch
between the image and review profiles. CLI request JSON uses the corresponding
`purpose:"image_generate"` or `"image_edit"`.

## Execution and file boundaries

Image calls snapshot only explicit input files, twice checking their bytes. They
do not snapshot the repository, inherit project configuration, or copy the
parent conversation. Input paths cannot traverse symlinks. The profile disables
hooks, plugins, MCP servers, web/browser/computer tools and nested agents.
Native code execution retains the read-only/no-network sandbox; remote execution
uses the ordinary native binding and keeper enforcement. This does not prohibit
the explicitly requested native image service call.

Only a completed native image tool event identifies a generated artifact. Cowboy
decodes and validates its complete PNG with byte, memory and pixel limits. A
text path, `savedPath`, view-image result or input echo is not a deliverable.
There are at most four outputs, each at most 8 MiB and 16 megapixels. The
adapter keeps the typed ACP image and omits the duplicate base64 in `rawOutput`;
native Codex history is unchanged.

Canonical history retains only the small provenance marker and native quota
receipt alongside artifact references. Rejected image payloads become bounded
failure markers, so a bad image cannot abort persistence for other sessions in
the same batch. Live observation and restored history use the same contract.

The Controller stores content-addressed bytes, then asks the execution Machine
to deliver them under the exact parent/child/current-round identity. The Machine
independently validates bytes and digest and writes only its private child
artifact directory. Repeated delivery verifies the same file; it never
overwrites a conflicting file. Images survive workspace refresh for continuation
and follow child cleanup. Retained call results keep Controller artifacts alive
even if transcript events are removed. Success requires verified delivery.

Both participating Machines need protocol 33 and workers that understand
`image_v1`; an older Machine is not sent the new round/delivery messages. This
requires a separate worker-generation maintenance acceptance. Once image records
exist, recovery Controllers/workers must understand the new purpose/profile; old
strict readers are not rollback targets for those sessions.

## Cost and verification

Resident MCP tool definitions do not grow. Capability schemas are loaded on
demand and revision-cached. A start sends the instruction and selected input
paths; a result sends only a short caption and file metadata. Managed Codex
calls skip ACP's additional model request for generating a session title: Cowboy
already owns their titles. Image threads use a short purpose-specific native
base instruction, and keep the signed feature restrictions in both process and
thread configuration.

Total task cost still includes the delegated native model turn, image generation
and any images the parent chooses to inspect. A small MCP definition is not a
measurement of that total. Native prompt caching and actual billed usage depend
on Provider observations, not on a synthetic tokenizer count.

The offline native gate is `tools/execution_managed_image_conformance.py` with
the exact candidate launcher and CLI in a private network/PID namespace. It uses
a scripted loopback service and no real account. Rust tests cover image parsing,
minimal snapshots, symlink refusal, exact delivery ownership and replay. These
checks do not by themselves establish live image-service availability or billed
usage. Document exact production receipts separately when activated.
