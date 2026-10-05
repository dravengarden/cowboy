# Editor plugin examples

Installable plugins for the shared Draft/Session editor. The format, API,
lifecycle and isolation model are in
[`docs/editor-plugins.md`](../../docs/editor-plugins.md).

- [`text-tools`](text-tools) — sort selected lines, insert a timestamp, count
  words, and a statistics panel; uses settings, plugin data, toolbar buttons and
  both editor permissions.

Pack and install:

```bash
just editor-plugin-pack examples/editor-plugins/text-tools text-tools-1.0.0.cowboy-plugin
```

Then open Editor extensions (Command Palette → _Editor extensions_, or the
editor's More menu) → Extensions → _Install plugin…_ and choose the file.
