/** Pack an editor plugin directory (manifest.json + main.js) into one
 * installable `.cowboy-plugin` file. The host validates the same format, so a
 * package this tool writes is exactly what Settings → Editor extensions →
 * Install plugin accepts.
 *
 *   deno run --allow-read --allow-write tools/editor-plugin-pack.ts \
 *     examples/editor-plugins/text-tools text-tools-1.0.0.cowboy-plugin
 */
import {
  EDITOR_PLUGIN_PACKAGE_FORMAT,
  editorPluginDigest,
  editorPluginIncompatibility,
  parseEditorPluginManifest,
  parseEditorPluginPackage,
} from "../web/src/editorPlugins/manifest.ts";

const [directory, output] = Deno.args;
if (!directory || !output) {
  console.error(
    "usage: editor-plugin-pack.ts <plugin-directory> <output.cowboy-plugin>",
  );
  Deno.exit(2);
}
const manifest = parseEditorPluginManifest(
  JSON.parse(await Deno.readTextFile(`${directory}/manifest.json`)),
);
const incompatible = editorPluginIncompatibility(manifest);
if (incompatible) throw new Error(`manifest ${incompatible}`);
const main = await Deno.readTextFile(`${directory}/main.js`);
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
await Deno.writeTextFile(output, text);
console.log(JSON.stringify({
  id: verified.manifest.id,
  version: verified.manifest.version,
  digest: verified.digest,
  output,
}));
