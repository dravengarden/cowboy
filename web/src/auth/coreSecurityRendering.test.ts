import { assert, assertEquals } from "jsr:@std/assert";
import * as React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { ProductLoginPage } from "./ProductLoginPage.tsx";

const sso = {
  id: "cardea",
  display_name: "Cardea",
  button_label: "External sign-in",
  start_url: "/api/auth/oidc/start",
};

function render(
  overrides: Partial<Parameters<typeof ProductLoginPage>[0]> = {},
): string {
  // Deno compiles linked packages outside web/ with classic JSX, while the
  // production Vite builder supplies the automatic runtime. Keep that test
  // adapter scoped to this synchronous render, not the installed application.
  const prior = Object.getOwnPropertyDescriptor(globalThis, "React");
  Object.defineProperty(globalThis, "React", {
    configurable: true,
    value: React,
  });
  try {
    return renderToStaticMarkup(React.createElement(ProductLoginPage, {
      setupRequired: false,
      setupPending: false,
      providers: [],
      hostPlugins: [],
      passwordEnabled: true,
      loginMethodOrder: ["password"],
      onAuthed: () => {
        throw new Error("Rendering may not authenticate");
      },
      ...overrides,
    }));
  } finally {
    if (prior) Object.defineProperty(globalThis, "React", prior);
    else Reflect.deleteProperty(globalThis, "React");
  }
}

Deno.test("password form renders synchronously with no Plugin inventory", () => {
  const html = render();
  assert(html.includes('name="username"'));
  assert(html.includes('name="password"'));
  assert(html.includes("Password"));
});

Deno.test("setup and sole-account creation cannot be replaced by the default SSO Plugin", () => {
  for (const passwordEnabled of [false, true]) {
    const policy = {
      setupRequired: true,
      providers: [sso],
      loginMethodOrder: passwordEnabled ? ["cardea", "password"] : ["cardea"],
      passwordEnabled,
    };
    const setup = render(policy);
    assert(setup.includes("Setup code"));
    assertEquals(setup.includes('name="password"'), false);
    assertEquals(setup.includes("External sign-in"), false);
    const creating = render({ ...policy, setupPending: true });
    assert(creating.includes('name="new-password"'));
    assert(creating.includes("Confirm password"));
    assertEquals(creating.includes("External sign-in"), false);
  }
});

Deno.test("Plugin labels and fields cannot alter a local password form", () => {
  const html = render({
    hostPlugins: [{
      id: "password",
      slots: ["login.method"],
      label: "untrusted-local-title",
      fields: { account: "untrusted-account", secret: "untrusted-secret" },
    }],
  });
  assert(html.includes('name="password"'));
  assertEquals(html.includes("untrusted-"), false);
});

Deno.test("ordinary disabled local authentication never renders a password fallback", () => {
  for (const providers of [[], [sso]]) {
    const html = render({
      passwordEnabled: false,
      providers,
      loginMethodOrder: ["password"],
    });
    assertEquals(html.includes('name="password"'), false);
    assertEquals(html.includes('name="new-password"'), false);
  }
});
