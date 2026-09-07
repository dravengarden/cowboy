import { assertEquals } from "jsr:@std/assert";
import {
  providerAuthenticationCompleted,
  providerAuthenticationPromoting,
} from "./providerAuthenticationFlow.ts";

Deno.test("Provider authentication completes only on the newest durable success state", () => {
  assertEquals(
    providerAuthenticationCompleted([
      { event: "login_state", state: "pending" },
    ]),
    false,
  );
  assertEquals(
    providerAuthenticationCompleted([
      { event: "login_state", state: "pending" },
      { event: "login_state", state: "signed_in" },
    ]),
    true,
  );
  assertEquals(
    providerAuthenticationCompleted([
      { event: "login_state", state: "signed_in" },
      { event: "login_state", state: "error" },
    ]),
    false,
  );
  assertEquals(
    providerAuthenticationCompleted([
      { event: "login_state", state: "ready" },
    ]),
    true,
  );
});

Deno.test("Provider authentication distinguishes browser waiting from credential promotion", () => {
  assertEquals(
    providerAuthenticationPromoting([
      { event: "login_state", state: "pending" },
      { event: "login_challenge" },
    ]),
    false,
  );
  assertEquals(
    providerAuthenticationPromoting([
      { event: "login_state", state: "pending" },
      { event: "login_challenge" },
      {
        event: "login_state",
        state: "pending",
        detail: "login completed; promoting credentials to Cowboy Service",
      },
    ]),
    true,
  );
  assertEquals(
    providerAuthenticationPromoting([
      { event: "login_challenge" },
      { event: "login_state", state: "signed_in" },
    ]),
    false,
  );
});
