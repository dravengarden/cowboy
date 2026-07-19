# Cowboy shell formatter WASM

This is a project-owned ABI bridge around the core
`mvdan.cc/sh/v3/syntax` parser and printer. It deliberately does not depend on
an npm parser wrapper. The browser loads the generated module only when a shell
command detail is opened; formatting is display-only and failure falls back to
the original command.

Rebuild and verify from the repository's Nix development shell:

```sh
just shellfmt-wasm
```

`shellfmt.wasm` and its matching Go `wasm_exec.js` runtime are committed so the
normal offline frontend build does not need to download Go modules.
