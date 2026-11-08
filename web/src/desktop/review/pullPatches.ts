// The patches of the pull request files Desktop Review has listed, in memory
// only: Code bodies and patches never enter session state or browser
// persistence (docs/remote-pr-review.md). A PR tab whose patch is gone (a
// reload) asks to be reopened from the Pull request tab.

import type { RemoteReviewBinding, RemoteReviewFile } from "../../review/remoteReviewModel";

export interface PullFileRef {
  readonly binding: RemoteReviewBinding;
  readonly revision: string;
  readonly file: RemoteReviewFile;
  /** The files of the page it was listed on, in order, for `]f`/`[f`. */
  readonly siblings: readonly string[];
}

const LIMIT = 200;
const patches = new Map<string, PullFileRef>();

export function pullFileKey(repositoryId: string, number: string, revision: string, path: string): string {
  return `${repositoryId}\0${number}\0${revision}\0${path}`;
}

export function rememberPullFiles(
  binding: RemoteReviewBinding,
  revision: string,
  files: readonly RemoteReviewFile[],
): void {
  const siblings = files.map((file) => file.path);
  for (const file of files) {
    const key = pullFileKey(binding.repositoryId, binding.number, revision, file.path);
    patches.delete(key);
    patches.set(key, { binding, revision, file, siblings });
  }
  while (patches.size > LIMIT) patches.delete(patches.keys().next().value!);
}

export function pullFile(
  repositoryId: string,
  number: string,
  revision: string,
  path: string,
): PullFileRef | undefined {
  return patches.get(pullFileKey(repositoryId, number, revision, path));
}
