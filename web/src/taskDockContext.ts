import type { SheetDockContext } from "@cowboy/app-shell";
import type { TaskSummary } from "./backgroundTasks";
import { elapsedLabel } from "./managedCalls";

export function taskDockContext(task: TaskSummary | null, now = Date.now()): SheetDockContext {
  if (!task) return { title: "Background tasks" };
  const call = task.call;
  return {
    title: task.title,
    status: task.group === "failed" ? "Failed" : task.group === "active" ? "Doing" : task.chip,
    meta: task.health ?? (call ? elapsedLabel(call.started_at_ms ?? call.created_at_ms, call.finished_at_ms ?? now) : undefined),
    tone: task.group === "failed" ? "error" : task.group === "needs_you" || task.health ? "warning" : task.group === "done" ? "success" : "neutral",
  };
}
