import { assert, assertEquals } from "jsr:@std/assert";
import { dirname, fromFileUrl, join } from "jsr:@std/path";

const toolsDir = dirname(fromFileUrl(import.meta.url));
const repoRoot = dirname(toolsDir);
const digest = `sha256:${"a".repeat(64)}`;

function isolatedEnv(): Record<string, string> {
  const env: Record<string, string> = {};
  for (const key of ["HOME", "PATH", "TMPDIR", "USER", "XDG_RUNTIME_DIR"]) {
    const value = Deno.env.get(key);
    if (value) env[key] = value;
  }
  return env;
}

async function writeBundle(args: string[]): Promise<string> {
  const output = await new Deno.Command("deno", {
    args: [
      "run",
      "--allow-read",
      "--allow-write",
      "--allow-run=ssh-keygen",
      join(toolsDir, "write-plugin-host-bundle.ts"),
      ...args,
    ],
    cwd: repoRoot,
    clearEnv: true,
    env: isolatedEnv(),
    stdout: "piped",
    stderr: "piped",
  }).output();
  if (!output.success) {
    throw new Error(new TextDecoder().decode(output.stderr).trim());
  }
  return new TextDecoder().decode(output.stdout);
}

Deno.test("write-plugin-host-bundle binds grok UI to a package digest", async () => {
  const root = await Deno.makeTempDir({ prefix: "cowboy-hostbundle-" });
  try {
    const output = join(root, "grok.hostbundle.json");
    const log = await writeBundle(["plugins/grok", output, digest]);
    assert(log.includes("grok\thostbundle"));
    const bundle = JSON.parse(await Deno.readTextFile(output)) as {
      schema: string;
      plugin_id: string;
      plugin_version: string;
      package_digest: string;
      files: Record<string, string>;
      signature?: string;
    };
    assertEquals(bundle.schema, "dravengarden.cowboy.plugin-hostbundle/v1");
    assertEquals(bundle.plugin_id, "grok");
    assertEquals(bundle.plugin_version, "3.1.7");
    assertEquals(bundle.package_digest, digest);
    assert(bundle.files["host.json"]?.includes("provider.usage"));
    assert(bundle.files["ui/index.js"]?.includes("GrokUsage"));
    assertEquals(bundle.signature, undefined);
  } finally {
    await Deno.remove(root, { recursive: true });
  }
});

Deno.test("write-plugin-host-bundle skips non-js host sources and can sign", async () => {
  const root = await Deno.makeTempDir({ prefix: "cowboy-hostbundle-sign-" });
  try {
    const plugin = join(root, "sample");
    await Deno.mkdir(join(plugin, "ui"), { recursive: true });
    await Deno.writeTextFile(
      join(plugin, "plugin.json"),
      JSON.stringify({ id: "sample", version: "1.0.0" }),
    );
    await Deno.writeTextFile(
      join(plugin, "host.json"),
      JSON.stringify({
        schema_version: 1,
        slots: ["provider.usage"],
        ui: { entry: "ui/index.js", host_api: "1.0.0", ui_kit: "1.0.0" },
      }),
    );
    await Deno.writeTextFile(join(plugin, "ui", "index.js"), "export default function Ui() {}");
    await Deno.writeTextFile(join(plugin, "ui", "PasskeysPanel.tsx"), "export const leak = 1;");
    const key = join(root, "publisher");
    const keygen = await new Deno.Command("ssh-keygen", {
      args: ["-t", "ed25519", "-f", key, "-N", "", "-q"],
      clearEnv: true,
      env: isolatedEnv(),
      stdout: "piped",
      stderr: "piped",
    }).output();
    if (!keygen.success) {
      throw new Error(new TextDecoder().decode(keygen.stderr).trim());
    }
    const output = join(root, "sample.hostbundle.json");
    const log = await writeBundle([plugin, output, digest, "--sign", key]);
    assert(log.includes("signed"));
    const bundle = JSON.parse(await Deno.readTextFile(output)) as {
      files: Record<string, string>;
      signature: string;
    };
    assertEquals(bundle.files["ui/PasskeysPanel.tsx"], undefined);
    assert(bundle.files["ui/index.js"]?.includes("export default"));
    assert(bundle.signature.includes("SSH SIGNATURE"));
  } finally {
    await Deno.remove(root, { recursive: true });
  }
});
