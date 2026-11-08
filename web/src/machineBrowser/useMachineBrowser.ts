/** State and commands of one account's Machine browser panel: the Controller
 * read, status polling while nothing is attached, and the lifecycle calls.
 * Everything observable lives in the pure `browserModel`. */
import { useCallback, useEffect, useReducer, useRef } from "react";
import { lowPriorityPoll } from "../network/bandwidth";
import { productSessionSignal } from "../productSessionEnd";
import { productSyncPrincipal } from "../productSyncIdentity";
import {
  type BrowserResult,
  type BrowserTarget,
  type BrowserViewport,
  MachineBrowserError,
  type MachineBrowserApi,
  browserErrorText,
  machineBrowserApi,
} from "./api";
import {
  type BrowserAction,
  type BrowserEvent,
  type BrowserModel,
  INITIAL_BROWSER_MODEL,
  RESTART_CONFIRM_ATTEMPTS,
  RESTART_CONFIRM_INTERVAL_MS,
  confirmStopped,
  pollPeriod,
  readTicket,
  reduceBrowser,
  resetTarget,
  shownTarget,
} from "./browserModel";
import type { ResetTarget } from "./confirmation";

export interface MachineBrowserInput {
  account: string;
  dataset: string;
  owner: string;
  /** The panel is open; reads and polling run only then. */
  active: boolean;
  browserEnabled: boolean;
  /** Opens bind to this native login instead of a plain tab. */
  signInRequest?: string | undefined;
  api?: MachineBrowserApi;
}

export interface MachineBrowserController {
  model: BrowserModel;
  dispatch: (event: BrowserEvent) => void;
  refresh(): void;
  /** Open (or, for a sign-in, re-validate) the browser; `takeover` mounts
   * the viewer over another attachment when the open answers attached, and
   * only over `expected` when the owner confirmed that browser. */
  open(viewport: BrowserViewport, takeover?: boolean, expected?: BrowserTarget): void;
  /** Instance commands name `target`, the browser a confirmation captured,
   * else the one shown now (`shownTarget`). */
  hibernate(target?: BrowserTarget): void;
  snapshot(): void;
  reset(target?: ResetTarget): void;
  /** Hibernate, confirm that exact browser stopped, then reopen it at
   * another device scale on the same Machine. */
  restart(viewport: BrowserViewport, target?: BrowserTarget): void;
}

const READ_TIMEOUT_MS = 15_000;
/** The Controller itself waits up to 150 s for a cold start. */
const OPEN_TIMEOUT_MS = 180_000;
const COMMAND_TIMEOUT_MS = 60_000;
const RESTART_TIMEOUT_MS =
  OPEN_TIMEOUT_MS +
  COMMAND_TIMEOUT_MS +
  RESTART_CONFIRM_ATTEMPTS * (RESTART_CONFIRM_INTERVAL_MS + READ_TIMEOUT_MS);

function errorText(cause: unknown): string {
  if (cause instanceof MachineBrowserError) return cause.message;
  if (cause instanceof DOMException && cause.name === "TimeoutError") {
    return "The Machine did not answer in time. Refresh to see the current state.";
  }
  return browserErrorText(undefined);
}

/** Hibernate exactly `target`, confirm it stopped (`confirmStopped`, with
 * reads pinned to its Machine), then reopen only on that Machine. */
async function restartOn(
  api: MachineBrowserApi,
  { account, dataset }: { account: string; dataset: string },
  target: BrowserTarget,
  abort: AbortSignal,
  reopen: (abort: AbortSignal, machine: string) => Promise<BrowserResult>,
): Promise<BrowserResult> {
  const hibernated = await api.hibernate(account, dataset, target, abort);
  const read = (signal: AbortSignal) =>
    api.read(
      account,
      AbortSignal.any([signal, AbortSignal.timeout(READ_TIMEOUT_MS)]),
      target.machine_id,
    );
  await confirmStopped(target, hibernated, read, abort);
  return reopen(abort, target.machine_id);
}

