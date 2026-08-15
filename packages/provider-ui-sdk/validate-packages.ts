import {
  PROVIDER_PACKAGE_SCHEMA_VERSION,
  validateProviderManifest,
} from "./src/index.ts";

function record(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

if (Deno.args.length === 0) {
  throw new Error("usage: validate-packages.ts <package.cowboy-provider>...");
}

for (const path of Deno.args) {
  const envelope = record(JSON.parse(await Deno.readTextFile(path)));
  if (!envelope || envelope.package_schema !== PROVIDER_PACKAGE_SCHEMA_VERSION) {
    throw new Error(`${path}: unsupported Provider package envelope`);
  }
  if (
    typeof envelope.contract_fingerprint !== "string" ||
    !/^sha256:[0-9a-f]{64}$/i.test(envelope.contract_fingerprint)
  ) {
    throw new Error(`${path}: invalid Provider contract fingerprint`);
  }
  validateProviderManifest(envelope.manifest);
  const manifest = envelope.manifest as { id: string; version: string };
  console.log(`${manifest.id}\t${manifest.version}\tTypeScript contract verified`);
}
