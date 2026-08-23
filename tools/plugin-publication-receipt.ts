export interface PluginPublicationReceiptIdentity {
  schema_version: number;
  plugin_id: string;
  plugin_version: string;
  package_digest: string;
  artifact_digest: string;
  publisher: string;
  catalog_package: string;
  catalog_release: string;
  published_urls: string[];
}

export interface PluginPublicationReceipt
  extends PluginPublicationReceiptIdentity {
  published_at: string;
}

export function resolveImmutableReceipt(
  identity: PluginPublicationReceiptIdentity,
  existingText?: string,
  now = new Date(),
): { receipt: PluginPublicationReceipt; text: string } {
  let publishedAt = now.toISOString();
  if (existingText !== undefined) {
    let existing: unknown;
    try {
      existing = JSON.parse(existingText);
    } catch {
      throw new Error("immutable publication receipt is not valid JSON");
    }
    if (
      typeof existing !== "object" || existing === null ||
      !("published_at" in existing) ||
      typeof existing.published_at !== "string"
    ) {
      throw new Error("immutable publication receipt has no published_at");
    }
    const parsed = new Date(existing.published_at);
    if (
      Number.isNaN(parsed.valueOf()) ||
      parsed.toISOString() !== existing.published_at
    ) {
      throw new Error("immutable publication receipt has invalid published_at");
    }
    publishedAt = existing.published_at;
  }

  const receipt = { ...identity, published_at: publishedAt };
  const text = `${JSON.stringify(receipt, null, 2)}\n`;
  if (existingText !== undefined && existingText !== text) {
    throw new Error(
      "immutable publication receipt does not match the release identity",
    );
  }
  return { receipt, text };
}