export function useMachineBrowser({
  account,
  dataset,
  owner,
  active,
  browserEnabled,
  signInRequest,
  api = machineBrowserApi,
}: MachineBrowserInput): MachineBrowserController {
  const [model, dispatch] = useReducer(reduceBrowser, INITIAL_BROWSER_MODEL);
  const alive = useRef(false);
  const modelRef = useRef(model);
  modelRef.current = model;
  const live = useCallback(
    () => alive.current && !productSessionSignal().aborted && productSyncPrincipal() === owner,
    [owner],
  );
  const signal = (timeoutMs: number): AbortSignal =>
    AbortSignal.any([productSessionSignal(), AbortSignal.timeout(timeoutMs)]);

  // Each read is tagged with its generation, pinned Machine and lifecycle
  // epoch (`readTicket`): a superseded, re-pinned or pre-transition answer
  // is discarded.
  const readGeneration = useRef(0);
  const inFlight = useRef<Promise<void> | undefined>(undefined);
  const refresh = useCallback((): Promise<void> => {
    const ticket = readTicket(readGeneration, () => modelRef.current);
    const current = (): boolean => live() && ticket.current();
    const issued = ticket.epoch;
    const done = api.read(account, signal(READ_TIMEOUT_MS), ticket.pinned).then(
      (read) => {
        if (current()) dispatch({ type: "read", read, now: Date.now(), issued });
      },
      (cause: unknown) => {
        if (current()) dispatch({ type: "read_failed", message: errorText(cause), issued });
      },
    );
    inFlight.current = done;
    void done.finally(() => {
      if (inFlight.current === done) inFlight.current = undefined;
    });
    return done;
  }, [account, api, live]);
  // A poll never overtakes a read still in flight (which would discard it):
  // it waits for that one, and the next poll is scheduled after it settles.
  const poll = useCallback(() => inFlight.current ?? refresh(), [refresh]);

  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  useEffect(() => {
    if (active) refresh();
  }, [active, browserEnabled, refresh]);
  // Status polling: 5 s idle, 2 s while starting (bounded), none while a
  // stream is attached and reports state itself.
  const period = active ? pollPeriod(model, Date.now()) : undefined;
  useEffect(() => {
    if (period === undefined) return undefined;
    return lowPriorityPoll(poll, period);
  }, [period, poll]);

  const run = useCallback(
    (
      action: BrowserAction,
      call: (signal: AbortSignal) => Promise<BrowserResult>,
      timeoutMs: number,
      takeover = false,
      expected?: BrowserTarget,
    ): void => {
      if (modelRef.current.busy) return;
      dispatch({ type: "busy", action });
      call(signal(timeoutMs)).then(
        (result) => {
          if (live()) {
            dispatch({ type: "result", action, result, now: Date.now(), takeover, expected });
          }
        },
        (cause: unknown) => {
          if (live()) dispatch({ type: "failed", action, message: errorText(cause) });
        },
      );
    },
    [live],
  );
  const openCall = useCallback(
    (viewport: BrowserViewport, abort: AbortSignal, machine?: string): Promise<BrowserResult> =>
      signInRequest
        ? api.openSignIn(account, signInRequest, dataset, viewport, abort, machine)
        : api.open(account, dataset, viewport, abort, machine),
    [account, api, dataset, signInRequest],
  );
  const open = useCallback(
    (viewport: BrowserViewport, takeover = false, expected?: BrowserTarget) =>
      run("open", (abort) => openCall(viewport, abort), OPEN_TIMEOUT_MS, takeover, expected),
    [openCall, run],
  );
  // Every command names the Machine and instance the panel shows, or the
  // one a confirmation captured.
  const hibernate = useCallback(
    (captured?: BrowserTarget) => {
      const target = captured ?? shownTarget(modelRef.current);
      if (!target) return;
      run(
        "hibernate",
        (abort) => api.hibernate(account, dataset, target, abort),
        COMMAND_TIMEOUT_MS,
      );
    },
    [account, api, dataset, run],
  );
  const snapshot = useCallback(() => {
    const target = shownTarget(modelRef.current);
    if (!target) return;
    run("snapshot", (abort) => api.snapshot(account, dataset, target, abort), COMMAND_TIMEOUT_MS);
  }, [account, api, dataset, run]);
  const reset = useCallback(
    (captured?: ResetTarget) => {
      const target = captured ?? resetTarget(modelRef.current.read);
      if (!target) return;
      run(
        "reset",
        (abort) => api.reset(account, dataset, target.machine_id, target.revision, abort),
        COMMAND_TIMEOUT_MS,
      );
    },
    [account, api, dataset, run],
  );
  // The restart reopens only on the captured Machine (`restartOn`).
  const restart = useCallback(
    (viewport: BrowserViewport, captured?: BrowserTarget) => {
      const target = captured ?? shownTarget(modelRef.current);
      if (!target) return;
      run(
        "open",
        (abort) =>
          restartOn(api, { account, dataset }, target, abort, (signal, machine) =>
            openCall(viewport, signal, machine),
          ),
        RESTART_TIMEOUT_MS,
      );
    },
    [account, api, dataset, openCall, run],
  );

  return { model, dispatch, refresh, open, hibernate, snapshot, reset, restart };
}
