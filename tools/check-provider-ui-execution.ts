/** Run after the independent SDK package build/validator. This keeps ordinary
 * Web unit tests independent of generated dist files and never invokes a host.
 */
import {
  type SurfaceSlot,
  validateProviderUiManifest,
} from "@cowboy/provider-ui";
import { createProviderUiOwner } from "../web/src/providerUiOwner.ts";

let providers = 0;
let surfaces = 0;
for (const path of Deno.args) {
  const artifact = JSON.parse(await Deno.readTextFile(path));
  if (artifact.payload?.kind !== "agent_provider") continue;
  const manifest: unknown = artifact.payload.contract.manifest;
  validateProviderUiManifest(manifest);
  for (const slot of Object.keys(manifest.ui.surfaces) as SurfaceSlot[]) {
    const owner = createProviderUiOwner(manifest, slot);
    try {
      if (owner.snapshot().problem) {
        throw new Error(
          `unsupported UI execution profile: ${manifest.id}/${slot}`,
        );
      }
      surfaces++;
    } finally {
      await owner.dispose();
    }
  }
  providers++;
}
if (providers === 0) throw new Error("no compiled Provider packages checked");
console.log(
  `UI executor profiles valid: ${providers} Providers, ${surfaces} surfaces; no host effects dispatched`,
);
