import { safeExternalUrl, shouldRouteExternalClick } from "./openExternal";

Deno.test("external links allow explicit network and contact protocols", () => {
  for (const url of [
    "https://example.com/docs?q=1",
    "http://127.0.0.1:4160/health",
    "mailto:reader@example.com",
    "tel:+15551234567",
  ]) {
    if (safeExternalUrl(url) !== new URL(url).href) {
      throw new Error(`expected allowed external URL: ${url}`);
    }
  }
});

Deno.test("external links reject executable and local protocols", () => {
  for (const url of [
    "javascript:alert(1)",
    "data:text/html,<script>alert(1)</script>",
    "file:///etc/passwd",
    "custom-handler:payload",
  ]) {
    if (safeExternalUrl(url) !== null) {
      throw new Error(`expected rejected external URL: ${url}`);
    }
  }
});

Deno.test("external links reject relative and malformed values", () => {
  for (const url of ["/relative", "notes/chapter-1", "not a URL", "http://["]) {
    if (safeExternalUrl(url) !== null) {
      throw new Error(`expected invalid external URL: ${url}`);
    }
  }
});

Deno.test("plain primary link clicks route through the platform opener", () => {
  const click = {
    button: 0,
    defaultPrevented: false,
    altKey: false,
    ctrlKey: false,
    metaKey: false,
    shiftKey: false,
  };
  if (!shouldRouteExternalClick(click)) {
    throw new Error("expected a plain primary click to use the external opener");
  }
  for (const changed of [
    { button: 1 },
    { defaultPrevented: true },
    { altKey: true },
    { ctrlKey: true },
    { metaKey: true },
    { shiftKey: true },
  ]) {
    if (shouldRouteExternalClick({ ...click, ...changed })) {
      throw new Error(`expected native click semantics for ${JSON.stringify(changed)}`);
    }
  }
});
