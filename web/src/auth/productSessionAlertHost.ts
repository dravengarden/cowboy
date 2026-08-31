type ProductSessionAlertHostListener = () => void;

let desktopHost: HTMLElement | null = null;
const listeners = new Set<ProductSessionAlertHostListener>();

export function productSessionAlertHost(): HTMLElement | null {
  return desktopHost;
}

export function setProductSessionAlertHost(host: HTMLElement | null): void {
  if (desktopHost === host) return;
  desktopHost = host;
  for (const listener of listeners) listener();
}

export function subscribeProductSessionAlertHost(
  listener: ProductSessionAlertHostListener,
): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
