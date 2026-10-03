import "../device-proof.js";

interface BrowserDevice {
  proof(url: string, method?: string): Promise<string>;
  resetChallenge(): void;
  install(): void;
}

const device = (globalThis as typeof globalThis & { CowboyDeviceProof: BrowserDevice }).CowboyDeviceProof;

export function installBrowserDevice(): void {
  device.install();
}

export async function browserDeviceProtocol(url: string): Promise<string> {
  return `cowboy-device.${await device.proof(url)}`;
}

export function resetBrowserDeviceChallenge(): void {
  device.resetChallenge();
}
