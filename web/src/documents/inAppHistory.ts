import { useEffect, useState } from "react";

// The Draft page's back/forward controls (Obsidian's navigation capsule).
// Use the Navigation API so each arrow is enabled only when an entry of this
// app exists in that direction: `history.back()` alone could leave the app for
// the native shell's loader page or a prior site.
interface NavigationLike extends EventTarget {
  readonly canGoBack: boolean;
  readonly canGoForward: boolean;
  back(): { finished: Promise<unknown> };
  forward(): { finished: Promise<unknown> };
}

function navigationApi(): NavigationLike | undefined {
  return (globalThis as { navigation?: NavigationLike }).navigation;
}

function readHistory(): { canGoBack: boolean; canGoForward: boolean } {
  const navigation = navigationApi();
  return {
    canGoBack: navigation?.canGoBack ?? false,
    canGoForward: navigation?.canGoForward ?? false,
  };
}

export function useInAppHistory(): {
  canGoBack: boolean;
  canGoForward: boolean;
  back: () => void;
  forward: () => void;
} {
  const [state, setState] = useState(readHistory);
  useEffect(() => {
    const navigation = navigationApi();
    if (!navigation) return undefined;
    const update = (): void => setState(readHistory());
    navigation.addEventListener("currententrychange", update);
    return () => navigation.removeEventListener("currententrychange", update);
  }, []);
  return {
    ...state,
    back: () => void navigationApi()?.back().finished.catch(() => {}),
    forward: () => void navigationApi()?.forward().finished.catch(() => {}),
  };
}
