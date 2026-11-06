import { flushSync } from "react-dom";
import { createRoot } from "react-dom/client";
import { Box, CssBaseline } from "@mui/material";
import { BrowserProductTheme } from "./browserProductTheme";
import { SessionActionBadge, SessionAttentionMenu } from "./SessionAttention";
import { receiveCallApproval } from "./callApproval";
import type { SessionMeta } from "./protocol";
import type { SessionFoldersValue } from "./sessionFolders";

const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 80));
function check(value: unknown, message: string): asserts value {
  if (!value) throw new Error(message);
}

export async function checkSessionAttention(): Promise<void> {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  let picked = "";
  const sessions: SessionMeta[] = [
    { id: "attention-1", title: "Deploy preview", provider: "codex", cwd: "/tmp", status: "busy", attention: { approvals: 2, questions: 0 } },
    { id: "attention-2", title: "Choose a destination", provider: "claude-code", cwd: "/tmp", status: "running", attention: { approvals: 0, questions: 1 } },
    { id: "attention-3", title: "Research", provider: "codex", cwd: "/tmp", status: "busy" },
  ];
  const folders: SessionFoldersValue = {
    folders: [
      { id: "parent", name: "Project", parent: null, project: null, position: 0 },
      { id: "child", name: "Nested", parent: "parent", project: null, position: 0 },
    ], placement: { "attention-1": "parent", "attention-2": "child" },
  };
  const render = (rows: SessionMeta[]) => flushSync(() => root.render(
    <BrowserProductTheme><CssBaseline /><Box sx={{ width: 320 }}>
      <SessionAttentionMenu sessions={rows} onPick={(id) => { picked = id; }} />
      <SessionAttentionMenu sessions={rows} folder="parent" folders={folders} onPick={(id) => { picked = id; }} />
      {rows.map((session) => <Box key={session.id}><SessionActionBadge session={session} /></Box>)}
    </Box></BrowserProductTheme>,
  ));
  try {
    render(sessions);
    await tick();
    check(host.querySelectorAll("[data-session-action-needed]").length === 2, "Unopened session summaries render without transcript hydration");
    const folder = host.querySelector<HTMLButtonElement>('[data-session-attention-inbox="parent"]');
    check(folder && folder.textContent?.includes("2 need action"), "Folder counts sessions once and includes nested descendants");
    folder.click();
    await tick();
    const target = [...document.querySelectorAll<HTMLElement>('[role="menuitem"]')].find((item) => item.textContent?.includes("Choose a destination"));
    check(target, "Collapsed-folder inbox exposes nested request");
    target.click();
    await tick();
    check(picked === "attention-2", "Selecting request navigates to its session");
    check(host.querySelectorAll("[data-session-action-needed]").length === 2, "Opening does not clear attention");
    receiveCallApproval("attention-3", { schema: 1, caller: "codex", reason: "calls_disabled", requests: 1, agents: ["claude-code"], items: [], ttl_ms: 350 });
    await tick();
    check(host.textContent?.includes("3 need action"), "Call approval participates in global inbox");
    receiveCallApproval("attention-3", { schema: 1, caller: "codex", reason: "calls_disabled", requests: 1, agents: ["claude-code"], items: [], ttl_ms: 500 });
    await new Promise((resolve) => setTimeout(resolve, 400));
    check(host.textContent?.includes("3 need action"), "Refresh extends a call prompt's lifetime");
    await new Promise((resolve) => setTimeout(resolve, 200));
    check(!host.textContent?.includes("3 need action"), "Expired calls leave inbox without opening their conversation");
    render(sessions.map((session) => ({ ...session, attention: { approvals: 0, questions: 0 } })));
    await tick();
    check(!host.querySelector("[data-session-attention-inbox]"), "Cross-device resolution removes all pending indicators");
  } finally {
    receiveCallApproval("attention-3", null);
    root.unmount();
    host.remove();
  }
}

export function renderSessionAttentionPreview(): void {
  const sessions: SessionMeta[] = [
    { id: "preview-1", title: "Web-sandbox research", provider: "codex", cwd: "/tmp", status: "busy", attention: { approvals: 1, questions: 0 } },
    { id: "preview-2", title: "Choose deployment target", provider: "claude-code", cwd: "/tmp", status: "busy", attention: { approvals: 0, questions: 1 } },
  ];
  const host = document.createElement("div");
  document.body.append(host);
  flushSync(() => createRoot(host).render(<BrowserProductTheme><CssBaseline />
    <Box sx={{ width: 340, p: 2, bgcolor: "background.paper" }}>
      <SessionAttentionMenu sessions={sessions} onPick={() => {}} />
      <Box sx={{ py: 1, fontWeight: 600 }}>Marketplace-signup</Box>
      {sessions.map((session) => <Box key={session.id} sx={{ py: 1, borderBottom: 1, borderColor: "divider" }}>
        <Box sx={{ fontSize: "0.875rem" }}>{session.title}</Box>
        <SessionActionBadge session={session} />
      </Box>)}
    </Box>
  </BrowserProductTheme>));
}
