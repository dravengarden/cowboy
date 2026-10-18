/** Keep a focused rich-editor caret visible when its writing canvas shrinks
 * (keyboard/split pane) or rewraps. Native textareas retain browser ownership.
 * Observe the actual canvas, not a guessed keyboard duration or viewport size. */
export function observeDraftViewport(
  canvas: HTMLElement,
  reveal: () => void,
): () => void {
  let width = canvas.clientWidth;
  let height = canvas.clientHeight;
  const observer = new ResizeObserver(() => {
    const nextWidth = canvas.clientWidth;
    const nextHeight = canvas.clientHeight;
    const changed = nextWidth !== width || nextHeight < height;
    width = nextWidth;
    height = nextHeight;
    // The rich editor queues its own measure/scroll frame. Adding another
    // rAF here paints the shrunken canvas with the old scroll position first,
    // then visibly jumps to the caret during keyboard opening. This callback
    // only schedules the editor effect; it does not measure/write layout.
    if (changed) reveal();
  });
  observer.observe(canvas);
  return () => {
    observer.disconnect();
  };
}
