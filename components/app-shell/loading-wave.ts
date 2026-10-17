// Shared by the pre-JS shell and the React theme: one cadence at every size.
export const loadingWaveAnimation = "cowboy-loading-wave 1500ms cubic-bezier(0.4, 0, 0.2, 1) infinite";
export const loadingWaveKeyframes = {
  "0%, 60%, 100%": { transform: "scaleY(0.4)" },
  "30%": { transform: "scaleY(1)" },
};

export const splashLoadingCss = `
#app-splash .app-splash-spinner { width: 2.125rem; height: 2.125rem; position: relative; }
#app-splash .app-splash-spinner::before,
#app-splash .app-splash-spinner::after,
#app-splash .app-splash-spinner > span {
  content: ""; position: absolute; top: 18%; width: 18%; height: 64%;
  border-radius: 999px; background: currentColor; transform: scaleY(.4);
  animation: ${loadingWaveAnimation}; animation-play-state: inherit;
}
#app-splash .app-splash-spinner::before { left: 12%; animation-delay: -300ms; }
#app-splash .app-splash-spinner > span { left: 41%; animation-delay: -150ms; }
#app-splash .app-splash-spinner::after { left: 70%; }
@keyframes cowboy-loading-wave {
  ${Object.entries(loadingWaveKeyframes).map(([time, style]) => `${time} { transform: ${style.transform}; }`).join("\n")}
}
@media (prefers-reduced-motion: reduce) {
  #app-splash .app-splash-spinner::before,
  #app-splash .app-splash-spinner::after,
  #app-splash .app-splash-spinner > span { animation: none; }
  #app-splash .app-splash-spinner > span { transform: scaleY(.8); }
}
@media (forced-colors: active) {
  #app-splash .app-splash-spinner::before,
  #app-splash .app-splash-spinner::after,
  #app-splash .app-splash-spinner > span { forced-color-adjust: none; background: CanvasText; }
}
`;
