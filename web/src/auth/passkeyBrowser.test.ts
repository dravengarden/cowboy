import { assertEquals } from "jsr:@std/assert";
import {
  shapePasskeyCreationPublicKey,
  shapePasskeyRequestPublicKey,
} from "./passkeyBrowser.ts";

const desktopTab = { standalone: false, mobile: false };
const desktopPwa = { standalone: true, mobile: false };
const phonePwa = { standalone: true, mobile: true };

Deno.test("desktop assertion prefers this device before hybrid QR", () => {
  const shaped = shapePasskeyRequestPublicKey({
    challenge: "challenge",
    rpId: "cowboy.example",
    userVerification: "required",
    allowCredentials: [{ type: "public-key", id: "cred" }],
  }, desktopPwa);
  assertEquals(shaped.hints, ["client-device", "hybrid"]);
  assertEquals(shaped.allowCredentials, [{
    type: "public-key",
    id: "cred",
    transports: ["internal", "hybrid"],
  }]);
});

Deno.test("missing transports are filled so Chrome tries Touch ID, not only QR", () => {
  const shaped = shapePasskeyRequestPublicKey({
    challenge: "challenge",
    allowCredentials: [{ type: "public-key", id: "cred" }],
  }, desktopTab);
  assertEquals(shaped.allowCredentials, [{
    type: "public-key",
    id: "cred",
    transports: ["internal", "hybrid"],
  }]);
});

// Widening a credential's reported transports is the client claiming a route
// the authenticator never offered, and that claim is what puts a third-party
// passkey provider in front of a credential it does not hold: the browser was
// told this one might be reachable the way that provider works.
Deno.test("a reported transport set is authoritative, never widened", () => {
  const platformOnly = shapePasskeyRequestPublicKey({
    challenge: "challenge",
    allowCredentials: [{
      type: "public-key",
      id: "cred",
      transports: ["internal"],
    }],
  }, desktopTab);
  assertEquals(platformOnly.allowCredentials, [{
    type: "public-key",
    id: "cred",
    transports: ["internal"],
  }]);
  // And with nothing reachable by hybrid, the hint does not ask for it either.
  assertEquals(platformOnly.hints, ["client-device"]);

  const securityKey = shapePasskeyRequestPublicKey({
    challenge: "challenge",
    allowCredentials: [{ type: "public-key", id: "cred", transports: ["usb"] }],
  }, desktopTab);
  assertEquals(securityKey.allowCredentials, [{
    type: "public-key",
    id: "cred",
    transports: ["usb"],
  }]);

  // A synced credential genuinely has both routes; keep both.
  const synced = shapePasskeyRequestPublicKey({
    challenge: "challenge",
    allowCredentials: [{
      type: "public-key",
      id: "cred",
      transports: ["internal", "hybrid"],
    }],
  }, desktopTab);
  assertEquals(synced.allowCredentials, [{
    type: "public-key",
    id: "cred",
    transports: ["internal", "hybrid"],
  }]);
  assertEquals(synced.hints, ["client-device", "hybrid"]);
});

Deno.test("phone assertion does not advertise hybrid QR", () => {
  const shaped = shapePasskeyRequestPublicKey({
    challenge: "challenge",
    allowCredentials: [{
      type: "public-key",
      id: "cred",
      transports: ["internal", "hybrid"],
    }],
  }, phonePwa);
  assertEquals(shaped.hints, ["client-device"]);
  assertEquals(shaped.allowCredentials, [{
    type: "public-key",
    id: "cred",
    transports: ["internal"],
  }]);
});

Deno.test("registration overrides discouraged resident keys", () => {
  const shaped = shapePasskeyCreationPublicKey({
    challenge: "challenge",
    authenticatorSelection: {
      residentKey: "discouraged",
      requireResidentKey: false,
      userVerification: "required",
    },
  }, desktopTab);
  assertEquals(shaped.hints, ["client-device", "hybrid"]);
  assertEquals(shaped.authenticatorSelection, {
    residentKey: "preferred",
    requireResidentKey: false,
    userVerification: "required",
  });
});

Deno.test("installed PWA registration asks for the platform authenticator", () => {
  const shaped = shapePasskeyCreationPublicKey({
    challenge: "challenge",
  }, desktopPwa);
  assertEquals(shaped.authenticatorSelection, {
    residentKey: "preferred",
    requireResidentKey: false,
    authenticatorAttachment: "platform",
  });
});

Deno.test("required resident keys stay required", () => {
  const shaped = shapePasskeyCreationPublicKey({
    challenge: "challenge",
    authenticatorSelection: {
      residentKey: "required",
      requireResidentKey: true,
    },
  }, desktopTab);
  assertEquals(shaped.authenticatorSelection, {
    residentKey: "required",
    requireResidentKey: true,
  });
});
