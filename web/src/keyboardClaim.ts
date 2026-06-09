// Claim the on-screen keyboard DURING a user gesture. iOS only raises the
// keyboard for a focus() that happens inside the gesture's task; a sheet's own
// field focuses AFTER it mounts/settles — outside that window — so the caret
// appears but no keyboard. Fix: synchronously focus a persistent off-screen input
// in the tap to "claim" the keyboard; when the real field mounts and focuses
// shortly after, focus just transfers between inputs and the keyboard stays up.
// Used by the rename input + the mobile fullscreen compose sheet (both open a
// non-Modal DetentSheet, which deliberately never steals focus back off the claim).
let kbClaimEl: HTMLInputElement | null = null;

export function claimKeyboard(): void {
  const doc = globalThis.document;
  if (typeof doc === "undefined") return;
  if (!kbClaimEl) {
    kbClaimEl = doc.createElement("input");
    kbClaimEl.setAttribute("aria-hidden", "true");
    kbClaimEl.tabIndex = -1;
    Object.assign(kbClaimEl.style, {
      position: "fixed",
      top: "0px",
      left: "0px",
      width: "1px",
      height: "1px",
      opacity: "0",
      border: "0",
      padding: "0",
      // ≥16px so focusing it never triggers iOS auto-zoom.
      fontSize: "16px",
    });
    doc.body.appendChild(kbClaimEl);
  }
  kbClaimEl.focus();
}
