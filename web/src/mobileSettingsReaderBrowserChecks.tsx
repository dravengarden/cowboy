import { useState } from "react";
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import { Box, CssBaseline } from "@mui/material";
import { BrowserProductTheme } from "./browserProductTheme";
import { MobileSettingsRoute, type MobileSettingsSection } from "./SettingsShell";
import { MobileSettingsNavigation } from "./MobileSettingsNavigation";
import { Sheet } from "./Sheet";

export async function checkMobileSettingsReader(keep = false): Promise<void> {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const wait = () => new Promise(resolve => setTimeout(resolve, 350));
  const check = (ok: boolean, message: string) => { if (!ok) throw new Error(message); };
  function Fixture() {
    const [section, setSection] = useState<MobileSettingsSection | null>("info");
    const [open, setOpen] = useState(true);
    return <BrowserProductTheme><CssBaseline /><Sheet open={open} onClose={() => setOpen(false)} forceSheet cover hideMobileTitle mobileDismiss="none"
      actions={<MobileSettingsNavigation section={section} onBack={() => setSection(null)} onClose={() => setOpen(false)} />}>
      <MobileSettingsRoute id="info" title="About" description="Cowboy information and diagnostics" activeSection={section} onChange={setSection}>
        <Box data-settings-reader-content><h3 style={{ margin: "0 0 12px" }}>Usage</h3>{Array.from({ length: 14 }, (_, i) => <Box key={i} sx={{ py: 3, borderBottom: 1, borderColor: "divider" }}>Account {i + 1} · Weekly usage</Box>)}</Box>
      </MobileSettingsRoute>
    </Sheet></BrowserProductTheme>;
  }
  try {
    flushSync(() => root.render(<Fixture />));
    await wait();
    const dialog = document.querySelector<HTMLElement>('[role="dialog"]')!;
    dialog.style.setProperty("--cowboy-system-top-clearance", "47px");
    const content = dialog.querySelector<HTMLElement>("[data-settings-reader-content]")!;
    const footer = dialog.querySelector<HTMLElement>("[data-mobile-settings-navigation]")!;
    check(!dialog.textContent?.includes("Cowboy information and diagnostics"), "Detail drops the redundant description");
    check(content.getBoundingClientRect().top - dialog.getBoundingClientRect().top < 100, "Detail starts directly below the safe-area fade");
    check(footer.textContent === "About", "Current section stays visible in the bottom navigation");
    check(footer.scrollWidth <= footer.clientWidth, "Navigation fits the available width");
    const back = footer.querySelector<HTMLButtonElement>('[aria-label="Back to Settings from About"]')!;
    check(back.getBoundingClientRect().height >= 44, "Back retains its touch target");
    check(back.getBoundingClientRect().width === 44, "Back remains an independent round action");
    const title = footer.querySelector<HTMLElement>('[title="About"]')!;
    check(Math.abs(title.getBoundingClientRect().left + title.getBoundingClientRect().width / 2 - (footer.getBoundingClientRect().left + footer.getBoundingClientRect().width / 2)) < 1, "Section identity is centered independently of the buttons");
    const scroller = dialog.querySelector<HTMLElement>("[data-scroll-edges]")!;
    scroller.scrollTop = scroller.scrollHeight;
    await wait();
    check(content.getBoundingClientRect().bottom <= footer.getBoundingClientRect().top, "Last content can scroll clear of the floating controls");
    back.click();
    await wait();
    check(Boolean(dialog.querySelector('[data-mobile-settings-route="info"]')), "Back restores the Settings index");
    check(dialog.textContent?.includes("Cowboy information and diagnostics") === true, "Index retains useful section descriptions");
    dialog.querySelector<HTMLButtonElement>('[data-mobile-settings-route="info"]')!.click();
    await wait();
    if (!keep) {
      dialog.querySelector<HTMLButtonElement>('[aria-label="Close settings"]')!.click();
      await wait();
      check(!document.querySelector('[role="dialog"]'), "Close is directly available from a detail");
    }
  } finally { if (!keep) { flushSync(() => root.unmount()); container.remove(); } }
}
