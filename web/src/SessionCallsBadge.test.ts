import { expect, test } from "bun:test";
import type { SessionMeta } from "./protocol";
import { runningCallsByParent } from "./SessionCallsBadge";

function session(
  id: string,
  status: SessionMeta["status"],
  parent?: string,
): SessionMeta {
  return {
    id,
    status,
    ...(parent
      ? {
        execution_binding: {
          schema: 1,
          phase: "managed_child",
          parent_session_id: parent,
        },
      }
      : {}),
  } as unknown as SessionMeta;
}

test("counts only running managed children per parent", () => {
  const sessions = [
    session("parent", "busy"),
    session("other", "running"),
    session("child-a", "busy", "parent"),
    session("child-b", "starting", "parent"),
    session("child-c", "running", "parent"),
    session("child-d", "exited", "other"),
    session("child-e", "busy", "other"),
  ];
  const counts = runningCallsByParent(sessions);
  expect(counts.get("parent")).toBe(2);
  expect(counts.get("other")).toBe(1);
  expect(counts.has("child-a")).toBe(false);
});

test("reuses one pass for the same sessions snapshot", () => {
  const sessions = [session("child", "busy", "parent")];
  expect(runningCallsByParent(sessions)).toBe(runningCallsByParent(sessions));
});
