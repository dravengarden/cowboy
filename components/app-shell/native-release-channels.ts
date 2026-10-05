import type { NativeAppPlatform } from "./native-app-version.ts";

export type NativeReleaseChannelKind = "sidestore" | "app_store";

export interface NativeReleaseChannel {
  readonly kind: NativeReleaseChannelKind;
  readonly url: string;
  readonly label?: string;
  /** Shells this channel installs on. SideStore and the App Store channel
   *  serve the iOS/iPadOS app unless a manifest says otherwise. */
  readonly platforms?: readonly NativeAppPlatform[];
}

/** The channels that can update the shell running on `platform`. */
export function nativeReleaseChannelsFor(
  channels: readonly NativeReleaseChannel[],
  platform: NativeAppPlatform,
): NativeReleaseChannel[] {
  return channels.filter((channel) =>
    (channel.platforms ?? ["ios"]).includes(platform)
  );
}

export function validChannelPlatforms(value: unknown): boolean {
  return value === undefined ||
    (Array.isArray(value) &&
      value.every((platform: unknown) =>
        platform === "ios" || platform === "macos" || platform === "other"
      ));
}
