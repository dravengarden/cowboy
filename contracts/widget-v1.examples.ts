// Worked examples for docs/widgets.md: the three report shapes history shows
// most (status table, comparison, approval) and one in-place update. Each
// example carries the Markdown an agent writes today for the same content,
// so widget-v1.test.ts can hold the documented size claims to the data.

import { type PatchOp, WIDGET_SCHEMA, type WidgetDocument } from "./widget-v1.ts";

export const statusTable: WidgetDocument = {
  schema: WIDGET_SCHEMA,
  title: "Hawk services",
  summary: "5 units checked, 1 degraded",
  icon: "machine",
  root: {
    type: "stack",
    children: [
      {
        type: "table",
        id: "units",
        columns: [
          "Unit",
          { label: "State", format: "status" },
          { label: "RSS", format: "bytes", align: "end" },
          "Since",
        ],
        rows: [
          ["cowboy.service", { v: "active", tone: "success" }, 412_000_000, "2d 4h"],
          ["victoria-logs.service", { v: "active", tone: "success" }, 1_320_000_000, "2d 4h"],
          ["chrome-debug.service", { v: "degraded", tone: "warning" }, 2_100_000_000, "35m"],
          ["stormbird-device.service", { v: "active", tone: "success" }, 88_000_000, "2d 4h"],
          ["janus.service", { v: "inactive", tone: "neutral" }, 0, "—"],
        ],
      },
      {
        type: "button",
        id: "restart",
        label: "Restart chrome-debug",
        variant: "secondary",
        icon: "refresh",
        confirm: { title: "Restart chrome-debug.service?", text: "Open Chrome tabs are lost." },
        action: { kind: "prompt", text: "Restart chrome-debug.service and report the new RSS." },
      },
    ],
  },
};

export const statusTableMarkdown = `**Hawk services** — 5 units checked, 1 degraded

| Unit | State | RSS | Since |
|---|---|---|---|
| cowboy.service | ✅ active | 412 MB | 2d 4h |
| victoria-logs.service | ✅ active | 1.32 GB | 2d 4h |
| chrome-debug.service | ⚠️ degraded | 2.1 GB | 35m |
| stormbird-device.service | ✅ active | 88 MB | 2d 4h |
| janus.service | ⚪ inactive | 0 | — |

要不要我重启 chrome-debug.service？会丢失已打开的 Chrome 标签页。`;

export const comparison: WidgetDocument = {
  schema: WIDGET_SCHEMA,
  title: "Signup token handoff",
  summary: "Option A (query) recommended",
  root: {
    type: "stack",
    children: [
      {
        type: "table",
        id: "matrix",
        columns: ["", "A: query", "B: redirect + cookie"],
        rows: [
          ["Redirects", "1", "2"],
          [
            "Token visible to seller script",
            { v: "until the SDK strips it", tone: "warning" },
            { v: "never (HttpOnly)", tone: "success" },
          ],
          ["Refresh", "sessionStorage", "cookie"],
          ["Host design", "one host + path", { v: "one subdomain per seller", tone: "warning" }],
          ["Worker", "reads R2", "redirect, cookie, proxy"],
        ],
      },
      {
        type: "alert",
        severity: "info",
        text: "Recommendation: **A**. The token is low-value; B's isolation does not pay for its DNS, wildcard and proxy complexity.",
      },
      {
        type: "choice",
        id: "pick",
        options: [
          { value: "a", label: "Go with A" },
          { value: "b", label: "Go with B" },
          { value: "more", label: "Need more detail" },
        ],
        submit: { kind: "prompt", text: "Decision on the signup token handoff" },
      },
    ],
  },
};

export const comparisonMarkdown = `## 对比

| | 方案一：query | 方案二：redirect + cookie |
|---|---|---|
| 跳转次数 | 1 | 2 |
| 卖家脚本能否读到 token | 被 SDK 擦掉之前读得到 | **完全读不到**（HttpOnly） |
| 刷新 | 依赖 sessionStorage | cookie 自带 |
| host 设计 | 一个 host 加 path | **每个卖家一个一级子域** |
| Worker | 只从 R2 读页面 | 还要处理 redirect、cookie、代理提交 |

**我的建议还是方案一**：token 价值很低，方案二多出来的隔离收益抵不上子域名、DNS、Worker 代理的复杂度。

选方案一、方案二，还是需要更多细节？`;

export const approval: WidgetDocument = {
  schema: WIDGET_SCHEMA,
  title: "Deploy cowboy-web 1b9f121?",
  summary: "Build passed; waiting for your decision",
  icon: "deploy",
  root: {
    type: "card",
    title: "Release candidate",
    status: { label: "built", tone: "success" },
    children: [
      {
        type: "kv",
        items: [
          { k: "Commit", v: "1b9f121eb" },
          { k: "Output", v: ".#cowboy-web-release" },
          { k: "Tests", v: "web 412 passed", tone: "success" },
        ],
      },
      {
        type: "steps",
        id: "plan",
        steps: [
          { label: "Build", state: "done" },
          { label: "Activate /run/cowboy-web", state: "pending" },
          { label: "Verify /version and the service worker", state: "pending" },
        ],
      },
    ],
    actions: [
      {
        type: "button",
        id: "go",
        label: "Activate",
        variant: "primary",
        confirm: { title: "Activate on hawk?", text: "Moves /run/cowboy-web; restarts nothing." },
        action: { kind: "prompt", text: "Activate the built cowboy-web release and verify it." },
      },
      {
        type: "button",
        id: "hold",
        label: "Not now",
        variant: "text",
        action: { kind: "prompt", text: "Do not activate; keep the build result." },
      },
    ],
  },
};

export const approvalMarkdown = `**Release candidate: built**

- **Commit**: 1b9f121eb
- **Output**: .#cowboy-web-release
- **Tests**: web 412 passed

计划：
- [x] 构建
- [ ] 激活 /run/cowboy-web
- [ ] 验证 /version 和 service worker

要不要我现在激活？只移动 /run/cowboy-web，不重启任何进程。`;

/** The update after activation: three fields change, nothing is re-sent. */
export const progressPatch: PatchOp[] = [
  { op: "set", path: "#plan/steps/1/state", value: "done" },
  { op: "set", path: "#plan/steps/2/state", value: "active" },
  { op: "set", path: "/summary", value: "Activated; verifying" },
];

/** What an agent writes today to report the same transition. */
export const progressMarkdown = `已激活 /run/cowboy-web（1b9f121eb），正在验证 /version、SPA 缓存头和 service worker。

- [x] 构建
- [x] 激活 /run/cowboy-web
- [ ] 验证 /version 和 service worker`;
