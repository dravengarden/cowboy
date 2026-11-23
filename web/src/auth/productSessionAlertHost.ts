type ProductSessionAlertHostListener = () => void;

let desktopHost: HTMLElement | null = null;
const listeners = new Set<ProductSessionAlertHostListener>();
let desktopAlertVisible = false;
const alertVisibilityListeners = new Set<ProductSessionAlertHostListener>();

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

export function productSessionAlertVisible(): boolean {
  return desktopAlertVisible;
}

export function setProductSessionAlertVisible(visible: boolean): void {
  if (desktopAlertVisible === visible) return;
  desktopAlertVisible = visible;
  for (const listener of alertVisibilityListeners) listener();
}

export function subscribeProductSessionAlertVisible(
  listener: ProductSessionAlertHostListener,
): () => void {
  alertVisibilityListeners.add(listener);
  return () => alertVisibilityListeners.delete(listener);
}
