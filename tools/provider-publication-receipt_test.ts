import { resolveImmutableReceipt } from "./provider-publication-receipt.ts";

const identity = {
  schema_version: 1,
  provider_id: "codex",
  provider_version: "1.0.0",
  package_digest: `sha256:${"1".repeat(64)}`,
  artifact_digest: `sha256:${"2".repeat(64)}`,
  publisher: "cowboy-first-party",
  catalog_package: "/catalog/codex.cowboy-provider",
  catalog_release: "/catalog/codex.release.json",
  published_urls: ["https://cowboy.example/provider-artifacts/digest/codex"],
};

Deno.test("publication receipt retries preserve the original timestamp", () => {
  const first = resolveImmutableReceipt(
    identity,
    undefined,
    new Date("2026-08-15T04:00:00.000Z"),
  );
  const retried = resolveImmutableReceipt(
    identity,
    first.text,
    new Date("2026-08-16T04:00:00.000Z"),
  );
  if (retried.text !== first.text) {
    throw new Error("an identical retry changed the immutable receipt");
  }
  if (retried.receipt.published_at !== "2026-08-15T04:00:00.000Z") {
    throw new Error("an identical retry changed published_at");
  }
});

Deno.test("publication receipt retries reject changed release identity", () => {
  const first = resolveImmutableReceipt(identity, undefined);
  let rejected = false;
  try {
    resolveImmutableReceipt(
      { ...identity, artifact_digest: `sha256:${"3".repeat(64)}` },
      first.text,
    );
  } catch {
    rejected = true;
  }
  if (!rejected) throw new Error("changed release identity was accepted");
});

Deno.test("publication receipt retries reject invalid timestamps", () => {
  const first = resolveImmutableReceipt(identity, undefined);
  const invalid = first.text.replace(first.receipt.published_at, "not-a-date");
  let rejected = false;
  try {
    resolveImmutableReceipt(identity, invalid);
  } catch {
    rejected = true;
  }
  if (!rejected) throw new Error("invalid receipt timestamp was accepted");
});
