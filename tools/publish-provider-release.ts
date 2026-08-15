import { resolveImmutableReceipt } from "./provider-publication-receipt.ts";

interface ProviderRelease {
  release_schema: number;
  provider_id: string;
  provider_version: string;
  package_digest: string;
  artifact_digest: string;
  artifact_url: string;
  publisher: string;
  signature: string;
  runtime_artifacts: Array<{
    os: string;
    architecture: string;
    components: Array<{
      command: string;
      artifact_url: string;
      artifact_digest: string;
      artifact_format: "raw" | "tar_gz";
    }>;
  }>;
}

const providerId = Deno.args[0] ?? "";
const catalogRoot = Deno.args[1] ?? "";
const publicKeyPath = Deno.args[2] ?? "";
if (!/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(providerId)) {
  throw new Error("Provider id must use lowercase kebab-case");
}
if (!catalogRoot.startsWith("/")) {
  throw new Error("Catalog root must be absolute");
}

const sourceRoot = `dist/providers/${providerId}`;
const packagePath = `${sourceRoot}/${providerId}.cowboy-provider`;
const releasePath = `${sourceRoot}/${providerId}.release.json`;
const release = JSON.parse(
  await Deno.readTextFile(releasePath),
) as ProviderRelease;
if (release.provider_id !== providerId || release.release_schema !== 2) {
  throw new Error("release identity or schema mismatch");
}
if (!release.signature.trim()) throw new Error("Provider release is unsigned");
const packageDigest = await sha256(packagePath);
if (release.package_digest !== `sha256:${packageDigest}`) {
  throw new Error("Provider package digest mismatch");
}

const publicKey = (await Deno.readTextFile(publicKeyPath)).trim();
if (
  !publicKey.startsWith("ssh-ed25519 ") || publicKey.includes("PRIVATE KEY")
) {
  throw new Error("Provider publisher public key is not Ed25519");
}
await copyImmutableText(
  publicKey,
  `${catalogRoot}/trusted-publishers/${release.publisher}.pub`,
  0o644,
);

const published: string[] = [];
const packageTarget = publicationTarget(
  release.artifact_url,
  release.package_digest,
);
await copyImmutable(
  packagePath,
  `${catalogRoot}/artifacts/${packageTarget.digest}/${packageTarget.name}`,
  0o644,
);
published.push(release.artifact_url);

for (const target of release.runtime_artifacts) {
  for (const component of target.components) {
    const extension = component.artifact_format === "tar_gz" ? ".tar.gz" : "";
    const localPath =
      `${sourceRoot}/runtime/${target.os}-${target.architecture}/${component.command}${extension}`;
    const digest = await sha256(localPath);
    if (component.artifact_digest !== `sha256:${digest}`) {
      throw new Error(
        `runtime artifact digest mismatch for ${component.command}`,
      );
    }
    const remote = publicationTarget(
      component.artifact_url,
      component.artifact_digest,
    );
    await copyImmutable(
      localPath,
      `${catalogRoot}/artifacts/${remote.digest}/${remote.name}`,
      0o644,
    );
    published.push(component.artifact_url);
  }
}

const releaseDigest = digestValue(release.artifact_digest);
const catalogStem =
  `${providerId}-${release.provider_version}-${releaseDigest}`;
const catalogPackage = `${catalogRoot}/${catalogStem}.cowboy-provider`;
const catalogRelease = `${catalogRoot}/${catalogStem}.release.json`;
await copyImmutable(packagePath, catalogPackage, 0o644);
await copyImmutable(releasePath, catalogRelease, 0o644);

const receiptIdentity = {
  schema_version: 1,
  provider_id: providerId,
  provider_version: release.provider_version,
  package_digest: release.package_digest,
  artifact_digest: release.artifact_digest,
  publisher: release.publisher,
  catalog_package: catalogPackage,
  catalog_release: catalogRelease,
  published_urls: [...new Set(published)].sort(),
};
const receiptPath = `${catalogRoot}/receipts/${catalogStem}.json`;
const existingReceipt = await exists(receiptPath)
  ? await Deno.readTextFile(receiptPath)
  : undefined;
