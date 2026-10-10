import { test } from "bun:test";
import { assertEquals, assertRejects } from "@std/assert";
import { AuthApiError, type ProductMe } from "./authApi.ts";
import {
  type RecentProductAuthOptions,
  retryWithRecentProductAuth,
} from "./recentAuth.ts";

const verified: ProductMe = { account: "draven", role: "owner" };

test("server session ceremony overrides recent-auth preference and preserves continuation", async () => {
  for (const kind of ["primary", "passkey"] as const) {
    let calls = 0;
    let received: RecentProductAuthOptions | undefined;
    await retryWithRecentProductAuth(() => {
      if (++calls === 1) {
        throw new AuthApiError(
          "",
          428,
          "session_reauthentication_required",
          kind,
        );
      }
      return Promise.resolve();
    }, (options) => {
      received = options;
      return Promise.resolve(verified);
    }, {
      purpose: "recent",
      resumeLabel: "Continue",
      resumeWithUserGesture: true,
    });
    assertEquals(received, {
      purpose: kind,
      resumeLabel: "Continue",
      resumeWithUserGesture: true,
    });
    assertEquals(calls, 2);
  }
});

test("recent-auth retry verifies once and repeats the protected operation", async () => {
  let operations = 0;
  let verifications = 0;
  let resumeLabel: string | undefined;
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
    (options) => {
      verifications += 1;
      resumeLabel = options?.resumeLabel;
      return Promise.resolve(verified);
    },
    {
      resumeLabel: "Continue to Passkey",
      resumeWithUserGesture: true,
    },
  );
  assertEquals(result, "created");
  assertEquals(operations, 2);
  assertEquals(verifications, 1);
  assertEquals(resumeLabel, "Continue to Passkey");
});

test("recent-auth retry does not intercept unrelated failures", async () => {
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

test("recent-auth retry never repeats the operation when verification fails", async () => {
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
