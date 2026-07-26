export interface MobileDrawerSurfaceVisual {
  progress: number;
  scale: number;
  opacity: number;
}

export function mobileDrawerSurfaceVisual(
  offset: number,
  width: number,
  phone: boolean,
  reducedMotion = false,
): MobileDrawerSurfaceVisual {
  const progress = width > 0
    ? Math.max(0, Math.min(1, offset / width))
    : 0;
  const openScale = reducedMotion ? 1 : phone ? 0.96 : 0.975;
  const openOpacity = reducedMotion ? 0.84 : phone ? 0.66 : 0.74;
  // DeepSeek keeps the foreground page optically solid while the gesture is
  // merely declaring its horizontal intent, then eases the page into the
  // background. A smoothstep avoids both the old first-pixel transparency and
  // a visible kink where fading begins.
  const fadeStart = 0.1;
  const fadePosition = Math.max(
    0,
    Math.min(1, (progress - fadeStart) / (1 - fadeStart)),
  );
  const fadeProgress = fadePosition * fadePosition * (3 - 2 * fadePosition);

  return {
    progress,
    scale: 1 - (1 - openScale) * progress,
    opacity: 1 - (1 - openOpacity) * fadeProgress,
  };
}
