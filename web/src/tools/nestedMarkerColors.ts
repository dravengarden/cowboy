export type MarkerTone = "violet" | "blue" | "cyan" | "green" | "yellow" | "orange" | "pink" | "red";

// Marker selection may vary by command tree, but its visual identity never
// does: every emoji belongs to one tone that matches its own character. Parent
// references, child badges, and rails all resolve through this same table.
const markerTones: Readonly<Record<MarkerTone, readonly string[]>> = {
  violet: ["🔮", "💎", "🧿", "✨", "🦄", "🎮", "🕹️", "🧩", "🪄", "🎭"],
  blue: ["🪐", "🌙", "⭐", "🌀", "🛸", "🛰️", "🌌", "🐳", "🐬", "🐧"],
  cyan: ["🌊", "🫐", "🦋", "🐙", "🐡"],
  green: ["🍀", "🥝", "🐢", "🌵", "🌴", "🌲"],
  yellow: ["⚡", "🍋", "🐝", "🎯", "🎺", "🌻"],
  orange: ["🚀", "☄️", "🍊", "🦊", "🎸", "🥁", "🎈", "🎪", "🍄", "🍁", "🦀"],
  pink: ["🌈", "🌸", "🍇", "🎹", "🎨", "🧸", "🎁", "🌺", "🪷", "🦜", "🦩"],
  red: ["🔥", "🐉", "🐈", "🐞"],
};

const lightColors: Readonly<Record<MarkerTone, string>> = {
  violet: "#7655d9",
  blue: "#3478c9",
  cyan: "#168ca3",
  green: "#26875d",
  yellow: "#9a7610",
  orange: "#b8651b",
  pink: "#c44f88",
  red: "#c34b4b",
};

const darkColors: Readonly<Record<MarkerTone, string>> = {
  violet: "#a88cf7",
  blue: "#67aaf5",
  cyan: "#55bfd0",
  green: "#62c995",
  yellow: "#d8bd55",
  orange: "#ee9b45",
  pink: "#ed82b5",
  red: "#ed7474",
};

const toneByMarker = new Map<string, MarkerTone>(
  Object.entries(markerTones).flatMap(([tone, markers]) =>
    markers.map((marker) => [marker, tone as MarkerTone] as const)
  ),
);

export function nestedMarkerColor(marker: string | undefined, dark: boolean, fallbackIndex = 0): string {
  const tone = marker ? toneByMarker.get(marker) : undefined;
  if (tone) return (dark ? darkColors : lightColors)[tone];
  const fallback = (dark ? darkColors : lightColors)["violet"];
  const palette = Object.values(dark ? darkColors : lightColors);
  return palette[Math.abs(fallbackIndex) % palette.length] ?? fallback;
}
