/** Negative core workflow contracts; included in the strict Web typecheck. */
import type {
  ProviderAuthenticationFlow,
  ProviderLoginEvent,
} from "./providerAuthenticationOwner";
import { managementEntryFixture } from "./providerManagement.fixture";

const identity = {
  provider: managementEntryFixture(),
  sharedProviderNames: ["Example"],
  credentialTitle: "Fixture",
  events: [],
};
const methods: ProviderAuthenticationFlow = identity;
const request: ProviderAuthenticationFlow = {
  ...identity,
  requestId: "fixture",
  expiresAtMs: 1_999_999_999_999,
};
// @ts-expect-error request identity cannot exist without its expiry
const missingExpiry: ProviderAuthenticationFlow = {
  ...identity,
  requestId: "fixture",
};
// @ts-expect-error expiry is meaningful only for a request incarnation
const missingRequest: ProviderAuthenticationFlow = {
  ...identity,
  expiresAtMs: 1_999_999_999_999,
};
const unknownState: ProviderLoginEvent = {
  event: "login_state",
  request_id: "fixture",
  provider: "example",
  // @ts-expect-error missing runtime codec for a future state must fail closed
  state: "future_success",
};
void [methods, request, missingExpiry, missingRequest, unknownState];
