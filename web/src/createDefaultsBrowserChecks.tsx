import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { CssBaseline } from "@mui/material";
import { CreateDialog } from "./App";
import { BrowserProductTheme } from "./browserProductTheme";
import { SurfaceContext } from "./surface/SurfaceProfile";
import { classifySurface } from "./surface/profile";
import { createSessionFolder, placeSessions, removeSessionFolder } from "./store";
import type { SessionMeta } from "./protocol";

/** Real Create dialog over the caller's socket/HTTP fixture: a directory
 *  without explicit defaults follows its latest session, explains that
 *  source, and can save the choices as its defaults in one action. */
export async function runCreateDefaultsBrowserChecks(
  source: SessionMeta,
  expectedAccount: string,
): Promise<void> {
  const check = (ok: unknown, message: string): void => {
    if (!ok) throw new Error(`create defaults: ${message}`);
  };
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const folder = createSessionFolder("Fixture dir", null)!;
  placeSessions([source.id], folder);
  const until = async (ready: () => boolean, what: string) => {
    for (let attempt = 0; attempt < 150; attempt++) {
      if (ready()) return;
      await new Promise((resolve) => setTimeout(resolve, 20));
    }
    throw new Error(`create defaults: timed out waiting for ${what}`);
  };
  const text = () => document.querySelector('[role="dialog"]')?.textContent ?? "";
  const saveButton = () =>
    [...document.querySelectorAll<HTMLButtonElement>('[role="dialog"] button')]
      .find((button) => button.textContent?.startsWith("Save as Fixture dir defaults"));
  try {
    flushSync(() =>
      root.render(
        <SurfaceContext.Provider
          value={classifySurface({
            finePointer: true,
            hover: true,
            tabletWidth: false,
            touchCapable: false,
            desktopHost: true,
          })}
        >
          <BrowserProductTheme>
            <CssBaseline />
            <CreateDialog
              open
              initialFolder={folder}
              onClose={() => {}}
              onCreated={() => {}}
            />
          </BrowserProductTheme>
        </SurfaceContext.Provider>,
      )
    );
    await until(
      () => text().includes(expectedAccount) && Boolean(saveButton()),
      "the inherited subscription and save action",
    );
    check(
      text().includes(`Choices match the latest session “${source.title}”`),
      "summary names the session the choices came from",
    );
    const labels = [...document.querySelectorAll('[role="dialog"] label')]
      .map((label) => label.textContent ?? "");
    check(
      labels.findIndex((label) => label.startsWith("Directory")) <
        labels.findIndex((label) => label.startsWith("Project")),
      "the directory is chosen before the fields it decides",
    );
    check(
      !text().includes("· this device"),
      "a directory-supplied project hides the device default line",
    );
    flushSync(() => saveButton()!.click());
    await until(
      () => text().includes("Defaults from Fixture dir") && !saveButton(),
      "the saved directory defaults",
    );
  } finally {
    flushSync(() => root.unmount());
    container.remove();
    removeSessionFolder(folder);
  }
}
