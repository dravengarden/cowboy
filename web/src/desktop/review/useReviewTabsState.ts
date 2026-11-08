import { useCallback, useEffect, useState } from "react";
import {
  type DesktopReviewTabs,
  loadReviewTabs,
  NO_REVIEW_TABS,
  saveReviewTabs,
} from "./desktopReviewTabs";

/** The open files of one reviewed context, remembered on this device. */
export function useReviewTabsState(
  context: string | undefined,
): [DesktopReviewTabs, (change: (tabs: DesktopReviewTabs) => DesktopReviewTabs) => void] {
  const [state, setState] = useState<{ context: string | undefined; tabs: DesktopReviewTabs }>(() => ({
    context,
    tabs: context ? loadReviewTabs(context) : NO_REVIEW_TABS,
  }));
  const tabs = state.context === context
    ? state.tabs
    : context ? loadReviewTabs(context) : NO_REVIEW_TABS;
  useEffect(() => {
    if (state.context !== context) {
      setState({ context, tabs: context ? loadReviewTabs(context) : NO_REVIEW_TABS });
    }
  }, [context, state.context]);
  const update = useCallback((change: (tabs: DesktopReviewTabs) => DesktopReviewTabs): void => {
    setState((previous) => {
      const base = previous.context === context
        ? previous.tabs
        : context ? loadReviewTabs(context) : NO_REVIEW_TABS;
      const next = change(base);
      if (next !== base && context) saveReviewTabs(context, next);
      return { context, tabs: next };
    });
  }, [context]);
  return [tabs, update];
}

/** The width of `element`, followed as it resizes (0 before layout). */
export function useElementWidth(element: HTMLElement | null): number {
  const [width, setWidth] = useState(0);
  useEffect(() => {
    if (!element) return undefined;
    setWidth(element.getBoundingClientRect().width);
    const observer = new ResizeObserver((entries) => {
      const box = entries[0]?.contentRect;
      if (box) setWidth(box.width);
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, [element]);
  return width;
}
