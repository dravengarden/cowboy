/** Keep a focused rich-editor caret visible when its writing canvas shrinks
 * (keyboard/split pane) or rewraps. Native textareas retain browser ownership.
 * Observe the actual canvas, not a guessed keyboard duration or viewport size. */
export function observeDraftViewport(
  canvas: HTMLElement,
  reveal: () => void,
): () => void {
  let width = canvas.clientWidth;
  let height = canvas.clientHeight;
  let frame = 0;
  const observer = new ResizeObserver(() => {
    const nextWidth = canvas.clientWidth;
    const nextHeight = canvas.clientHeight;
    const changed = nextWidth !== width || nextHeight < height;
    width = nextWidth;
    height = nextHeight;
    if (!changed || frame) return;
    frame = requestAnimationFrame(() => {
      frame = 0;
      reveal();
    });
  });
  observer.observe(canvas);
  return () => {
    observer.disconnect();
    if (frame) cancelAnimationFrame(frame);
  };
}
