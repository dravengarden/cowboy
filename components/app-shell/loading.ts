// The pre-React shell uses the same one-third arc language as MUI's general
// waits. Borders only split into quarters, so a masked conic gradient draws it.
export const splashLoadingCss = `
#app-splash .app-splash-spinner {
  --cowboy-splash-ink: currentColor;
  width: 2.125rem; height: 2.125rem; box-sizing: border-box;
  border-radius: 50%;
  background: conic-gradient(
    var(--cowboy-splash-ink) 0turn calc(1turn / 3),
    color-mix(in srgb, var(--cowboy-splash-ink) 20%, transparent) 0
  );
  -webkit-mask: radial-gradient(farthest-side, transparent calc(100% - .1875rem), #000 calc(100% - .1875rem + .5px));
  mask: radial-gradient(farthest-side, transparent calc(100% - .1875rem), #000 calc(100% - .1875rem + .5px));
  animation: cowboy-splash-rotate 600ms linear infinite;
}
#app-splash .app-splash-spinner > span { display: none; }
@keyframes cowboy-splash-rotate { to { transform: rotate(360deg); } }
@media (prefers-reduced-motion: reduce) {
  #app-splash .app-splash-spinner { animation: none; }
}
@media (forced-colors: active) {
  #app-splash .app-splash-spinner {
    forced-color-adjust: none;
    --cowboy-splash-ink: CanvasText;
  }
}
`;
