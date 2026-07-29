type FlagProps = { className?: string };

const wrap = {
  viewBox: "0 0 3 2",
  width: 22,
  height: 15,
  className: "flag-icon",
};

export function FlagUS({ className }: FlagProps) {
  return (
    <svg {...wrap} className={`${wrap.className} ${className ?? ""}`}>
      <rect width="3" height="2" fill="#b22234" />
      {[1, 3, 5, 7, 9, 11].map((i) => (
        <rect key={i} y={(i * 2) / 13} width="3" height={2 / 13} fill="#fff" />
      ))}
      <rect width="1.2" height={2 * (7 / 13)} fill="#3c3b6e" />
    </svg>
  );
}

export function FlagES({ className }: FlagProps) {
  return (
    <svg {...wrap} className={`${wrap.className} ${className ?? ""}`}>
      <rect width="3" height="2" fill="#aa151b" />
      <rect y="0.5" width="3" height="1" fill="#f1bf00" />
    </svg>
  );
}

export function FlagFR({ className }: FlagProps) {
  return (
    <svg {...wrap} className={`${wrap.className} ${className ?? ""}`}>
      <rect width="1" height="2" fill="#0055a4" />
      <rect x="1" width="1" height="2" fill="#fff" />
      <rect x="2" width="1" height="2" fill="#ef4135" />
    </svg>
  );
}

export function FlagDE({ className }: FlagProps) {
  return (
    <svg {...wrap} className={`${wrap.className} ${className ?? ""}`}>
      <rect width="3" height="2" fill="#000" />
      <rect y="0.667" width="3" height="1.333" fill="#dd0000" />
      <rect y="1.333" width="3" height="0.667" fill="#ffce00" />
    </svg>
  );
}

export function FlagPL({ className }: FlagProps) {
  return (
    <svg {...wrap} className={`${wrap.className} ${className ?? ""}`}>
      <rect width="3" height="2" fill="#dc143c" />
      <rect width="3" height="1" fill="#fff" />
    </svg>
  );
}

export function FlagPT({ className }: FlagProps) {
  return (
    <svg {...wrap} className={`${wrap.className} ${className ?? ""}`}>
      <rect width="3" height="2" fill="#ff0000" />
      <rect width="1.2" height="2" fill="#006600" />
      <circle cx="1.2" cy="1" r="0.35" fill="#ffcc00" stroke="#000" strokeWidth="0.02" />
    </svg>
  );
}

export function FlagCN({ className }: FlagProps) {
  return (
    <svg {...wrap} className={`${wrap.className} ${className ?? ""}`}>
      <rect width="3" height="2" fill="#de2910" />
      <polygon
        points="0.5,0.35 0.62,0.7 0.98,0.7 0.68,0.9 0.8,1.25 0.5,1.05 0.2,1.25 0.32,0.9 0.02,0.7 0.38,0.7"
        fill="#ffde00"
      />
    </svg>
  );
}

export function FlagJP({ className }: FlagProps) {
  return (
    <svg {...wrap} className={`${wrap.className} ${className ?? ""}`}>
      <rect width="3" height="2" fill="#fff" />
      <circle cx="1.5" cy="1" r="0.55" fill="#bc002d" />
    </svg>
  );
}

export function FlagGlobe({ className }: FlagProps) {
  return (
    <svg
      viewBox="0 0 24 24"
      width={22}
      height={22}
      fill="none"
      stroke="currentColor"
      strokeWidth={1.8}
      strokeLinecap="round"
      strokeLinejoin="round"
      className={`flag-icon ${className ?? ""}`}
    >
      <circle cx="12" cy="12" r="9" />
      <path d="M3 12h18" />
      <path d="M12 3a14 14 0 0 1 0 18 14 14 0 0 1 0-18Z" />
    </svg>
  );
}
