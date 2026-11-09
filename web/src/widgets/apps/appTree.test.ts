import { describe, expect, test } from "bun:test";
import { AppBudgetError, applyAppOps, emptyAppTree, isText } from "./appTree";

const ops = (...list: unknown[]): string => JSON.stringify(list);

describe("app tree", () => {
  test("builds, updates and removes nodes in order", () => {
    let tree = applyAppOps(
      emptyAppTree(),
      ops(
        ["create", 1, "Stack", { gap: "sm" }],
        ["create", 2, "Text", {}],
        ["text", 3, "hello"],
        ["append", 2, 3],
        ["append", 1, 2],
        ["append", 0, 1],
      ),
    );
    expect(tree.root).toEqual([1]);
    const text = tree.nodes.get(3);
    expect(isText(text) && text.text).toBe("hello");
    tree = applyAppOps(tree, ops(["create", 4, "Divider", {}], ["before", 1, 4, 2]));
    const stack = tree.nodes.get(1);
    expect(stack && !isText(stack) && stack.children).toEqual([4, 2]);
    tree = applyAppOps(tree, ops(["remove", 2]));
    expect(tree.nodes.has(3)).toBe(false);
  });

  test("a first-mount clear keeps the instances created before it", () => {
    const tree = applyAppOps(
      emptyAppTree(),
      ops(["create", 1, "Stack", {}], ["clear"], ["append", 0, 1]),
    );
    expect(tree.root).toEqual([1]);
    expect(tree.nodes.has(1)).toBe(true);
  });

  test("unknown components and malformed batches stop the app", () => {
    expect(() => applyAppOps(emptyAppTree(), ops(["create", 1, "script", {}]))).toThrow(
      AppBudgetError,
    );
    expect(() => applyAppOps(emptyAppTree(), "not json")).toThrow(AppBudgetError);
    expect(() => applyAppOps(emptyAppTree(), ops(["append", 9, 1]))).toThrow(AppBudgetError);
  });

  test("table cells count toward the expansion budget", () => {
    const rows = Array.from({ length: 200 }, () => Array.from({ length: 12 }, () => "x"));
    let tree = emptyAppTree();
    tree = applyAppOps(tree, ops(["create", 1, "Table", { columns: [], rows }], ["append", 0, 1]));
    expect(tree.units).toBe(2401);
    tree = applyAppOps(tree, ops(["create", 2, "Table", { columns: [], rows }], ["append", 0, 2]));
    expect(() =>
      applyAppOps(tree, ops(["create", 3, "Table", { columns: [], rows }], ["append", 0, 3])),
    ).toThrow("the app drew too much at once");
  });
});
