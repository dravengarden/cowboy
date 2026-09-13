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
    allowCredentials: [{
      type: "public-key",
      id: "cred",
      transports: ["usb"],
    }],
  }, desktopTab);
  assertEquals(shaped.allowCredentials, [{
    type: "public-key",
    id: "cred",
    transports: ["usb", "internal", "hybrid"],
  }]);
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
