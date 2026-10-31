/** Actual elicitation form fields, choices and approval surfaces, driven
 *  through the DOM. The answer is captured instead of sent. */
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { Button, CssBaseline, Stack } from "@mui/material";
import { BrowserProductTheme } from "./browserProductTheme";
import { type Elicitation, elicitationOf } from "./elicitation";
import { ElicitationFields, useElicitationForm } from "./ElicitationFields";
import { DesktopPermissionDock } from "./desktop/DesktopApprovalDock";
import { PermissionOverlay } from "./PermissionOverlay";
import type { PendingPermission } from "./derive";

function check(value: unknown, label: string): asserts value {
  if (!value) throw new Error(label);
}

const FORM = {
  mode: "form",
  sessionId: "s",
  toolCallId: "item-1",
  message: "Codex needs your input to continue.",
  requestedSchema: {
    type: "object",
    properties: {
      database: {
        type: "string",
        title: "Which database should the service use?",
        oneOf: [
          { const: "Postgres", title: "Postgres", description: "Recommended" },
          { const: "SQLite", title: "SQLite" },
        ],
      },
      features: {
        type: "array",
        title: "Features",
        items: { anyOf: [{ const: "Auth", title: "Auth" }, { const: "Search", title: "Search" }] },
      },
      token: { type: "string", title: "API token", _meta: { codex: { isSecret: true } } },
    },
    required: ["database", "token"],
  },
};

function parsed(request: Record<string, unknown>): Elicitation {
  const elicitation = elicitationOf({ _cowboy: { elicitation: request } });
  check(elicitation, "fixture elicitation did not parse");
  return elicitation;
}

function Harness(
  { elicitation, answers }: { elicitation: Elicitation; answers: string[] },
): React.JSX.Element {
  const form = useElicitationForm("r-1", elicitation, (answer) => answers.push(answer));
  return (
    <Stack spacing={1} sx={{ width: 420, p: 2 }} data-harness>
      <ElicitationFields
        elicitation={elicitation}
        values={form.values}
        setValue={form.setValue}
        disabled={false}
      />
      {form.choices.map((choice) => (
        <Button
          key={choice.id}
          data-choice={choice.id}
          disabled={choice.disabled}
          onClick={choice.run}
        >
          {choice.label}
        </Button>
      ))}
    </Stack>
  );
}

async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
}

