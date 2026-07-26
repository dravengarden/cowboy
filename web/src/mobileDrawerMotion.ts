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
  const openOpacity = reducedMotion ? 0.78 : phone ? 0.58 : 0.68;

  return {
    progress,
    scale: 1 - (1 - openScale) * progress,
    opacity: 1 - (1 - openOpacity) * progress,
  };
}