const { receipt, text: receiptText } = resolveImmutableReceipt(
  receiptIdentity,
  existingReceipt,
);
await copyImmutableText(
  receiptText,
  receiptPath,
  0o644,
);
console.log(JSON.stringify({ ...receipt, receipt: receiptPath }));

function publicationTarget(
  artifactUrl: string,
  artifactDigest: string,
): { digest: string; name: string } {
  const url = new URL(artifactUrl);
  if (url.protocol !== "https:" || url.href.includes("latest")) {
    throw new Error(`artifact URL is not immutable HTTPS: ${artifactUrl}`);
  }
  const parts = url.pathname.split("/").filter(Boolean);
  if (parts.length < 3 || parts.at(-3) !== "provider-artifacts") {
    throw new Error(
      `artifact URL is outside the Provider publication route: ${artifactUrl}`,
    );
  }
  const digest = digestValue(artifactDigest);
  const urlDigest = parts.at(-2) ?? "";
  const name = parts.at(-1) ?? "";
  if (urlDigest !== digest || !/^[A-Za-z0-9._-]+$/.test(name)) {
    throw new Error(`artifact URL does not bind its digest: ${artifactUrl}`);
  }
  return { digest, name };
}

function digestValue(value: string): string {
  const match = /^sha256:([a-f0-9]{64})$/.exec(value);
  if (!match) throw new Error(`invalid SHA-256 digest ${value}`);
  return match[1];
}

async function copyImmutable(
  source: string,
  destination: string,
  mode: number,
): Promise<void> {
  await Deno.mkdir(destination.slice(0, destination.lastIndexOf("/")), {
    recursive: true,
  });
  if (await exists(destination)) {
    if (await sha256(source) !== await sha256(destination)) {
      throw new Error(
        `immutable publication target already has different bytes: ${destination}`,
      );
    }
    return;
  }
  const temporary = `${destination}.${Deno.pid}.partial`;
  await Deno.copyFile(source, temporary);
  await Deno.chmod(temporary, mode);
  await Deno.rename(temporary, destination).catch(async (error) => {
    if (!(error instanceof Deno.errors.AlreadyExists)) throw error;
    await Deno.remove(temporary);
    if (await sha256(source) !== await sha256(destination)) {
      throw new Error(
        `publication race changed immutable target: ${destination}`,
      );
    }
  });
}

async function copyImmutableText(
  value: string,
  destination: string,
  mode = 0o644,
): Promise<void> {
  await Deno.mkdir(destination.slice(0, destination.lastIndexOf("/")), {
    recursive: true,
  });
  if (await exists(destination)) {
    if (await Deno.readTextFile(destination) !== value) {
      throw new Error(
        `immutable publication target already has different text: ${destination}`,
      );
    }
    return;
  }
  const temporary = `${destination}.${Deno.pid}.partial`;
  await Deno.writeTextFile(temporary, value, { mode });
  await Deno.rename(temporary, destination).catch(async (error) => {
    if (!(error instanceof Deno.errors.AlreadyExists)) throw error;
    await Deno.remove(temporary);
    if (await Deno.readTextFile(destination) !== value) {
      throw new Error(
        `publication race changed immutable target: ${destination}`,
      );
    }
  });
}

async function sha256(path: string): Promise<string> {
  const output = await new Deno.Command("sha256sum", {
    args: [path],
    clearEnv: true,
  }).output();
  if (!output.success) throw new Error(`sha256sum failed for ${path}`);
  return new TextDecoder().decode(output.stdout).trim().split(/\s+/)[0]
    .toLowerCase();
}

async function exists(path: string): Promise<boolean> {
  try {
    await Deno.lstat(path);
    return true;
  } catch (error) {
    if (error instanceof Deno.errors.NotFound) return false;
    throw error;
  }
}
