import { describe, expect, test } from "bun:test";
import { appsEnabledFor, iosMajor, TESTED_IOS_MAJOR } from "./appsEnabled";

// Safari freezes the OS in its user agent; only `Version/` is current.
const IPHONE = (major: number): string =>
  `Mozilla/5.0 (iPhone; CPU iPhone OS 18_7 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/${major}.2 Mobile/15E148 Safari/604.1`;
const navigator = (userAgent: string, userAgentData?: unknown, maxTouchPoints = 0): Navigator =>
  ({ userAgent, userAgentData, maxTouchPoints }) as unknown as Navigator;
const handlers = { messageHandlers: {} };

describe("widget app enablement", () => {
  test("desktop Chromium of a tested version on a tested OS", () => {
    const data = (version: string, platform = "Linux", mobile = false) => ({
      brands: [{ brand: "Google Chrome", version }],
      mobile,
      platform,
    });
    expect(appsEnabledFor({ navigator: navigator("", data("151")) })).toBe(true);
    expect(appsEnabledFor({ navigator: navigator("", data("139")) })).toBe(false);
    expect(appsEnabledFor({ navigator: navigator("", data("151", "Android", true)) })).toBe(false);
  });

  test("iOS WebKit by user agent from the tested major", () => {
    expect(iosMajor(IPHONE(27))).toBe(27);
    // A home-screen web app names no Safari version: fallback.
    expect(
      iosMajor(
        "Mozilla/5.0 (iPad; CPU OS 18_7 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148",
      ),
    ).toBe(null);
    expect(appsEnabledFor({ navigator: navigator(IPHONE(TESTED_IOS_MAJOR)) })).toBe(true);
    expect(appsEnabledFor({ navigator: navigator(IPHONE(TESTED_IOS_MAJOR - 1)) })).toBe(false);
    // iPadOS Safari sends a Mac user agent; touch tells it from macOS Safari.
    const mac =
      "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/27.2 Safari/605.1.15";
    expect(appsEnabledFor({ navigator: navigator(mac, undefined, 5) })).toBe(true);
    expect(appsEnabledFor({ navigator: navigator(mac) })).toBe(false);
  });

  test("a native shell needs the hardened capability with a tested OS", () => {
    const ua = navigator(IPHONE(27));
    expect(appsEnabledFor({ webkit: handlers, navigator: ua })).toBe(false);
    expect(
      appsEnabledFor({ webkit: handlers, navigator: ua, __cowboyAppsSandbox: { v: 1, os: "27.2" } }),
    ).toBe(true);
    expect(
      appsEnabledFor({ webkit: handlers, navigator: ua, __cowboyAppsSandbox: { v: 1, os: "18.6" } }),
    ).toBe(false);
    expect(
      appsEnabledFor({ webkit: handlers, navigator: ua, __cowboyAppsSandbox: { v: 2, os: "27.2" } }),
    ).toBe(false);
    expect(appsEnabledFor({ webkit: handlers, navigator: ua, __cowboyAppsSandbox: true })).toBe(false);
  });

  test("Tauri desktop shells stay on the fallback", () => {
    const data = { brands: [{ brand: "Chromium", version: "151" }], mobile: false, platform: "Windows" };
    expect(appsEnabledFor({ __TAURI_INTERNALS__: {}, navigator: navigator("", data) })).toBe(false);
    expect(appsEnabledFor({ __TAURI_INTERNALS__: {}, webkit: handlers, navigator: navigator("") })).toBe(
      false,
    );
  });
});
