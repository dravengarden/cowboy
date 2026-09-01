import { assertEquals } from "jsr:@std/assert";
import { providerAuthenticationCompleted } from "./providerAuthenticationFlow.ts";

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
