import { test } from "bun:test";
import { assertEquals } from "../../../tools/test-assert.ts";
import { placeNotice, widgetOfNotice } from "./widgetRows";

const snapshot = (stateVersion: number, title: string) => ({
  sessionUpdate: "notice",
  title,
  _cowboy: {
    widget: {
      widgetId: "deploy",
      handle: "w1",
      revision: 1,
      stateVersion,
      closed: false,
      document: { title },
    },
  },
});

test("a later snapshot replaces the widget's row in place and an older one never wins", () => {
  const items: {
    kind: string;
    key: string;
    title: string;
    widget?: ReturnType<typeof widgetOfNotice>;
  }[] = [];
  placeNotice(items, {
    kind: "notice",
    key: "10",
    title: "a",
    widget: widgetOfNotice(snapshot(1, "a")),
  });
  placeNotice(items, { kind: "notice", key: "11", title: "plain" });
  placeNotice(items, {
    kind: "notice",
    key: "20",
    title: "b",
    widget: widgetOfNotice(snapshot(3, "b")),
  });
  placeNotice(items, {
    kind: "notice",
    key: "21",
    title: "stale",
    widget: widgetOfNotice(snapshot(2, "stale")),
  });
  assertEquals(
    items.map((item) => `${item.key}:${item.title}`),
    ["10:b", "11:plain"],
  );
});

test("a notice without a well-formed snapshot is an ordinary notice", () => {
  assertEquals(widgetOfNotice({ sessionUpdate: "notice", title: "x" }), undefined);
  assertEquals(widgetOfNotice({ _cowboy: { widget: { widgetId: 1 } } }), undefined);
});
