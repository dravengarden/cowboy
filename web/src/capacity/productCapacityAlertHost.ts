type ProductCapacityAlertHostListener = () => void;

let desktopHost: HTMLElement | null = null;
const listeners = new Set<ProductCapacityAlertHostListener>();

export function productCapacityAlertHost(): HTMLElement | null {
  return desktopHost;
}

export function setProductCapacityAlertHost(host: HTMLElement | null): void {
  if (desktopHost === host) return;
  desktopHost = host;
  for (const listener of listeners) listener();
}

export function subscribeProductCapacityAlertHost(
  listener: ProductCapacityAlertHostListener,
): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
