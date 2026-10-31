// The pre-React shell uses the same quarter-arc language as MUI's general waits.
export const splashLoadingCss = `
#app-splash .app-splash-spinner {
  width: 2.125rem; height: 2.125rem; box-sizing: border-box;
  border: .1875rem solid color-mix(in srgb, currentColor 20%, transparent);
  border-top-color: currentColor;
  border-radius: 50%; animation: cowboy-splash-rotate 800ms linear infinite;
}
#app-splash .app-splash-spinner > span { display: none; }
@keyframes cowboy-splash-rotate { to { transform: rotate(360deg); } }
@media (prefers-reduced-motion: reduce) {
  #app-splash .app-splash-spinner { animation: none; }
}
@media (forced-colors: active) {
  #app-splash .app-splash-spinner {
    forced-color-adjust: none;
    border-color: color-mix(in srgb, CanvasText 20%, transparent);
    border-top-color: CanvasText;
  }
}
`;
