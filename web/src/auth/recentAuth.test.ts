import { assertEquals, assertRejects } from "jsr:@std/assert";
import { AuthApiError, type ProductMe } from "./authApi.ts";
import { retryWithRecentProductAuth } from "./recentAuth.ts";

const verified: ProductMe = { account: "draven", role: "owner" };

Deno.test("recent-auth retry verifies once and repeats the protected operation", async () => {
  let operations = 0;
  let verifications = 0;
  const result = await retryWithRecentProductAuth(
    () => {
      operations += 1;
      if (operations === 1) {
        throw new AuthApiError(
          "Recent login or Passkey verification required",
          428,
        );
      }
      return Promise.resolve("created");
    },
    () => {
      verifications += 1;
      return Promise.resolve(verified);
    },
  );
  assertEquals(result, "created");
  assertEquals(operations, 2);
  assertEquals(verifications, 1);
});

Deno.test("recent-auth retry does not intercept unrelated failures", async () => {
  let verifications = 0;
  await assertRejects(
    () =>
      retryWithRecentProductAuth(
        () => Promise.reject(new AuthApiError("invalid credentials", 401)),
        () => {
          verifications += 1;
          return Promise.resolve(verified);
        },
      ),
    AuthApiError,
    "invalid credentials",
  );
  assertEquals(verifications, 0);
});

Deno.test("recent-auth retry never repeats the operation when verification fails", async () => {
  let operations = 0;
  await assertRejects(
    () =>
      retryWithRecentProductAuth(
        () => {
          operations += 1;
          return Promise.reject(
            new AuthApiError(
              "Recent login or Passkey verification required",
              428,
            ),
          );
        },
        () => Promise.reject(new DOMException("Cancelled", "AbortError")),
      ),
    DOMException,
    "Cancelled",
  );
  assertEquals(operations, 1);
});
