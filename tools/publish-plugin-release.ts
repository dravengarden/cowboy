import { resolveImmutableReceipt } from "./plugin-publication-receipt.ts";
import {
  copyImmutable,
  copyImmutableText,
  exists,
  sha256,
} from "./immutable-publication.ts";

interface PluginRelease {
  release_schema: number;
  plugin_id: string;
  plugin_version: string;
  package_digest: string;
  artifact_digest: string;
  artifact_url: string;
  publisher: string;
  host_bundle_digest?: string;
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

const pluginId = Deno.args[0] ?? "";
const catalogRoot = Deno.args[1] ?? "";
const publicKeyPath = Deno.args[2] ?? "";
if (!/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(pluginId)) {
  throw new Error("Plugin id must use lowercase kebab-case");
}
if (!catalogRoot.startsWith("/")) {
  throw new Error("Catalog root must be absolute");
}

const sourceRoot = `dist/plugins/${pluginId}`;
const packagePath = `${sourceRoot}/${pluginId}.cowboy-plugin`;
const releasePath = `${sourceRoot}/${pluginId}.release.json`;
const release = JSON.parse(
  await Deno.readTextFile(releasePath),
) as PluginRelease;
if (
  release.plugin_id !== pluginId ||
  ![1, 2].includes(release.release_schema)
) {
  throw new Error("release identity or schema mismatch");
}
if (!release.signature.trim()) throw new Error("Plugin release is unsigned");
const packageDigest = await sha256(packagePath);
if (release.package_digest !== `sha256:${packageDigest}`) {
  throw new Error("Plugin package digest mismatch");
}
if (
  (release.release_schema === 1) !==
    (release.host_bundle_digest === undefined)
) {
  throw new Error("Plugin release/host bundle schema mismatch");
}
const hostBundleSource = `${sourceRoot}/${pluginId}.hostbundle.json`;
if (release.host_bundle_digest !== undefined) {
  digestValue(release.host_bundle_digest);
  if (!await exists(hostBundleSource)) {
    throw new Error("bound Plugin host bundle is missing");
  }
  if (
    release.host_bundle_digest !== `sha256:${await sha256(hostBundleSource)}`
  ) {
    throw new Error("Plugin host bundle digest mismatch");
  }
} else if (await exists(hostBundleSource)) {
  throw new Error("Plugin host bundle is not bound by the signed release");
}

const publicKey = (await Deno.readTextFile(publicKeyPath)).trim();
if (
  !publicKey.startsWith("ssh-ed25519 ") || publicKey.includes("PRIVATE KEY")
) {
  throw new Error("Plugin publisher public key is not Ed25519");
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
const catalogStem = `${pluginId}-${release.plugin_version}-${releaseDigest}`;
const catalogPackage = `${catalogRoot}/${catalogStem}.cowboy-plugin`;
const catalogRelease = `${catalogRoot}/${catalogStem}.release.json`;
const catalogHostBundle = `${catalogRoot}/${catalogStem}.hostbundle.json`;
await copyImmutable(packagePath, catalogPackage, 0o644);
if (release.host_bundle_digest !== undefined) {
  await copyImmutable(hostBundleSource, catalogHostBundle, 0o644);
}
// The release envelope is the Catalog commit marker. Publish every byte it
// binds first so a concurrent refresh sees either the old Catalog or one
// complete immutable release, never a package/host half-transaction.
await copyImmutable(releasePath, catalogRelease, 0o644);

const receiptIdentity = {
  schema_version: 1,
  plugin_id: pluginId,
  plugin_version: release.plugin_version,
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
  if (parts.length < 3 || parts.at(-3) !== "plugin-artifacts") {
    throw new Error(
      `artifact URL is outside the Plugin publication route: ${artifactUrl}`,
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
