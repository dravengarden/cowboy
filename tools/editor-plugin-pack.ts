/** Pack an editor plugin directory (manifest.json + main.js) into one
 * installable `.cowboy-plugin` file. The host validates the same format, so a
 * package this tool writes is exactly what Settings → Editor extensions →
 * Install plugin accepts.
 *
 *   bun tools/editor-plugin-pack.ts \
 *     examples/editor-plugins/text-tools text-tools-1.0.0.cowboy-plugin
 */
import { readFile, writeFile } from "node:fs/promises";
import {
  EDITOR_PLUGIN_PACKAGE_FORMAT,
  editorPluginDigest,
  editorPluginIncompatibility,
  parseEditorPluginManifest,
  parseEditorPluginPackage,
} from "../web/src/editorPlugins/manifest.ts";

const [directory, output] = process.argv.slice(2);
if (!directory || !output) {
  console.error(
    "usage: editor-plugin-pack.ts <plugin-directory> <output.cowboy-plugin>",
  );
  process.exit(2);
}
const manifest = parseEditorPluginManifest(
  JSON.parse(await readFile(`${directory}/manifest.json`, "utf8")),
);
const incompatible = editorPluginIncompatibility(manifest);
if (incompatible) throw new Error(`manifest ${incompatible}`);
const main = await readFile(`${directory}/main.js`, "utf8");
// Fail at pack time on syntax errors instead of at install time.
new Function("definePlugin", main);
const text = `${
  JSON.stringify(
    {
      format: EDITOR_PLUGIN_PACKAGE_FORMAT,
      manifest,
      main,
      digest: await editorPluginDigest(manifest, main),
    },
    null,
    2,
  )
}\n`;
const verified = await parseEditorPluginPackage(text);
await writeFile(output, text);
console.log(JSON.stringify({
  id: verified.manifest.id,
  version: verified.manifest.version,
  digest: verified.digest,
  output,
}));
