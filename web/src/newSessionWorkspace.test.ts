import { assertEquals } from "jsr:@std/assert";
import { defaultNewSessionWorkspace } from "./newSessionWorkspace.ts";

Deno.test("new sessions prefer Columbus regardless of workspace ordering", () => {
  const choices = [
    { value: "cowboy", label: "cowboy", help: "/home/draven/columbus/projects/cowboy" },
    { value: "columbus", label: "columbus", help: "/home/draven/columbus" },
  ];

  assertEquals(defaultNewSessionWorkspace(choices)?.value, "columbus");
});

Deno.test("new session workspace falls back to the first available choice", () => {
  const choices = [
    { value: "remote-root", label: "Remote", help: "/srv/work" },
    { value: "other", label: "Other", help: "/srv/other" },
  ];

  assertEquals(defaultNewSessionWorkspace(choices)?.value, "remote-root");
});
