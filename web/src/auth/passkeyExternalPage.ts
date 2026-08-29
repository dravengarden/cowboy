import { externalPasskeyApi } from "./authApi";
import {
  assertPasskey,
  createPasskey,
  passkeysSupported,
} from "./passkeyBrowser";
import { passkeyErrorMessage } from "./passkeyFlow";

const title = document.querySelector<HTMLElement>("#title");
const status = document.querySelector<HTMLElement>("#status");
const error = document.querySelector<HTMLElement>("#error");

function setCopy(nextTitle: string, nextStatus: string): void {
  if (title) title.textContent = nextTitle;
  if (status) status.textContent = nextStatus;
}

function showError(message: string): void {
  setCopy("Passkey was not completed", "Return to Cowboy and try again.");
  if (!error) return;
  error.textContent = message;
  error.dataset.visible = "true";
}

async function run(): Promise<void> {
  const fragment = new URLSearchParams(location.hash.slice(1));
  const transactionId = fragment.get("transaction") ?? "";
  history.replaceState(null, "", location.pathname);
  if (!/^[a-f0-9]{64}$/.test(transactionId)) {
    showError("This Passkey request is invalid or has expired.");
    return;
  }
  if (!passkeysSupported()) {
    await externalPasskeyApi.fail(transactionId).catch(() => undefined);
    showError("System Passkeys are unavailable in this browser.");
    return;
  }
  try {
    const options = await externalPasskeyApi.options(transactionId);
    if (options.status === "complete") {
      setCopy("Passkey verified", "Returning to Cowboy…");
      return;
    }
    if (options.status === "failed") {
      showError("This Passkey request was cancelled.");
      return;
    }
    setCopy(
      options.action === "register" ? "Create a Passkey" : "Verify your Passkey",
      "Use Face ID, Touch ID, or your device passcode in the system prompt.",
    );
    const credential = options.action === "register"
      ? await createPasskey(options)
      : await assertPasskey(options);
    await externalPasskeyApi.complete(transactionId, credential);
    setCopy("Passkey verified", "Returning to Cowboy…");
  } catch (reason) {
    await externalPasskeyApi.fail(transactionId).catch(() => undefined);
    showError(passkeyErrorMessage(reason, "Passkey verification failed."));
  }
}

void run();
