import type { EditorView } from "@codemirror/view";
import { useCallback } from "react";
import { appendDraftText } from "../../draftStore";
import { forkAvailability, forkSession } from "../../sessionFork";
import { requestPickSession } from "../../sessionPickRequest";
import { notify, renameSession, send, trashItems, useStoreSelector } from "../../store";
import { reviewExplainPrompt, reviewExplainSessions, reviewExplainTitle } from "../../mobile/review/reviewLineSelection";
import { keepDesktopTab, updateDesktopTabs } from "../workspaceTabs";
import { delegationTask, recordDelegation } from "./reviewDelegations";
import { documentRemark, pathRemark } from "./reviewWorkbenchModel";
import type { ReviewWorkbench } from "./useReviewWorkbench";

/**
 * The fixed first message of a review fork (docs/desktop-code-review.md
 * "The agent loop"). `review.start` returns structured findings only with
 * both `task` and `base`, and a base-bound review needs `root`; anything
 * else comes back as prose, which Review shows as text and never anchors.
 */
export function reviewDelegationPrompt(task: string, path: string, remark: string, root: string | undefined): string {
  const where = root ?? "your current working directory";
  return [
    "Review this code for concrete defects with one Cowboy managed review. Call review.start exactly once with:",
    `- task: "${task}"`,
    `- base: the full SHA of \`git -C ${where} rev-parse HEAD\` (never shortened)`,
    `- root: ${root ? `"${root}"` : "your current working directory, as an absolute path"}`,
    `- files: ["${path}"]`,
    "- focus: the question and the lines below",
    "- notify: true",
    "Do not change any files. When the review ends, report its verdict and findings. A failed, partial or incomplete review is not a clean one.",
    "",
    remark,
  ].join("\n");
}

/** `␣VE` Explain and `␣VR` Review: ask a fork of the Agent about the
 *  Document's selection (or the focused path), keep the source Session's
 *  tab, and open the fork beside it. A reader has no call authority of its
 *  own; the fork does. */
export function useDelegate(review: ReviewWorkbench): (kind: "explain" | "review", path?: string) => void {
  const sessions = useStoreSelector((snapshot) => snapshot.sessions);
  const machines = useStoreSelector((snapshot) => snapshot.machines);
  return useCallback((kind, focusedPath) => {
    const source = sessions.find((session) => session.id === review.context.agentSessionId);
    if (!source || forkAvailability(source, machines) === null) {
      notify("This session cannot be asked through a fork", "info");
      return;
    }
    const document = review.document;
    const path = focusedPath ?? document?.path;
    if (!path) {
      notify("Open a file or focus one in the Navigator first", "info");
      return;
    }
    const root = review.target && !review.target.own ? review.target.cwd : undefined;
    const view: EditorView | null = review.editor.current;
    const remark = !focusedPath && view && document
      ? documentRemark(view, document, root, review.split && review.load?.state === "ready"
        ? { text: review.load.value.text, rows: review.split.unifiedRows }
        : undefined)
      : pathRemark(path, root);
    const fingerprint = review.changes.queue.find((entry) => entry.change.path === path)?.change.fingerprint;
    const task = delegationTask(Date.now(), Math.random());
    void (async () => {
      const earlier = reviewExplainSessions(sessions, source.id);
      const fork = await forkSession(source.id, { latestCompleted: true, reuse: earlier.at(-1)?.id });
      if (!earlier.some((session) => session.id === fork.session_id)) {
        renameSession(fork.session_id, reviewExplainTitle(path));
        if (earlier.length > 0) void trashItems({ sessions: earlier.map((session) => session.id) }).catch(() => undefined);
      }
      const prompt = kind === "explain"
        ? reviewExplainPrompt(remark, root, "explain")
        : reviewDelegationPrompt(task, path, remark, root);
      if (!send({ type: "submit", session_id: fork.session_id, text: prompt, cmid: `review-${crypto.randomUUID()}` })) {
        appendDraftText(fork.session_id, prompt);
      }
      if (kind === "review") {
        recordDelegation(source.id, { fork: fork.session_id, task, kind, files: [{ path, fingerprint }], atMs: Date.now() });
      }
      // The fork opens as a preview tab; keep the source so it stays beside it.
      updateDesktopTabs((state) => keepDesktopTab(state, source.id));
      requestPickSession(fork.session_id, fork.session ?? undefined);
    })().catch((reason: unknown) => {
      notify(reason instanceof Error ? reason.message : "The fork could not be created", "warning");
    });
  }, [machines, review, sessions]);
}
