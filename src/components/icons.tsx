type IconProps = { className?: string };

const base = {
  width: 18,
  height: 18,
  viewBox: "0 0 24 24",
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 1.8,
  strokeLinecap: "round" as const,
  strokeLinejoin: "round" as const,
};

export function IconGeneral({ className }: IconProps) {
  return (
    <svg {...base} className={className}>
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06A2 2 0 1 1 7.04 4.3l.06.06A1.65 1.65 0 0 0 8.92 4.7c.55-.22 1-.68 1-1.27V3a2 2 0 0 1 4 0v.09c0 .59.45 1.05 1 1.27.63.26 1.34.14 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06c-.47.48-.59 1.19-.33 1.82.22.55.68 1 1.27 1H21a2 2 0 0 1 0 4h-.09c-.59 0-1.05.45-1.27 1Z" />
    </svg>
  );
}

export function IconStyle({ className }: IconProps) {
  return (
    <svg {...base} className={className}>
      <polyline points="4 7 4 4 20 4 20 7" />
      <line x1="9" y1="20" x2="15" y2="20" />
      <line x1="12" y1="4" x2="12" y2="20" />
    </svg>
  );
}

export function IconTranslate({ className }: IconProps) {
  return (
    <svg {...base} className={className}>
      <circle cx="12" cy="12" r="9" />
      <path d="M3 12h18" />
      <path d="M12 3a14 14 0 0 1 0 18 14 14 0 0 1 0-18Z" />
    </svg>
  );
}

export function IconModels({ className }: IconProps) {
  return (
    <svg {...base} className={className}>
      <ellipse cx="12" cy="5" rx="8" ry="3" />
      <path d="M4 5v6c0 1.66 3.58 3 8 3s8-1.34 8-3V5" />
      <path d="M4 11v6c0 1.66 3.58 3 8 3s8-1.34 8-3v-6" />
    </svg>
  );
}

export function IconMic({ className }: IconProps) {
  return (
    <svg {...base} className={className}>
      <rect x="9" y="2" width="6" height="12" rx="3" />
      <path d="M5 10a7 7 0 0 0 14 0" />
      <line x1="12" y1="19" x2="12" y2="22" />
      <line x1="8" y1="22" x2="16" y2="22" />
    </svg>
  );
}

export function IconSpeaker({ className }: IconProps) {
  return (
    <svg {...base} className={className}>
      <polygon points="4 9 8 9 13 4 13 20 8 15 4 15 4 9" />
      <path d="M17 8a5 5 0 0 1 0 8" />
      <path d="M19.5 5.5a9 9 0 0 1 0 13" />
    </svg>
  );
}

export function IconOffline({ className }: IconProps) {
  return (
    <svg {...base} className={className}>
      <rect x="6" y="6" width="12" height="12" rx="2" />
      <line x1="9" y1="2" x2="9" y2="6" />
      <line x1="15" y1="2" x2="15" y2="6" />
      <line x1="9" y1="18" x2="9" y2="22" />
      <line x1="15" y1="18" x2="15" y2="22" />
      <line x1="2" y1="9" x2="6" y2="9" />
      <line x1="2" y1="15" x2="6" y2="15" />
      <line x1="18" y1="9" x2="22" y2="9" />
      <line x1="18" y1="15" x2="22" y2="15" />
    </svg>
  );
}

export function IconCloud({ className }: IconProps) {
  return (
    <svg {...base} className={className}>
      <path d="M17.5 19a4.5 4.5 0 0 0 0-9 6 6 0 0 0-11.4 1.5A4 4 0 0 0 6.5 19h11Z" />
    </svg>
  );
}

export function IconInfo({ className }: IconProps) {
  return (
    <svg {...base} className={className}>
      <circle cx="12" cy="12" r="9" />
      <line x1="12" y1="11" x2="12" y2="16.5" />
      <circle cx="12" cy="7.8" r="0.9" fill="currentColor" stroke="none" />
    </svg>
  );
}

// Ascending signal-style bars, used to indicate model size/accuracy tiers.
// `filled` (1-4) is how many of the four bars are highlighted.
function TierBars({ className, filled }: IconProps & { filled: number }) {
  const heights = [6, 10, 14, 18];
  return (
    <svg width={18} height={18} viewBox="0 0 24 24" className={className}>
      {heights.map((h, i) => (
        <rect
          key={i}
          x={2 + i * 5.5}
          y={20 - h}
          width={3.5}
          height={h}
          rx={1}
          fill="currentColor"
          opacity={i < filled ? 1 : 0.25}
        />
      ))}
    </svg>
  );
}

export function IconTierTiny({ className }: IconProps) {
  return <TierBars className={className} filled={1} />;
}

export function IconTierBase({ className }: IconProps) {
  return <TierBars className={className} filled={2} />;
}

export function IconTierSmall({ className }: IconProps) {
  return <TierBars className={className} filled={3} />;
}

export function IconTierMedium({ className }: IconProps) {
  return <TierBars className={className} filled={4} />;
}
