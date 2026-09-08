import {
  PROVIDER_PACKAGE_SCHEMA_VERSION,
  validateProviderManifest,
  validateTelemetryBackendContract,
} from "./src/index.ts";

function record(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? value as Record<string, unknown>
    : null;
}

if (Deno.args.length === 0) {
  throw new Error("usage: validate-packages.ts <package.cowboy-plugin>...");
}

for (const path of Deno.args) {
  const plugin = record(JSON.parse(await Deno.readTextFile(path)));
  if (!plugin || plugin.package_schema !== 1) {
    throw new Error(`${path}: unsupported Plugin package envelope`);
  }
  const payload = record(plugin.payload);
  if (payload?.kind === "telemetry_backend") {
    const contract = validateTelemetryBackendContract(payload.contract);
    const manifest = record(plugin.manifest);
    if (
      manifest?.kind !== "telemetry_backend" || manifest.id !== contract.id ||
      manifest.version !== contract.version ||
      Object.keys(payload).some((key) => !["kind", "contract"].includes(key))
    ) {
      throw new Error(`${path}: telemetry payload binding mismatch`);
    }
    console.log(
      `${contract.id}\t${contract.version}\tTypeScript telemetry contract verified`,
    );
    continue;
  }
  if (payload?.kind !== "agent_provider") continue;
  const envelope = record(payload.contract);
  if (
    !envelope || envelope.package_schema !== PROVIDER_PACKAGE_SCHEMA_VERSION
  ) {
    throw new Error(`${path}: unsupported Agent Provider payload`);
  }
  if (
    typeof envelope.contract_fingerprint !== "string" ||
    !/^sha256:[0-9a-f]{64}$/i.test(envelope.contract_fingerprint)
  ) {
    throw new Error(`${path}: invalid Provider contract fingerprint`);
  }
  validateProviderManifest(envelope.manifest);
  const manifest = envelope.manifest as { id: string; version: string };
  console.log(
    `${manifest.id}\t${manifest.version}\tTypeScript contract verified`,
  );
}
