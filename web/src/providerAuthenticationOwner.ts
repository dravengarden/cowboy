import type { ProviderCatalogEntry } from "@cowboy/provider-ui";
import {
  providerAuthenticationCompleted,
  providerAuthenticationPromoting,
} from "./providerAuthenticationFlow";
import {
  createProviderDialogOwner,
  expectProviderResponse,
} from "./providerDialogOwner";

const LOGIN_STATES = [
  "unsupported",
  "signed_out",
  "pending",
  "signed_in",
  "ready",
  "expired",
  "error",
] as const;
type LoginState = typeof LOGIN_STATES[number];
function loginState(value: unknown): value is LoginState {
  return LOGIN_STATES.some((state) => state === value);
}
export type ProviderLoginEvent =
  | {
    event: "login_challenge";
    request_id: string;
    provider: string;
    verification_url: string;
    user_code?: string;
    input_required?: boolean;
    input_label?: string;
    secret_input?: boolean;
    expires_at_ms: number;
  }
  | {
    event: "login_state";
    request_id: string;
    provider: string;
    state: LoginState;
    account_label?: string;
    detail?: string;
  }
  | {
    event: "command_result";
    request_id: string;
    accepted: boolean;
    detail?: string;
  };

interface AuthenticationIdentity {
  provider: ProviderCatalogEntry;
  sharedProviderNames: string[];
  credentialTitle: string;
  events: ProviderLoginEvent[];
}
export type ProviderAuthenticationFlow =
  & AuthenticationIdentity
  & (
    | { requestId?: never; expiresAtMs?: never }
    | { requestId: string; expiresAtMs: number }
  );
interface AuthenticationDialog {
  flow: ProviderAuthenticationFlow;
  input: string;
  clipboardNotice: string;
  pendingMethod: string;
}
interface Ports {
  fetch: (url: string, init?: RequestInit) => Promise<Response>;
  executor: (
    provider: string,
    method: string,
  ) => ProviderCatalogEntry | undefined;
  refresh: () => Promise<unknown>;
  closeBrowser: () => void;
  copy: (text: string) => Promise<boolean>;
}

function record(value: unknown): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error("Invalid Provider authentication response");
  }
  return value as Record<string, unknown>;
}
function timestamp(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value > 0;
}
function text(value: unknown, maximum = 16_384): value is string {
  return typeof value === "string" && value.length <= maximum;
}
function eventsFrom(
  value: unknown,
  provider: string,
  request: string,
): ProviderLoginEvent[] {
  const body = record(value);
  if (
    body.request_id !== request || !Array.isArray(body.events) ||
    body.events.length > 2048
  ) {
    throw new Error("Invalid Provider authentication response");
  }
  return body.events.map((value: unknown) => {
    const event = record(value);
    if (
      event.request_id !== request ||
      ["detail", "account_label", "input_label", "user_code"].some((key) =>
        event[key] !== undefined && !text(event[key])
      )
    ) {
      throw new Error("Invalid Provider authentication response");
    }
    if (
      event.event === "command_result" && typeof event.accepted === "boolean"
    ) {
      return {
        event: "command_result",
        request_id: request,
        accepted: event.accepted,
        ...(typeof event.detail === "string" ? { detail: event.detail } : {}),
      };
    }
    if (event.provider !== provider) {
      throw new Error("Invalid Provider authentication response");
    }
    if (event.event === "login_state" && loginState(event.state)) {
      return {
        event: "login_state",
        request_id: request,
        provider,
        state: event.state,
        ...(typeof event.account_label === "string"
          ? { account_label: event.account_label }
          : {}),
        ...(typeof event.detail === "string" ? { detail: event.detail } : {}),
      };
    }
    if (
      event.event === "login_challenge" && text(event.verification_url) &&
      timestamp(event.expires_at_ms) &&
      ["input_required", "secret_input"].every((key) =>
        event[key] === undefined || typeof event[key] === "boolean"
      )
    ) {
      return {
        event: "login_challenge",
        request_id: request,
        provider,
        verification_url: event.verification_url,
        expires_at_ms: event.expires_at_ms,
        ...(typeof event.user_code === "string"
          ? { user_code: event.user_code }
          : {}),
        ...(typeof event.input_label === "string"
          ? { input_label: event.input_label }
          : {}),
        ...(typeof event.input_required === "boolean"
          ? { input_required: event.input_required }
          : {}),
        ...(typeof event.secret_input === "boolean"
          ? { secret_input: event.secret_input }
          : {}),
      };
    }
    throw new Error("Invalid Provider authentication response");
  });
}

