import { useState } from "react";
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { BrowserProductTheme } from "../browserProductTheme";
import { ExternalSignInButton } from "./ExternalSignInButton";
import { LoginMethodFallback } from "./ProductLoginPage";
import { restoreSubscriptionSignIn, SUBSCRIPTION_SIGN_IN_RETURN } from "../subscriptionSignInLink";

function check(value: unknown, label: string): asserts value {
  if (!value) throw new Error(label);
}
export async function runSignInBrowserConformance(): Promise<string[]> {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const results: string[] = [];
  let parentBusy = false;
  let error: string | null = null;
  let timeout: (() => void) | undefined;
  const originalTimeout = globalThis.setTimeout;
  const originalUrl = location.href;
  const originalReturn = sessionStorage.getItem(SUBSCRIPTION_SIGN_IN_RETURN);
  const handoff = "#subscription-sign-in/fd376b94-c493-4d27-81da-67a1d154ca17/provider-login-d1c970fe-2420-4d83-ab57-01ed5325a30e";
  // Exercise the actual slow-navigation recovery without a 20-second fixture.
  globalThis.setTimeout =
    ((handler: TimerHandler, delay?: number, ...args: unknown[]) => {
      if (delay === 20_000 && typeof handler === "function") {
        timeout = handler as () => void;
        return originalTimeout(() => {}, 60_000);
      }
      return originalTimeout(handler, delay, ...args);
    }) as typeof setTimeout;
  function Fixture() {
    const [busy, setBusy] = useState(false);
    return (
      <div onClick={(event) => event.preventDefault()}><ExternalSignInButton
        href="#sign-in"
        busy={busy}
        onRedirectBusy={(value) => {
          parentBusy = value;
          setBusy(value);
        }}
        onRedirectError={(value) => {
          error = value;
        }}
      >
        Continue with Cardea
      </ExternalSignInButton></div>
    );
  }
  const button = () => {
    const node = container.querySelector<HTMLElement>("a, button");
    check(node, "sign-in button missing");
    return node;
  };
  try {
    flushSync(() =>
      root.render(
        <BrowserProductTheme>
          <Fixture />
        </BrowserProductTheme>,
      )
    );
    const link = button();
    check(
      link.getAttribute("href") === "#sign-in",
      "ordinary redirect href lost",
    );
    history.replaceState(history.state, "", `/${handoff}`);
    flushSync(() => link.click());
    check(sessionStorage.getItem(SUBSCRIPTION_SIGN_IN_RETURN) === handoff, "product redirect lost the subscription reference");
    history.replaceState(history.state, "", "/");
    restoreSubscriptionSignIn();
    check(location.hash === handoff && !sessionStorage.getItem(SUBSCRIPTION_SIGN_IN_RETURN), "product callback failed to restore the subscription once");
    results.push("full-page product login returns to the original subscription sign-in");
    check(parentBusy, "redirect failed to lock method selection");
    check(
      button().getAttribute("aria-busy") === "true",
      "redirect failed to announce busy",
    );
    check(
      container.querySelector('[role="progressbar"]'),
      "redirect spinner missing",
    );
    check(
      container.textContent?.includes("Opening sign-in…"),
      "redirect status missing",
    );
    const duplicate = new MouseEvent("click", {
      bubbles: true,
      cancelable: true,
    });
    flushSync(() => link.dispatchEvent(duplicate));
    check(duplicate.defaultPrevented, "duplicate redirect not blocked");
    results.push("redirect shows spinner/status and blocks duplicate taps");
    flushSync(() =>
      globalThis.dispatchEvent(
        new PageTransitionEvent("pageshow", { persisted: true }),
      )
    );
    check(
      !parentBusy && button().getAttribute("aria-busy") === "false",
      "back navigation stuck busy",
    );
    results.push("Safari back/forward restoration unlocks sign-in");
    flushSync(() => button().click());
    check(timeout, "recovery timer missing");
    flushSync(() => timeout?.());
    check(
      !parentBusy && error !== null,
      "slow redirect failed to expose retry",
    );
    check(
      button().getAttribute("aria-busy") === "false",
      "slow redirect stayed locked",
    );
    results.push("slow navigation reports a recoverable error");
    flushSync(() =>
      root.render(
        <BrowserProductTheme>
          <LoginMethodFallback
            context={{
              kind: "oidc",
              buttonLabel: "Continue with Cardea",
              startUrl: "/unused",
              native: true,
              busy: true,
              onStart: () => {},
              onCancel: () => {},
            }}
          />
        </BrowserProductTheme>,
      )
    );
    check(
      container.querySelector('[role="progressbar"]'),
      "native approval spinner missing",
    );
    check(
      container.textContent?.includes("Waiting for approval…"),
      "native approval status missing",
    );
    check(
      container.textContent?.includes("Cancel"),
      "native approval cancellation missing",
    );
    results.push("native approval shows progress and retains cancellation");
    return results;
  } finally {
    history.replaceState(history.state, "", originalUrl);
    if (originalReturn === null) sessionStorage.removeItem(SUBSCRIPTION_SIGN_IN_RETURN);
    else sessionStorage.setItem(SUBSCRIPTION_SIGN_IN_RETURN, originalReturn);
    globalThis.setTimeout = originalTimeout;
    flushSync(() => root.unmount());
    container.remove();
  }
}
