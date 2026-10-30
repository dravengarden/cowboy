import { expect, test } from "bun:test";
import {
  type ProviderLoginEvent,
  retainAuthenticationChallenge,
} from "./providerAuthenticationOwner";

const challenge: ProviderLoginEvent = {
  event: "login_challenge",
  request_id: "request",
  provider: "claude-code",
  verification_url: "https://example.com/login",
  input_required: true,
  expires_at_ms: 200,
};
test("Machine history eviction keeps an already observed authorization input", () => {
  expect(
    retainAuthenticationChallenge(
      [challenge],
      [],
      "claude-code",
      "request",
      100,
    ),
  ).toEqual([challenge]);
});
test("challenge retention cannot cross requests, providers, expiry or completion", () => {
  for (
    const [provider, request, now] of [["other", "request", 100], [
      "claude-code",
      "other",
      100,
    ], ["claude-code", "request", 200]] as const
  ) {
    expect(
      retainAuthenticationChallenge([challenge], [], provider, request, now),
    ).toEqual([]);
  }
  const completed: ProviderLoginEvent[] = [{
    event: "login_state",
    request_id: "request",
    provider: "claude-code",
    state: "signed_in",
  }];
  expect(
    retainAuthenticationChallenge(
      [challenge],
      completed,
      "claude-code",
      "request",
      100,
    ),
  ).toEqual(completed);
  const replacement = { ...challenge, expires_at_ms: 300 };
  expect(
    retainAuthenticationChallenge(
      [challenge],
      [replacement],
      "claude-code",
      "request",
      100,
    ),
  ).toEqual([replacement]);
});