/** Finite, core-owned Service sign-in UI. No credential persistence, automatic
 * mutation retry, remote cancellation on unmount, or Machine login authority.
 * Timers and aborted reads remain owned until they actually settle.
 */
export function createProviderAuthenticationOwner(
  ports: Ports,
  schedule: (callback: () => void, delayMs: number) => () => void = (
    callback,
    delayMs,
  ) => {
    const timer = setTimeout(callback, delayMs);
    return () => clearTimeout(timer);
  },
) {
  const dialog = createProviderDialogOwner<
    AuthenticationDialog,
    "start" | "submit" | "back" | "cancel"
  >();
  type Lease = NonNullable<ReturnType<typeof dialog.current>>;
  let polling: { lease: Lease; stop: () => void } | undefined;
  const reading = new WeakSet<Lease>();
  const resumeRead = new WeakMap<Lease, () => void>();
  const refreshing = new WeakSet<Lease>();
  const copying = new WeakSet<Lease>();
  let clipboardTicket: object | undefined;
  const quiet = (task: Promise<unknown>): Promise<void> =>
    task.then(() => {}, () => {});
  const requestUrl = (flow: ProviderAuthenticationFlow) =>
    `/api/plugins/${encodeURIComponent(flow.provider.provider_id)}/auth/${
      encodeURIComponent(flow.requestId!)
    }`;
  const refresh = (lease: Lease): void => {
    if (!lease.active || refreshing.has(lease)) return;
    refreshing.add(lease);
    void quiet(lease.observe(async () => {
      try {
        await ports.refresh();
      } finally {
        refreshing.delete(lease);
      }
    }));
  };
  function stopPolling(): void {
    const previous = polling;
    polling = undefined;
    previous?.stop();
  }
  function methods(value: AuthenticationDialog): AuthenticationDialog {
    clipboardTicket = undefined;
    const { requestId: _id, expiresAtMs: _expiry, ...flow } = value.flow;
    return {
      flow: { ...flow, events: [] },
      input: "",
      clipboardNotice: "",
      pendingMethod: "",
    };
  }
  function startPolling(lease: Lease): void {
    stopPolling();
    if (
      !lease.active || !lease.value().flow.requestId ||
      providerAuthenticationCompleted(lease.value().flow.events)
    ) return;
    const flow = lease.value().flow;
    let stopped = false;
    let cancelTimer: (() => void) | undefined;
    let cancelDeadline: (() => void) | undefined;
    let read: AbortController | undefined;
    const seal = () => {
      stopped = true;
      if (resumeRead.get(lease) === poll) resumeRead.delete(lease);
      cancelTimer?.();
      cancelDeadline?.();
      read?.abort();
    };
    const release = lease.defer(() => {
      seal();
      lease.signal.removeEventListener("abort", seal);
    });
    lease.signal.addEventListener("abort", seal, { once: true });
    const live = () => lease.active && !stopped;
    const stop = () => {
      seal();
      void quiet(release());
    };
    polling = { lease, stop };
    const poll = () => {
      if (!live() || reading.has(lease)) return;
      reading.add(lease);
      void quiet(lease.observe(async () => {
        read = new AbortController();
        cancelDeadline = schedule(() => read?.abort(), 8_000);
        try {
          const response = await ports.fetch(requestUrl(flow), {
            signal: read.signal,
          });
          if (!live()) return;
          if (response.status === 404 || response.status === 410) {
            const detail = (await response.text()).trim();
            if (!live()) return; // response bodies are another async boundary
            stop();
            ports.closeBrowser();
            lease.update(methods);
            lease.error(
              detail ||
                "This sign-in request ended. Choose a method to try again.",
            );
            refresh(lease);
            return;
          }
          if (response.status === 401 || response.status === 403) {
            stop();
            lease.error(
              "Sign-in status is unavailable. Close this dialog and check your Cowboy access.",
            );
            return;
          }
          if (!response.ok) return;
          const body: unknown = await response.json();
          if (!live()) return;
          let events: ProviderLoginEvent[];
          try {
            events = eventsFrom(
              body,
              flow.provider.provider_id,
              flow.requestId!,
            );
          } catch {
            stop();
            lease.error(
              "Invalid Provider authentication response. Close and reopen this dialog.",
            );
            return;
          }
          const complete = providerAuthenticationCompleted(events);
          lease.update((value) => ({
            ...value,
            flow: { ...value.flow, events },
            ...(complete ? { input: "", clipboardNotice: "" } : {}),
          }));
          if (complete && live()) {
            stop();
            ports.closeBrowser();
            lease.error("");
            refresh(lease);
          }
        } catch {
          /* A single later read can recover a transient failure. */
        } finally {
          cancelDeadline?.();
          read = undefined;
          reading.delete(lease);
          // Schedule AFTER settlement: never overlap polls or reorder snapshots.
          if (live()) cancelTimer = schedule(poll, 750);
          // A failed explicit cancel may request fresh observation. Even then,
          // wait for the previous aborted read/body to ACTUALLY settle first.
          else resumeRead.get(lease)?.();
        }
      }));
    };
    resumeRead.set(lease, poll);
    poll();
  }
  function dismiss(lease = dialog.current()): void {
    if (!lease?.active) return;
    stopPolling();
    ports.closeBrowser();
    dialog.close(lease);
  }
  async function cancel(returnToMethods: boolean): Promise<void> {
    const lease = dialog.current();
    if (!lease || dialog.snapshot().busy) return;
    const flow = lease.value().flow;
    if (!flow.requestId || providerAuthenticationCompleted(flow.events)) {
      if (!returnToMethods) dismiss(lease);
      return;
    }
    if (providerAuthenticationPromoting(flow.events)) return;
    await quiet(
      lease.run(
        returnToMethods ? "back" : "cancel",
        "Could not cancel Provider sign-in",
        async () => {
          stopPolling();
          const response = await ports.fetch(requestUrl(flow), {
            method: "DELETE",
          });
          // Already-ended means no active Service request; it does not prove that
          // an external helper or an emitted effect was undone.
          if (response.status !== 404 && response.status !== 410) {
            await expectProviderResponse(
              response,
              "Could not cancel Provider sign-in",
            );
          }
          if (!lease.active) return;
          ports.closeBrowser();
          if (returnToMethods) lease.update(methods);
          else dialog.close(lease);
          refresh(lease);
        },
      ),
    );
    if (lease.active && lease.value().flow.requestId) startPolling(lease);
  }
  return {
    snapshot: dialog.snapshot,
    subscribe: dialog.subscribe,
    lifecycle: dialog.lifecycle,
    open(
      flow: Omit<
        ProviderAuthenticationFlow,
        "events" | "requestId" | "expiresAtMs"
      >,
    ): void {
      const lease = dialog.open({
        flow: { ...structuredClone(flow), events: [] },
        input: "",
        clipboardNotice: "",
        pendingMethod: "",
      });
      if (lease?.active) stopPolling();
    },
    dismiss,
    back: () => cancel(true),
    cancel: () => cancel(false),
    error: (detail: string) => dialog.current()?.error(detail),
    setInput(input: string): void {
      if (!dialog.snapshot().busy) {
        dialog.current()?.update((value) => ({ ...value, input }));
      }
    },
    async start(method: string): Promise<void> {
      const lease = dialog.current();
      if (!lease || lease.value().flow.requestId || dialog.snapshot().busy) {
        return;
      }
      await quiet(
        lease.run(
          "start",
          "Could not start Provider authentication",
          async () => {
            clipboardTicket = undefined;
            lease.update((value) => ({
              ...value,
              pendingMethod: method,
              clipboardNotice: "",
            }));
            const provider = lease.value().flow.provider;
            if (
              !provider.manifest.authentication.methods.some((value) =>
                value.id === method
              )
            ) throw new Error("Unknown Provider sign-in method");
            const executor = ports.executor(provider.provider_id, method);
            if (!executor?.artifact_digest) {
              throw new Error(
                "No online Machine has a compatible installed Provider for this sign-in method. Install or upgrade the Provider on one Machine, then try again.",
              );
            }
            const response = await ports.fetch(
              `/api/plugins/${
                encodeURIComponent(provider.provider_id)
              }/auth/start`,
              {
                method: "POST",
                headers: { "content-type": "application/json" },
                body: JSON.stringify({
                  method,
                  provider_version: executor.provider_version,
                  generation_digest: executor.artifact_digest,
                }),
              },
            );
            await expectProviderResponse(
              response,
              "Could not start Provider authentication",
            );
            const body = record(await response.json());
            if (
              !text(body.request_id, 256) || !body.request_id ||
              !timestamp(body.expires_at_ms)
            ) throw new Error("Invalid Provider authentication response");
            const requestId = body.request_id, expiresAtMs = body.expires_at_ms;
            lease.update((value) => ({
              ...value,
              flow: { ...value.flow, requestId, expiresAtMs, events: [] },
            }));
            if (lease.active) {
              startPolling(lease);
              refresh(lease);
            }
          },
        ),
      );
    },
    async submit(fallback: string): Promise<void> {
      const lease = dialog.current();
      if (!lease) return;
      const { flow, input } = lease.value();
      const challenge = flow.events.findLast((event) =>
        event.event === "login_challenge"
      );
      if (
        !flow.requestId || !input.trim() || !challenge?.input_required ||
        providerAuthenticationCompleted(flow.events) ||
        providerAuthenticationPromoting(flow.events)
      ) return;
      await quiet(lease.run("submit", fallback, async () => {
        try {
          const response = await ports.fetch(requestUrl(flow), {
            method: "POST",
            headers: { "content-type": "application/json" },
            body: JSON.stringify({ code: input.trim() }),
          });
          await expectProviderResponse(response, fallback);
          if (lease.value().flow.requestId === flow.requestId) {
            lease.update((value) => ({ ...value, input: "" }));
          }
        } catch (cause) {
          if (
            lease.value().flow.requestId === flow.requestId &&
            !providerAuthenticationCompleted(lease.value().flow.events)
          ) {
            throw cause;
          }
        }
      }));
    },
    copyCode(): void {
      const lease = dialog.current();
      if (!lease || copying.has(lease)) return;
      const flow = lease.value().flow;
      const challenge = flow.events.findLast((event) =>
        event.event === "login_challenge"
      );
      if (
        !challenge?.user_code || providerAuthenticationCompleted(flow.events) ||
        providerAuthenticationPromoting(flow.events)
      ) return;
      const code = challenge.user_code;
      const ticket = clipboardTicket = {};
      copying.add(lease);
      lease.update((value) => ({
        ...value,
        clipboardNotice: "Copying device code…",
      }));
      if (!lease.active) {
        copying.delete(lease);
        return;
      }
      void quiet(lease.observe(async () => {
        let copied = false;
        try {
          copied = await ports.copy(code);
        } catch {
          /* Show manual fallback. */
        } finally {
          copying.delete(lease);
        }
        if (
          !lease.active || clipboardTicket !== ticket ||
          lease.value().flow.requestId !== flow.requestId ||
          providerAuthenticationCompleted(lease.value().flow.events) ||
          providerAuthenticationPromoting(lease.value().flow.events) ||
          lease.value().flow.events.findLast((event) =>
              event.event === "login_challenge"
            )?.user_code !== code
        ) return;
        lease.update((value) => ({
          ...value,
          clipboardNotice: copied
            ? `Device code ${code} copied. Paste it on the Provider page if it is not filled automatically.`
            : `Could not copy the device code automatically. Close the browser, then tap Copy ${code}.`,
        }));
      }));
    },
    dispose(): Promise<void> {
      stopPolling();
      // No native close or DELETE on React teardown: these are observations.
      return dialog.dispose();
    },
  };
}
export type ProviderAuthenticationOwner = ReturnType<
  typeof createProviderAuthenticationOwner
>;
