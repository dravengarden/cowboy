/** State and commands of one account's Machine browser panel: the Controller
 * read, status polling while nothing is attached, and the lifecycle calls.
 * Everything observable lives in the pure `browserModel`. */
import { useCallback, useEffect, useReducer, useRef } from "react";
import { PREFERENCES_CHANGED } from "../aboutPreferences";
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
  modelFor,
  pollPeriod,
  readTicket,
  reduceAccountBrowser,
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
  api?: MachineBrowserApi;
}

export interface MachineBrowserController {
  model: BrowserModel;
  dispatch: (event: BrowserEvent) => void;
  refresh(): void;
  /** Open the account's ordinary tab browser; `machine` pins the open to
   * that Machine (the owner's choice for a first open, or the Machine a
   * caller names). */
  open(viewport: BrowserViewport, machine?: string): void;
  /** Navigate exactly `target` to an `https` URL. */
  navigate(url: string, target: BrowserTarget): void;
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

/** The browser identity a restart was confirmed for, and how to report
 * another one an intermediate answer names. */
export interface RestartIdentity {
  expected: string | undefined;
  observe: (identity: string) => void;
}

/** Throws (after reporting it) when `answer` names another browser
 * identity than the restart was confirmed for: nothing is reopened then. */
export function checkRestartIdentity(
  identity: RestartIdentity | undefined,
  answer: { browser_identity?: string | undefined },
): void {
  const named = answer.browser_identity;
  if (identity?.expected === undefined || named === undefined || named === identity.expected) {
    return;
  }
  identity.observe(named);
  throw new MachineBrowserError("browser_identity_changed", 409);
}

/** Hibernate exactly `target`, confirm it stopped (`confirmStopped`, with
 * reads pinned to its Machine), then reopen only on that Machine, unless
 * the panel ended (re-bound to another browser identity) meanwhile or an
 * intermediate answer names another identity than the confirmed one. */
export async function restartOn(
  api: MachineBrowserApi,
  { account, dataset }: { account: string; dataset: string },
  target: BrowserTarget,
  abort: AbortSignal,
  reopen: (abort: AbortSignal, machine: string) => Promise<BrowserResult>,
  identity?: RestartIdentity,
): Promise<BrowserResult> {
  const hibernated = await api.hibernate(account, dataset, target, abort);
  checkRestartIdentity(identity, hibernated);
  const read = async (signal: AbortSignal) => {
    const answer = await api.read(
      account,
      AbortSignal.any([signal, AbortSignal.timeout(READ_TIMEOUT_MS)]),
      target.machine_id,
    );
    checkRestartIdentity(identity, answer);
    return answer;
  };
  await confirmStopped(target, hibernated, read, abort);
  abort.throwIfAborted();
  return reopen(abort, target.machine_id);
}

/** The panel's model, bound to one account: switching accounts starts from
 * nothing, and every event is tagged with the account that produced it so a
 * late answer or viewer callback of another account is discarded
 * (`reduceAccountBrowser`). `modelRef` follows the shown model. */
function useAccountModel(account: string): {
  model: BrowserModel;
  modelRef: { current: BrowserModel };
  dispatch: (event: BrowserEvent) => void;
} {
  const [bound, send] = useReducer(reduceAccountBrowser, {
    account,
    model: INITIAL_BROWSER_MODEL,
  });
  const model = modelFor(bound, account);
  const modelRef = useRef(model);
  modelRef.current = model;
  const dispatch = useCallback(
    (event: BrowserEvent) => send({ kind: "event", account, event }),
    [account],
  );
  useEffect(() => send({ kind: "bind", account }), [account]);
  return { model, modelRef, dispatch };
}

/** Whether a reset answer changed the account inventory: a shared
 * profile's reset the Machine carried out renews its family bindings. */
export function resetChangesAccounts(identity: string | undefined, result: BrowserResult): boolean {
  return (
    identity?.startsWith("share:") === true &&
    result.state.state === "hibernated" &&
    result.state.reason === "owner_request"
  );
}

/** Reset exactly `target`; a confirmed reset of a shared profile renewed
 * its family bindings, so the account inventory and settings re-read. */
async function resetOn(
  api: MachineBrowserApi,
  { account, dataset }: { account: string; dataset: string },
  target: ResetTarget,
  abort: AbortSignal,
): Promise<BrowserResult> {
  const result = await api.reset(
    account,
    dataset,
    target.machine_id,
    target.revision,
    abort,
    target.identity,
  );
  if (resetChangesAccounts(target.identity, result)) announceAccountsChanged();
  return result;
}

/** The identity a restart is confirmed for: the one the panel shows. */
function restartIdentity(
  model: BrowserModel,
  dispatch: (event: BrowserEvent) => void,
): RestartIdentity {
  return {
    expected: model.observedIdentity ?? model.read?.browser_identity,
    observe: (identity) => dispatch({ type: "identity", identity }),
  };
}

function announceAccountsChanged(): void {
  globalThis.dispatchEvent(
    new CustomEvent(PREFERENCES_CHANGED, { detail: { provider_accounts: 0 } }),
  );
}

/** Request signals bound to the product session, a timeout and the
 * mounted panel: every request and continuation (a restart's reopen) ends
 * with the panel, so a re-bound panel never acts on its former identity. */
function usePanelSignal(): (timeoutMs: number) => AbortSignal {
  const lifetime = useRef(new AbortController());
  useEffect(() => {
    if (lifetime.current.signal.aborted) lifetime.current = new AbortController();
    const controller = lifetime.current;
    return () => controller.abort();
  }, []);
  return (timeoutMs) =>
    AbortSignal.any([
      productSessionSignal(),
      lifetime.current.signal,
      AbortSignal.timeout(timeoutMs),
    ]);
}

/** Whether an answer may still be applied: the panel is mounted and the
 * product session and principal are the ones it was opened for. */
function useLive(owner: string): () => boolean {
  const alive = useRef(false);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  return useCallback(
    () => alive.current && !productSessionSignal().aborted && productSyncPrincipal() === owner,
    [owner],
  );
}

export function useMachineBrowser({
  account,
  dataset,
  owner,
  active,
  browserEnabled,
  api = machineBrowserApi,
}: MachineBrowserInput): MachineBrowserController {
  const { model, modelRef, dispatch } = useAccountModel(account);
  const live = useLive(owner);
  const signal = usePanelSignal();

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
  }, [account, api, dispatch, live]);
  // A poll never overtakes a read still in flight (which would discard it):
  // it waits for that one, and the next poll is scheduled after it settles.
  const poll = useCallback(() => inFlight.current ?? refresh(), [refresh]);

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
    ): void => {
      if (modelRef.current.busy) return;
      dispatch({ type: "busy", action });
      call(signal(timeoutMs)).then(
        (result) => {
          if (live()) dispatch({ type: "result", action, result, now: Date.now() });
        },
        (cause: unknown) => {
          if (live()) dispatch({ type: "failed", action, message: errorText(cause) });
        },
      );
    },
    [dispatch, live],
  );
  const openCall = useCallback(
    (viewport: BrowserViewport, abort: AbortSignal, machine?: string): Promise<BrowserResult> =>
      api.open(account, dataset, viewport, abort, machine),
    [account, api, dataset],
  );
  const open = useCallback(
    (viewport: BrowserViewport, machine?: string) =>
      run("open", (abort) => openCall(viewport, abort, machine), OPEN_TIMEOUT_MS),
    [openCall, run],
  );
  const navigate = useCallback(
    (url: string, target: BrowserTarget) =>
      run(
        "navigate",
        (abort) => api.navigate(account, dataset, target, url, abort),
        COMMAND_TIMEOUT_MS,
      ),
    [account, api, dataset, run],
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
        (abort) => resetOn(api, { account, dataset }, target, abort),
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
          restartOn(
            api,
            { account, dataset },
            target,
            abort,
            (signal, machine) => openCall(viewport, signal, machine),
            restartIdentity(modelRef.current, dispatch),
          ),
        RESTART_TIMEOUT_MS,
      );
    },
    [account, api, dataset, dispatch, openCall, run],
  );

  return { model, dispatch, refresh, open, navigate, hibernate, snapshot, reset, restart };
}
