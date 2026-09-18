/** One view of an original navigation target. No path reads or handle import. */
import {
  type CapturedContent,
  capturedIdentity,
  contentRequest,
} from "../../codeBuffers/content.ts";
import type { OwnedNavigation } from "../../codeBuffers/navigation.ts";
import type { NavigationTarget } from "../../codeBuffers/navigationDestinations.ts";
import type { NavigationLocation } from "../../codeBuffers/navigationProtocol.ts";
import type { OwnedCodeBuffer } from "../../codeBuffers/owner.ts";
import { BufferClientError, requireValue } from "../../codeBuffers/protocol.ts";

/** Validate both UTF-16 endpoints against the exact complete native text. */
export function reviewDestinationRange(
  content: CapturedContent,
  location: NavigationLocation,
) {
  const identity = capturedIdentity(content);
  requireValue(
    identity.sha256 === location.content.sha256 &&
      identity.utf8Bytes === location.content.utf8Bytes,
  );
  const { position: start } = contentRequest(content, {
    kind: "hover",
    position: location.start,
  }).query;
  const { position: end } = contentRequest(content, {
    kind: "hover",
    position: location.end,
  }).query;
  requireValue(
    start.row < end.row || start.row === end.row && start.column <= end.column,
  );
  return Object.freeze({ start, end, id: 1 });
}

export function createReviewDestination(
  operation: OwnedNavigation,
  target: NavigationTarget,
) {
  // A second view cannot silently take over another view's child or its cleanup.
  if (!operation.targets().includes(target) || operation.destination(target)) {
    throw new BufferClientError("state");
  }
  const lifetime = new AbortController();
  let child: OwnedCodeBuffer | undefined;
  let attempted = false;
  let opened = false;
  let busy = false;
  let status: "idle" | "unavailable" | "mismatch" | "stale" | "ready" = "idle";
  let displayed: {
    content: CapturedContent;
    range: ReturnType<typeof reviewDestinationRange>;
  } | undefined;
  const check = () => {
    if (lifetime.signal.aborted) throw new BufferClientError("cancelled");
    if (operation.view().contextLost || child?.view().contextLost) {
      throw new BufferClientError("context_lost");
    }
  };
  const run = async (action: () => Promise<void>) => {
    check();
    if (busy) throw new BufferClientError("busy");
    busy = true;
    try {
      await action();
      check();
    } catch (error) {
      displayed = undefined;
      status = "unavailable";
      throw error;
    } finally {
      busy = false;
    }
  };
  const read = async () => {
    check();
    if (!child || !opened) throw new BufferClientError("state");
    displayed = undefined;
    const result = await child.readText(
      target.location.content,
      lifetime.signal,
    );
    check();
    if (result.kind === "complete") {
      displayed = Object.freeze({
        content: result.content,
        range: reviewDestinationRange(result.content, target.location),
      });
      status = "ready";
    } else status = result.kind;
  };
  const open = async () => {
    check();
    if (!child || opened) throw new BufferClientError("state");
    const view = child.view();
    if (
      !view.fresh || view.observation?.state !== "prepared" ||
      view.observation.pending
    ) {
      throw new BufferClientError("state");
    }
    opened = true; // one intent, including a lost or pending response
    const observation = await child.open(lifetime.signal);
    check();
    if (observation.pending || observation.state !== "open") {
      throw new BufferClientError("state");
    }
    await read();
  };
  return Object.freeze({
    view() {
      const view = child?.view();
      const ended = lifetime.signal.aborted || operation.view().contextLost ||
        !!view?.contextLost;
      const available = !ended && !busy && !!view && !view.closing &&
        !view.busy && !view.cleaning && view.fresh &&
        !view.observation?.pending;
      return {
        busy,
        ended,
        status,
        displayed: ended ? undefined : displayed,
        canOpen: available && !opened && view.observation?.state === "prepared",
        canRead: available && opened && view.observation?.state === "open",
        canInspect: !ended && !busy && !!child && !view?.busy &&
          (view?.handingOff ? operation.view().canInspect : !!view?.resourceId),
      };
    },
    start() {
      return run(async () => {
        if (attempted || operation.destination(target)) {
          throw new BufferClientError("state");
        }
        attempted = true;
        const prepared = operation.prepareDestination(target, lifetime.signal);
        // Core reserves synchronously, even when the response is lost or late.
        child = operation.destination(target);
        await prepared;
        check();
        await open();
      });
    },
    open: () => run(open),
    read: () => run(read),
    inspect: () =>
      run(async () => {
        if (!child) throw new BufferClientError("state");
        displayed = undefined;
        if (child.view().handingOff) await operation.observe(lifetime.signal);
        else await child.observe(lifetime.signal);
        status = "idle";
        // Observation never replays handoff/Open or silently starts another read.
      }),
    close() {
      lifetime.abort();
      displayed = undefined;
      return child?.close() ?? Promise.resolve({ kind: "unopened" as const });
    },
  });
}
export type ReviewDestination = ReturnType<typeof createReviewDestination>;
