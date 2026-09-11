/** Real React/MUI integration of the production committed-owner hook. The
 * harness submits only deferred fixture effects, never a product endpoint.
 */
import { Button } from "@mui/material";
import {
  createElement,
  StrictMode,
  useEffect,
  useLayoutEffect,
  useRef,
} from "react";
import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { useProviderManagementDialogs } from "./useProviderManagementDialogs";
import {
  deferredFixture,
  managementEntryFixture,
  uninstallPlanFixture,
} from "./providerManagement.fixture";

function check(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}
async function settle() {
  await new Promise<void>((resolve) => setTimeout(resolve, 20));
}

export async function runProviderManagementBrowserConformance(): Promise<
  string[]
> {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const calls: {
    url: string;
    init: RequestInit;
    reply: ReturnType<typeof deferredFixture<Response>>;
  }[] = [];
  let committed: ReturnType<typeof useProviderManagementDialogs>["owners"];
  let mounts = 0;
  const tests: string[] = [];
  const callCount = () => calls.length;
  const ports = {
    fetch(url: string, init: RequestInit = {}) {
      const reply = deferredFixture<Response>();
      calls.push({ url, init, reply });
      return reply.promise;
    },
    executor: (id: string) => managementEntryFixture(id),
    refresh: () => Promise.resolve(),
    closeBrowser: () => {},
    copy: () => Promise.resolve(true),
  };
  function Harness() {
    const { owners, authentication, uninstall } = useProviderManagementDialogs(
      ports,
    );
    const opened = useRef(false);
    useLayoutEffect(() => {
      committed = owners;
    }, [owners]);
    useEffect(() => {
      mounts++;
    }, []);
    useEffect(() => {
      if (!owners || opened.current) return;
      opened.current = true;
      owners.authentication.open({
        provider: managementEntryFixture(),
        sharedProviderNames: ["Example"],
        credentialTitle: "Fixture",
      });
    }, [owners]);
    return createElement(
      "section",
      {},
      createElement(
        "output",
        { id: "authentication" },
        `${authentication.value?.flow.provider.provider_id ?? "closed"}:${
          authentication.value?.flow.requestId ?? "methods"
        }:${authentication.busy ?? "idle"}:${authentication.error}`,
      ),
      createElement(Button, {
        id: "start",
        disabled: Boolean(authentication.busy),
        onClick: () => void owners?.authentication.start("key"),
      }, "Start"),
      createElement(Button, {
        id: "submit",
        disabled: Boolean(authentication.busy),
        onClick: () => void owners?.authentication.submit("Submit failed"),
      }, "Submit"),
      createElement(
        "output",
        { id: "uninstall" },
        `${
          uninstall.value?.phase === "ready"
            ? uninstall.value.plan.plan_id
            : "closed"
        }:${uninstall.busy ?? "idle"}:${uninstall.error}`,
      ),
      createElement(Button, {
        id: "confirm",
        disabled: Boolean(uninstall.busy),
        onClick: () => void owners?.uninstall.confirm(),
      }, "Confirm"),
    );
  }
  const render = (key: string) =>
    flushSync(() =>
      root.render(
        createElement(StrictMode, {}, createElement(Harness, { key })),
      )
    );
  const button = (id: string) => {
    const node = container.querySelector(`#${id}`);
    check(node instanceof HTMLButtonElement, `missing ${id}`);
    return node;
  };
  const output = (id: string) =>
    container.querySelector(`#${id}`)?.textContent ?? "";
  const reply = (index: number, body: unknown, status = 200) =>
    calls[index]!.reply.resolve(Response.json(body, { status }));
  try {
    render("first");
    await settle();
    check(
      mounts >= 2 && committed?.authentication.lifecycle().phase === "active",
      "StrictMode replay left a retired owner",
    );
    check(
      output("authentication") === "example:methods:idle:",
      "committed auto-open was lost to replay",
    );
    tests.push(
      "StrictMode replay constructs a usable committed owner and retains auto-open",
    );
    button("start").click();
    button("start").click();
    await settle();
    check(
      callCount() === 1 && button("start").disabled,
      "same-stack double click duplicated sign-in",
    );
    tests.push(
      "real React/MUI same-stack start click admits one immutable request",
    );

    flushSync(() => {
      committed!.authentication.dismiss();
      committed!.authentication.open({
        provider: managementEntryFixture("other"),
        sharedProviderNames: ["Other"],
        credentialTitle: "Other",
      });
    });
    button("start").click();
    await settle();
    reply(0, { request_id: "old", expires_at_ms: 1_999_999_999_999 });
    await settle();
    check(
      output("authentication") === "other:methods:start:" && callCount() === 2,
      "old start rebound the replacement or started polling",
    );
    tests.push(
      "close/reopen fences a late start without cancelling its submitted effect",
    );
    reply(1, { request_id: "new", expires_at_ms: 1_999_999_999_999 });
    await settle();
    reply(2, {
      request_id: "new",
      events: [{
        event: "login_challenge",
        provider: "other",
        request_id: "new",
        verification_url: "https://example.invalid/login",
        input_required: true,
        secret_input: true,
        expires_at_ms: 1_999_999_999_999,
      }],
    });
    await settle();
    flushSync(() => committed!.authentication.setInput("fixture-value"));
    button("submit").click();
    button("submit").click();
    await settle();
    check(
      callCount() === 4 && button("submit").disabled,
      "duplicate input submission",
    );
    reply(3, { detail: "Fixture rejected" }, 409);
    await settle();
    check(
      output("authentication").endsWith(":idle:Fixture rejected"),
      "submit failure is not visible",
    );
    tests.push(
      "input submission is single-flight and rejection is rendered without losing input",
    );

    flushSync(() => committed!.authentication.dismiss());
    flushSync(() =>
      committed!.authentication.open({
        provider: managementEntryFixture(),
        sharedProviderNames: ["Example"],
        credentialTitle: "Fixture",
      })
    );
    button("start").click();
    await settle();
    const retired = committed!.authentication;
    render("replacement");
    await settle();
    check(
      retired.lifecycle().phase === "draining",
      "unmount falsely reported a pending write drained",
    );
    reply(4, { detail: "retired failure" }, 409);
    await settle();
    check(
      retired.lifecycle().phase === "disposed" &&
        output("authentication") === "example:methods:idle:",
      "unmount completion affected a new owner",
    );
    check(
      !calls.some((call) => call.init.method === "DELETE"),
      "teardown sent remote cancellation",
    );
    tests.push(
      "unmount/remount drains old writes without DELETE or poisoning the replacement",
    );

    let task = committed!.uninstall.prepare("machine-a", "example");
    reply(5, uninstallPlanFixture());
    await task;
    await settle();
    flushSync(() => committed!.uninstall.setConfirmActive(true));
    button("confirm").click();
    button("confirm").click();
    await settle();
    check(
      callCount() === 7 && button("confirm").disabled,
      "double confirmation escaped core admission",
    );
    flushSync(() => committed!.uninstall.close());
    task = committed!.uninstall.prepare("machine-b", "example");
    reply(7, uninstallPlanFixture("plan-b", "machine-b"));
    await task;
    await settle();
    flushSync(() => committed!.uninstall.setConfirmActive(true));
    button("confirm").click();
    await settle();
    reply(6, {});
    await settle();
    check(
      output("uninstall") === "plan-b:confirm:",
      "old success closed the next confirmation",
    );
    reply(8, { detail: "Current refusal" }, 409);
    await settle();
    check(
      output("uninstall") === "plan-b:idle:Current refusal",
      "current confirmation failure disappeared",
    );
    tests.push(
      "uninstall double click and replacement confirmation keep exact plan/error ownership",
    );
    return tests;
  } finally {
    flushSync(() => root.unmount());
    for (const call of calls) call.reply.reject(new Error("fixture closed"));
    await settle();
    container.remove();
  }
}