function setInput(input: HTMLInputElement | HTMLTextAreaElement, value: string): void {
  const prototype = input instanceof HTMLTextAreaElement
    ? HTMLTextAreaElement.prototype
    : HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(prototype, "value")?.set?.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

export async function runElicitationBrowserConformance(): Promise<string[]> {
  const passed: string[] = [];
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const render = (node: React.JSX.Element): void =>
    flushSync(() => root.render(<BrowserProductTheme><CssBaseline />{node}</BrowserProductTheme>));

  // 1. A form submits only once its required answers are in, as typed content.
  const answers: string[] = [];
  render(<Harness elicitation={parsed(FORM)} answers={answers} />);
  const submit = (): HTMLButtonElement => {
    const node = container.querySelector<HTMLButtonElement>('[data-choice="accept"]');
    check(node, "missing Submit");
    return node;
  };
  check(submit().disabled, "Submit must wait for required answers");
  const radios = container.querySelectorAll<HTMLInputElement>('[role="radiogroup"] input[type="radio"]');
  check(radios.length === 2, "single choice renders its options as radios");
  radios[1]?.click();
  await settle();
  const boxes = container.querySelectorAll<HTMLInputElement>('[role="group"] input[type="checkbox"]');
  check(boxes.length === 2, "multi choice renders its options as checkboxes");
  boxes[1]?.click();
  await settle();
  const secret = container.querySelector<HTMLInputElement>('input[aria-label="API token"]');
  check(secret?.type === "password", "a secret answer is masked");
  setInput(secret, "t0ken");
  await settle();
  check(!submit().disabled, "Submit is available once required answers are in");
  submit().click();
  await settle();
  check(
    answers.at(-1) === 'accept:{"database":"SQLite","features":["Search"],"token":"t0ken"}',
    `unexpected form answer ${String(answers.at(-1))}`,
  );
  passed.push("form submits typed content once complete");

  // 2. Decline never carries content.
  container.querySelector<HTMLButtonElement>('[data-choice="decline"]')?.click();
  await settle();
  check(answers.at(-1) === "decline", "Decline answers decline");
  passed.push("decline carries no content");

  // 3. A URL elicitation links out and is answered with Done.
  const url = parsed({
    mode: "url",
    requestId: 1,
    elicitationId: "e-1",
    url: "https://auth.example.com/authorize?x=1",
    message: "Sign in to the MCP server",
  });
  render(<Harness elicitation={url} answers={answers} />);
  const link = container.querySelector<HTMLAnchorElement>('[data-elicitation="url"] a');
  check(link?.href === "https://auth.example.com/authorize?x=1", "URL elicitation links to its page");
  check(link.target === "_blank" && link.rel.includes("noopener"), "the link opens apart from Cowboy");
  check(link.textContent?.includes("auth.example.com"), "the link names its host");
  check(submit().textContent?.includes("Done") && !submit().disabled, "Done is available at once");
  submit().click();
  await settle();
  check(answers.at(-1) === "accept", "Done accepts without content");
  passed.push("url elicitation links out and accepts with Done");

  // 4. Both approval surfaces draw the form instead of the raw request.
  const item: PendingPermission = {
    seq: 1,
    requestId: "r-2",
    requestKind: "question",
    title: "Codex needs your input to continue.",
    options: [
      { optionId: "accept", name: "Submit", kind: "allow_once" },
      { optionId: "decline", name: "Decline", kind: "reject_once" },
    ],
    elicitation: parsed(FORM),
  };
  render(<DesktopPermissionDock item={item} waiting={1} sessionId="s" />);
  check(container.querySelector('[data-elicitation="form"]'), "Desktop dock draws the form");
  check(!container.querySelector("[data-desktop-approval-subject]"), "Desktop dock hides the raw subject");
  const dockSubmit = container.querySelector<HTMLButtonElement>('[data-desktop-approval-choice="1"]');
  check(dockSubmit?.disabled, "Desktop Submit waits for required answers");
  render(<PermissionOverlay item={item} sessionId="s" />);
  check(container.querySelector('[data-elicitation="form"]'), "Mobile overlay draws the form");
  passed.push("approval surfaces draw the form");

  // 5. An option preview is collapsed, opens as bounded plain text, and
  //    opening it never picks the option.
  const previewed = parsed({
    mode: "form",
    sessionId: "s",
    message: "Which layout?",
    requestedSchema: {
      type: "object",
      properties: {
        question_0: {
          type: "string",
          oneOf: [
            {
              const: "Grid",
              title: "Grid",
              _meta: {
                "_claude/askUserQuestionOption": {
                  preview: `<b>grid</b>\n${"| cell ".repeat(40)}\n${"row\n".repeat(60)}`,
                },
              },
            },
            { const: "List", title: "List" },
          ],
        },
      },
    },
  });
  render(<Harness elicitation={previewed} answers={answers} />);
  const previews = container.querySelectorAll<HTMLElement>("[data-elicitation-preview]");
  check(previews.length === 1, "only the option with a preview offers one");
  check(previews[0]?.dataset["elicitationPreview"] === "closed", "a preview starts collapsed");
  const toggle = previews[0]?.querySelector<HTMLButtonElement>("button");
  check(toggle?.getAttribute("aria-expanded") === "false", "the toggle reports collapsed");
  toggle.click();
  await settle();
  const pre = container.querySelector<HTMLElement>('[data-elicitation-preview="open"] pre');
  check(pre, "the preview opens");
  check(toggle.getAttribute("aria-expanded") === "true", "the toggle reports expanded");
  check(!pre.querySelector("b") && pre.textContent?.startsWith("<b>grid</b>"), "a preview is plain text");
  const style = getComputedStyle(pre);
  check(style.fontFamily.includes("monospace") && style.whiteSpace === "pre", "a preview is monospace and unwrapped");
  check(pre.clientHeight <= 240 && pre.scrollHeight > pre.clientHeight, "a long preview scrolls inside a bounded box");
  const picked = container.querySelectorAll<HTMLInputElement>('[role="radiogroup"] input[type="radio"]');
  check(picked.length === 2 && !picked[0]?.checked && !picked[1]?.checked, "opening a preview picks nothing");
  passed.push("option preview is collapsed plain text that picks nothing");

  // 6. A Codex request that resolves itself shows its countdown on both surfaces.
  const timed: PendingPermission = {
    ...item,
    seq: 2,
    requestId: "r-3",
    elicitation: parsed({ ...FORM, _meta: { codex: { autoResolutionMs: 75_000 } } }),
  };
  render(<DesktopPermissionDock item={timed} waiting={1} sessionId="s" />);
  const desktopTimer = container.querySelector<HTMLElement>('[role="timer"]');
  check(desktopTimer?.textContent?.includes("1:15") || desktopTimer?.textContent?.includes("1:14"), `Desktop countdown: ${String(desktopTimer?.textContent)}`);
  render(<PermissionOverlay item={timed} sessionId="s" />);
  const mobileTimer = container.querySelector<HTMLElement>('[role="timer"]');
  check(mobileTimer?.textContent?.includes("Continues without your answer"), "Mobile overlay shows the countdown");
  render(<PermissionOverlay item={item} sessionId="s" />);
  check(!container.querySelector('[role="timer"]'), "a request that waits for the user has no countdown");
  passed.push("auto-resolving request shows its countdown");

  root.unmount();
  container.remove();
  return passed;
}

/** Leave both approval surfaces drawn for a visual review. */
export function renderElicitationPreview(): void {
  const container = document.createElement("div");
  document.body.append(container);
  const item: PendingPermission = {
    seq: 1,
    requestId: "preview",
    requestKind: "question",
    title: "Codex needs your input to continue.",
    options: [],
    elicitation: parsed({
      ...FORM,
      _meta: { codex: { autoResolutionMs: 75_000 } },
      requestedSchema: {
        ...FORM.requestedSchema,
        properties: {
          ...FORM.requestedSchema.properties,
          database: {
            ...FORM.requestedSchema.properties.database,
            oneOf: [
              {
                const: "Postgres",
                title: "Postgres",
                description: "Recommended",
                _meta: {
                  "_claude/askUserQuestionOption": {
                    preview: "services:\n  db:\n    image: postgres:17\n    volumes: [pg:/var/lib/postgresql]",
                  },
                },
              },
              { const: "SQLite", title: "SQLite" },
            ],
          },
        },
      },
    }),
  };
  const tool: PendingPermission = {
    seq: 2,
    requestId: "preview-tool",
    requestKind: "permission",
    title: "cd /srv/storage/fast0/agent/worktrees/cowboy && just check",
    options: [
      { optionId: "once", name: "Allow once", kind: "allow_once" },
      { optionId: "always", name: "Always allow in this session", kind: "allow_always" },
      { optionId: "reject", name: "Reject", kind: "reject_once" },
    ],
    elicitation: null,
  };
  createRoot(container).render(
    <BrowserProductTheme>
      <CssBaseline />
      <Stack direction="row" spacing={3} sx={{ p: 2, alignItems: "flex-start" }}>
        <Stack sx={{ width: 640, border: 1, borderColor: "divider" }}>
          <DesktopPermissionDock item={item} waiting={1} sessionId="s" />
        </Stack>
        <Stack sx={{ width: 390 }} spacing={2}>
          <PermissionOverlay item={item} sessionId="s" />
          <PermissionOverlay item={tool} sessionId="s" />
        </Stack>
      </Stack>
    </BrowserProductTheme>,
  );
}
