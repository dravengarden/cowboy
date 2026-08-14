/**
 * Desktop keeps CM6's true block replacement for Vim cursor measurement.
 * Touch keeps the same block-looking image inside an ordinary editable line so
 * physical iOS WebKit has a continuous native-caret DOM around it.
 */
export function inlineImageUsesBlockDecoration(touchInput: boolean): boolean {
  return !touchInput;
}
