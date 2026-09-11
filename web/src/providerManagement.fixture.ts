/** Public fake identities only; never imported by the application. */
import type { ProviderCatalogEntry } from "@cowboy/provider-ui";
import { providerUiManifestFixture } from "./providerUiContract.fixture";
import type { ProviderUninstallPlan } from "./providerUninstallOwner";

export function managementEntryFixture(id = "example"): ProviderCatalogEntry {
  const manifest = providerUiManifestFixture();
  manifest.id = id;
  manifest.authentication = {
    schema_version: 1,
    required: true,
    methods: [{ id: "key", label: "API key", flow: "secret_input" }],
  };
  return {
    provider_id: id,
    provider_version: manifest.version,
    package_digest: `sha256:${"1".repeat(64)}`,
    artifact_digest: `sha256:${"2".repeat(64)}`,
    authentication_scope: `${id}-auth-v1`,
    release_state: "ready",
    publisher: "fixture",
    contract_fingerprint: `sha256:${"3".repeat(64)}`,
    supported_platforms: [],
    manifest,
  };
}
export function uninstallPlanFixture(
  plan_id = "plan-a",
  machine_id = "machine-a",
): ProviderUninstallPlan {
  return {
    plan_id,
    machine_id,
    plugin_id: "example",
    plugin_version: "1.0.0",
    generation_digest: `sha256:${"2".repeat(64)}`,
    affected_sessions: [{
      id: "session-a",
      title: "Fixture",
      status: "running",
    }],
    active_session_ids: ["session-a"],
    purge_after_ms: 1_999_999_999_999,
    expires_at_ms: 1_999_999_999_000,
    warning: "Fixture only",
  };
}
export function deferredFixture<T>() {
  let resolve!: (value: T) => void;
  let reject!: (cause: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
