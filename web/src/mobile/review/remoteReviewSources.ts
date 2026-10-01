import type {
  ExtensionResponse,
  RepositoryRemote,
  WorkspaceExtension,
} from "../../extensions/api";
import type { RemoteReviewBinding } from "./remoteReviewModel";
type Inventory = Extract<ExtensionResponse, { type: "inventory" }>;
export type Choice = {
  extension: WorkspaceExtension;
  view: string;
  remote: RepositoryRemote;
  key: string;
};

export function choices(inventory: Inventory | undefined): Choice[] {
  return inventory?.extensions.filter((extension) => extension.available)
    .flatMap((extension) =>
      extension.views.filter((view) => view.review === "pull_request").flatMap((
        view,
      ) =>
        inventory.remotes.map((remote) => ({
          extension,
          view: view.id,
          remote,
          key: JSON.stringify([
            extension.identity.pluginId,
            view.id,
            remote.name,
          ]),
        }))
      )
    ) ?? [];
}

export function matches(choice: Choice, binding: RemoteReviewBinding): boolean {
  return choice.extension.identity.pluginId === binding.pluginId &&
    choice.view === binding.view &&
    choice.remote.host === binding.host &&
    (choice.remote.owner === binding.owner &&
        choice.remote.repository === binding.repository ||
      choice.extension.views.some((view) =>
        view.id === choice.view && view.discovery
      ));
}
