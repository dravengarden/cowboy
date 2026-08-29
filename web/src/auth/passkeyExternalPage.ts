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
    setCopy("Passkey verified", "Returning to Cowboy…");
    if (actions) actions.hidden = true;
  } catch (reason) {
    if (passkeyFlowCancelled(reason)) {
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
      setCopy("Passkey request cancelled", "Return to Cowboy when ready.");
      if (actions) actions.hidden = true;
    }).catch(() => {
      showRetryableError("Could not cancel this Passkey request.");
    });
  });
  globalThis.addEventListener("pagehide", () => {
    if (terminal) return;
    const body = new Blob(
      [JSON.stringify({ transaction_id: transactionId })],
      { type: "application/json" },
    );
    navigator.sendBeacon?.("/api/auth/passkeys/external/fail", body);
  });
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
      setCopy("Passkey verified", "Returning to Cowboy…");
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
    if (actions) actions.hidden = false;
  } catch (reason) {
    await externalPasskeyApi.fail(transactionId).catch(() => undefined);
    showTerminalError(
      passkeyErrorMessage(reason, "Passkey verification failed."),
    );
  }
}

void run();
