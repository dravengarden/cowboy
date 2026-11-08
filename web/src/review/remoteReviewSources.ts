import {
  extensionRequest,
  type ExtensionResponse,
  type RepositoryRemote,
  resourceQuery,
  type WorkspaceExtension,
} from "../extensions/api";
import { pullNumber, type RemoteReviewBinding, type RemoteReviewPage } from "./remoteReviewModel";
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

/** One page of a pull request, checked against the requested identity,
 *  revision and repository (shared by Mobile and Desktop Review). */
export async function readRemoteReview(
  context: string,
  choice: Choice,
  number: string,
  page: number,
  signal: AbortSignal,
  repositoryId?: string,
  revision?: string,
  repository?: string,
): Promise<RemoteReviewPage> {
  const query = resourceQuery(
    choice.extension.identity,
    choice.remote.name,
    choice.view,
    "",
    page,
    number,
  );
  query.set("review", "true");
  if (repository) query.set("repository", repository);
  if (repositoryId) query.set("repositoryId", repositoryId);
  if (revision) query.set("revision", revision);
  const response = await extensionRequest(context, query, signal);
  if (
    response.type !== "review" || response.review.number !== number ||
    (repositoryId && response.review.repositoryId !== repositoryId) ||
    (revision && response.review.revision !== revision)
  ) throw new Error("The PR identity changed. Refresh or associate it again.");
  const url = new URL(response.review.url);
  if (
    url.hostname !== choice.remote.host ||
    pullNumber(
        url.href,
        repository
          ? {
            ...choice.remote,
            owner: repository.split("/")[0]!,
            repository: repository.split("/")[1]!,
          }
          : choice.remote,
      ) !== number
  ) throw new Error("The PR repository changed.");
  return response.review;
}
