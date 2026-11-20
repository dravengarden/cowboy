// Cowboy-owned motion for a Provider's loading asset in the activity line
// (ProviderSurface's `activity` node). A Provider selects the strategy and its
// period; the geometry, colors and reduced-motion behavior stay here.
import { Box, keyframes } from "@mui/material";

const assetSignalMotion = keyframes`
  from { -webkit-mask-position: 120% 0; mask-position: 120% 0; }
  to { -webkit-mask-position: -20% 0; mask-position: -20% 0; }
`;
const assetPulseMotion = keyframes`
  0%, 100% { opacity: 0.46; }
  50% { opacity: 0.92; }
`;
// Levitation: the mark rises off its shadow and settles again; the shadow
// tightens and fades while it is aloft; particles drift up past it.
const assetLevitateMotion = keyframes`
  0%, 100% { transform: translateY(1px) rotate(-2deg); }
  50% { transform: translateY(-2.5px) rotate(2deg); }
`;
const assetLevitateShadow = keyframes`
  0%, 100% { transform: translateX(-50%) scaleX(1); opacity: 0.32; }
  50% { transform: translateX(-50%) scaleX(0.55); opacity: 0.12; }
`;
const assetLevitateParticle = keyframes`
  0% { transform: translateY(0) scale(1); opacity: 0; }
  20% { opacity: 0.9; }
  100% { transform: translateY(-13px) scale(0.4); opacity: 0; }
`;

export function AssetActivityMark({
  kind,
  period,
  box,
  colors: [primary, secondary],
  reducedMotion,
  graphic,
}: {
  kind: "asset_signal" | "asset_pulse" | "asset_levitate";
  period: number;
  box: string;
  colors: [string, string];
  reducedMotion: boolean;
  graphic: () => React.JSX.Element;
}): React.JSX.Element {
  switch (kind) {
    case "asset_signal":
      return (
        <Box
          component="span"
          aria-hidden
          sx={{
            width: box,
            height: box,
            position: "relative",
            display: "inline-block",
            flexShrink: 0,
            "& .provider-signal-base": {
              opacity: reducedMotion ? 0.82 : 0.62,
            },
            "& .provider-signal-sweep": reducedMotion
              ? { display: "none" }
              : {
                WebkitMaskImage:
                  "linear-gradient(115deg, transparent 32%, #000 46%, #000 54%, transparent 68%)",
                maskImage:
                  "linear-gradient(115deg, transparent 32%, #000 46%, #000 54%, transparent 68%)",
                WebkitMaskSize: "300% 100%",
                maskSize: "300% 100%",
                WebkitMaskRepeat: "no-repeat",
                maskRepeat: "no-repeat",
                animation: `${assetSignalMotion} ${period}ms ease-in-out infinite`,
              },
          }}
        >
          <Box className="provider-signal-base" sx={{ position: "absolute", inset: 0 }}>
            {graphic()}
          </Box>
          <Box className="provider-signal-sweep" sx={{ position: "absolute", inset: 0 }}>
            {graphic()}
          </Box>
        </Box>
      );
    case "asset_pulse":
      return (
        <Box
          component="span"
          aria-hidden
          sx={{
            width: box,
            height: box,
            display: "inline-flex",
            animation: reducedMotion
              ? "none"
              : `${assetPulseMotion} ${period}ms ease-in-out infinite`,
          }}
        >
          {graphic()}
        </Box>
      );
    case "asset_levitate": {
      const particles = [
        { left: "12%", color: secondary, delay: 0 },
        { left: "78%", color: primary, delay: period / 3 },
        { left: "46%", color: secondary, delay: (period * 2) / 3 },
      ];
      return (
        <Box
          component="span"
          aria-hidden
          data-provider-levitate
          sx={{
            width: box,
            height: box,
            position: "relative",
            display: "inline-block",
            flexShrink: 0,
            overflow: "visible",
          }}
        >
          <Box
            component="span"
            sx={{
              position: "absolute",
              left: "50%",
              bottom: "-2px",
              width: "80%",
              height: "2px",
              borderRadius: "50%",
              bgcolor: "text.primary",
              transform: "translateX(-50%)",
              opacity: 0.24,
              animation: reducedMotion
                ? "none"
                : `${assetLevitateShadow} ${period}ms ease-in-out infinite`,
            }}
          />
          {!reducedMotion && particles.map((particle) => (
            <Box
              key={particle.left}
              component="span"
              sx={{
                position: "absolute",
                left: particle.left,
                bottom: 0,
                width: "2px",
                height: "2px",
                borderRadius: "50%",
                bgcolor: particle.color,
                opacity: 0,
                animation:
                  `${assetLevitateParticle} ${period}ms ease-out ${particle.delay}ms infinite`,
              }}
            />
          ))}
          <Box
            component="span"
            sx={{
              position: "absolute",
              inset: 0,
              display: "inline-flex",
              animation: reducedMotion
                ? "none"
                : `${assetLevitateMotion} ${period}ms ease-in-out infinite`,
            }}
          >
            {graphic()}
          </Box>
        </Box>
      );
    }
  }
}
