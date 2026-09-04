/** Write a host.json + ui sidecar bound to a signed .cowboy-plugin digest. */

const NAMESPACE = "cowboy-plugin-hostbundle-v1";
const encoder = new TextEncoder();

function usage(): never {
  console.error(
    "usage: write-plugin-host-bundle.ts <plugin-dir> <output.hostbundle.json> [package-digest] [--sign <private-key>]",
  );
  Deno.exit(2);
}

function isSafePath(path: string): boolean {
  return path === "host.json" ||
    (path.startsWith("ui/") &&
      !path.includes("//") &&
      !path.split("/").some((part) => part === "" || part === ".."));
}

async function collectFiles(
  root: string,
): Promise<Record<string, string> | null> {
  const hostPath = `${root}/host.json`;
  try {
    await Deno.stat(hostPath);
  } catch {
    return null;
  }
  const files: Record<string, string> = {
    "host.json": await Deno.readTextFile(hostPath),
  };
  const uiRoot = `${root}/ui`;
  try {
    if (!(await Deno.stat(uiRoot)).isDirectory) return files;
  } catch {
    return files;
  }
  for await (const entry of Deno.readDir(uiRoot)) {
    if (!entry.isFile) continue;
    const relative = `ui/${entry.name}`;
    if (!isSafePath(relative)) {
      throw new Error(`unsafe plugin UI path ${relative}`);
    }
    if (
      !entry.name.endsWith(".js") &&
      !entry.name.endsWith(".css") &&
      !entry.name.endsWith(".json")
    ) {
      continue;
    }
    files[relative] = await Deno.readTextFile(`${uiRoot}/${entry.name}`);
  }
  return files;
}

function appendField(parts: number[], value: string): void {
  const bytes = encoder.encode(value);
  parts.push(...encoder.encode(`${bytes.length}:`), ...bytes, 10);
}

async function sha256Hex(content: string): Promise<string> {
  const hash = await crypto.subtle.digest("SHA-256", encoder.encode(content));
  return [...new Uint8Array(hash)]
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
}

async function hostBundleProof(bundle: {
  schema: string;
  plugin_id: string;
  plugin_version: string;
  package_digest: string;
  files: Record<string, string>;
}): Promise<Uint8Array> {
  const parts: number[] = [...encoder.encode(`${NAMESPACE}\n`)];
  for (const field of [
    bundle.schema,
    bundle.plugin_id,
    bundle.plugin_version,
    bundle.package_digest,
  ]) {
    appendField(parts, field);
  }
  const paths = Object.keys(bundle.files).sort();
  for (const path of paths) {
    appendField(parts, path);
    const digest = await sha256Hex(bundle.files[path]!);
    parts.push(...encoder.encode(digest), 10);
  }
  return new Uint8Array(parts);
}

async function signProof(privateKey: string, proof: Uint8Array): Promise<string> {
  const child = new Deno.Command("ssh-keygen", {
    args: ["-Y", "sign", "-f", privateKey, "-n", NAMESPACE],
    stdin: "piped",
    stdout: "piped",
    stderr: "piped",
  }).spawn();
  const writer = child.stdin.getWriter();
  await writer.write(proof);
  await writer.close();
  const output = await child.output();
  if (!output.success) {
    throw new Error(
      `ssh-keygen sign failed: ${new TextDecoder().decode(output.stderr).trim()}`,
    );
  }
  return new TextDecoder().decode(output.stdout);
}

const args = Deno.args.slice();
let signKey: string | undefined;
const signIndex = args.indexOf("--sign");
if (signIndex >= 0) {
  signKey = args[signIndex + 1];
  args.splice(signIndex, 2);
}
if (args.length < 2 || args.length > 3) usage();
if (signIndex >= 0 && !signKey) usage();

const pluginDir = args[0]!;
const output = args[1]!;

const files = await collectFiles(pluginDir);
if (!files) {
  Deno.exit(0);
}

const manifest = JSON.parse(
  await Deno.readTextFile(`${pluginDir}/plugin.json`),
) as {
  id: string;
  version: string;
};
async function packageDigest(path: string): Promise<string> {
  const bytes = await Deno.readFile(path);
  const hash = await crypto.subtle.digest("SHA-256", bytes);
  const hex = [...new Uint8Array(hash)]
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
  return `sha256:${hex}`;
}

const siblingPlugin = output.replace(/\.hostbundle\.json$/, ".cowboy-plugin");
let digest = args[2];
if (!digest) {
  try {
    digest = await packageDigest(siblingPlugin);
  } catch {
    digest = `sha256:${"0".repeat(64)}`;
  }
}
const bundle: {
  schema: string;
  plugin_id: string;
  plugin_version: string;
  package_digest: string;
  files: Record<string, string>;
  signature?: string;
} = {
  schema: "dravengarden.cowboy.plugin-hostbundle/v1",
  plugin_id: manifest.id,
  plugin_version: manifest.version,
  package_digest: digest,
  files,
};
if (signKey) {
  bundle.signature = await signProof(signKey, await hostBundleProof(bundle));
}
const json = `${JSON.stringify(bundle, null, 2)}\n`;
const slash = output.lastIndexOf("/");
if (slash > 0) {
  await Deno.mkdir(output.slice(0, slash), { recursive: true });
}
await Deno.writeFile(output, encoder.encode(json));
console.log(
  `${manifest.id}\thostbundle\t${output}${signKey ? "\tsigned" : ""}`,
);
