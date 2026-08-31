import {
  externalPasskeyApi,
  type ExternalPasskeyBrowserState,
} from "./authApi";
import {
  assertPasskey,
  createPasskey,
  passkeysSupported,
} from "./passkeyBrowser";
import { passkeyErrorMessage, passkeyFlowCancelled } from "./passkeyFlow";

const title = document.querySelector<HTMLElement>("#title");
const status = document.querySelector<HTMLElement>("#status");
const error = document.querySelector<HTMLElement>("#error");
const actions = document.querySelector<HTMLElement>("#actions");
const continueButton = document.querySelector<HTMLButtonElement>("#continue");
const cancelButton = document.querySelector<HTMLButtonElement>("#cancel");

let ceremony: Extract<ExternalPasskeyBrowserState, { status: "ready" }> | null =
  null;
let transactionId = "";
let busy = false;
let terminal = false;
let nativeCallback = false;

type NativeCallbackStatus = "complete" | "cancelled" | "failed";

function finishNative(status: NativeCallbackStatus): boolean {
  if (!nativeCallback) return false;
  const callback = new URL("cowboy-passkey://complete");
  callback.searchParams.set("status", status);
  location.replace(callback.href);
  return true;
}

function setCopy(nextTitle: string, nextStatus: string): void {
  if (title) title.textContent = nextTitle;
  if (status) status.textContent = nextStatus;
}

function setBusy(next: boolean): void {
  busy = next;
  if (continueButton) continueButton.disabled = next;
  if (cancelButton) cancelButton.disabled = next;
}

function clearError(): void {
  if (!error) return;
  error.textContent = "";
  delete error.dataset.visible;
}

function showRetryableError(message: string): void {
  setCopy(
    "Passkey was not completed",
    "No Passkey was created. Tap Try Again or cancel this request.",
  );
  if (error) {
    error.textContent = message;
    error.dataset.visible = "true";
  }
  if (continueButton) continueButton.textContent = "Try Again";
  if (actions) actions.hidden = false;
  setBusy(false);
}

function showTerminalError(message: string): void {
  terminal = true;
  setCopy("Passkey was not completed", "Return to Cowboy and try again.");
  if (error) {
    error.textContent = message;
    error.dataset.visible = "true";
  }
  if (actions) actions.hidden = true;
  setBusy(false);
  finishNative("failed");
}

async function performPasskey(): Promise<void> {
  if (!ceremony || busy || terminal) return;
  setBusy(true);
  clearError();
  setCopy(
    ceremony.action === "register" ? "Create a Passkey" : "Verify your Passkey",
    "Use Face ID, Touch ID, or your device passcode in the system prompt.",
  );
  try {
    const credential = ceremony.action === "register"
      ? await createPasskey(ceremony)
      : await assertPasskey(ceremony);
    await externalPasskeyApi.complete(transactionId, credential);
    terminal = true;
    if (!finishNative("complete")) {
      setCopy(
        ceremony.action === "register" ? "Passkey created" : "Passkey verified",
        "Tap Done to return to Cowboy. Your account will update automatically.",
      );
    }
    if (actions) actions.hidden = true;
  } catch (reason) {
    if (passkeyFlowCancelled(reason)) {
      // Assertions do not require a second gesture. In the native shell they
      // start as soon as this fixed-origin page loads, so cancelling Face ID /
      // Touch ID must close the authentication session instead of exposing an
      // intermediate browser retry page.
      if (nativeCallback && ceremony.action === "assert") {
        terminal = true;
        finishNative("cancelled");
        return;
      }
      showRetryableError(
        passkeyErrorMessage(reason, "Passkey verification failed."),
      );
      return;
    }
    await externalPasskeyApi.fail(transactionId).catch(() => undefined);
    showTerminalError(
      passkeyErrorMessage(reason, "Passkey verification failed."),
    );
  }
}

async function run(): Promise<void> {
  const fragment = new URLSearchParams(location.hash.slice(1));
  nativeCallback = fragment.get("callback") === "native";
  transactionId = fragment.get("transaction") ?? "";
  if (!/^[a-f0-9]{64}$/.test(transactionId)) {
    showTerminalError("This Passkey request is invalid or has expired.");
    return;
  }
  continueButton?.addEventListener("click", () => void performPasskey());
  cancelButton?.addEventListener("click", () => {
    if (busy || terminal) return;
    setBusy(true);
    void externalPasskeyApi.fail(transactionId).then(() => {
      terminal = true;
      if (!finishNative("cancelled")) {
        setCopy("Passkey request cancelled", "Return to Cowboy when ready.");
      }
      if (actions) actions.hidden = true;
    }).catch(() => {
      showRetryableError("Could not cancel this Passkey request.");
    });
  });
  // `pagehide` is not cancellation: the system Passkey UI and a fast native
  // sheet dismissal can hide this page while its completion POST is in flight.
  // Only the explicit Cancel action above may fail the server ceremony.
  history.replaceState(null, "", location.pathname);
  if (!passkeysSupported()) {
    await externalPasskeyApi.fail(transactionId).catch(() => undefined);
    showTerminalError("System Passkeys are unavailable in this browser.");
    return;
  }
  try {
    const options = await externalPasskeyApi.options(transactionId);
    if (options.status === "complete") {
      terminal = true;
      if (!finishNative("complete")) {
        setCopy(
          "Passkey is ready",
          "Tap Done to return to Cowboy. Your account will update automatically.",
        );
      }
      return;
    }
    if (options.status === "failed") {
      showTerminalError("This Passkey request was cancelled.");
      return;
    }
    ceremony = options;
    setCopy(
      options.action === "register"
        ? "Create a Passkey"
        : "Verify your Passkey",
      "Tap Continue, then use Face ID, Touch ID, or your device passcode.",
    );
    if (continueButton) {
      continueButton.textContent = options.action === "register"
        ? "Create Passkey"
        : "Verify Passkey";
    }
    if (nativeCallback && options.action === "assert") {
      if (actions) actions.hidden = true;
      await performPasskey();
      return;
    }
    if (actions) actions.hidden = false;
  } catch (reason) {
    await externalPasskeyApi.fail(transactionId).catch(() => undefined);
    showTerminalError(
      passkeyErrorMessage(reason, "Passkey verification failed."),
    );
  }
}

void run();
