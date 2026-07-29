import { useState } from "react";
import { useTranslation } from "react-i18next";

interface Props {
  value: string;
  onChange: (fontFamily: string) => void;
}

// Fallback list used until the user loads real system fonts (or on
// platforms/webviews where the Local Font Access API isn't available).
const FALLBACK_FONTS: { label: string; family: string }[] = [
  { label: "System UI", family: "system-ui, sans-serif" },
  { label: "Segoe UI", family: "'Segoe UI', sans-serif" },
  { label: "Arial", family: "Arial, sans-serif" },
  { label: "Georgia", family: "Georgia, serif" },
  { label: "Times New Roman", family: "'Times New Roman', serif" },
  { label: "Courier New", family: "'Courier New', monospace" },
  { label: "Consolas", family: "Consolas, monospace" },
];

interface LocalFontFace {
  family: string;
  fullName: string;
  postscriptName: string;
}

declare global {
  interface Window {
    queryLocalFonts?: () => Promise<LocalFontFace[]>;
  }
}

export default function FontPicker({ value, onChange }: Props) {
  const { t } = useTranslation();
  const [fonts, setFonts] = useState(FALLBACK_FONTS);
  const [status, setStatus] = useState<"idle" | "loading" | "loaded" | "unsupported" | "denied">(
    "idle",
  );

  const supportsLocalFonts = typeof window !== "undefined" && "queryLocalFonts" in window;

  const loadSystemFonts = async () => {
    if (!window.queryLocalFonts) {
      setStatus("unsupported");
      return;
    }
    setStatus("loading");
    try {
      const faces = await window.queryLocalFonts();
      const families = Array.from(new Set(faces.map((f) => f.family))).sort((a, b) =>
        a.localeCompare(b),
      );
      setFonts(families.map((family) => ({ label: family, family: `'${family}'` })));
      setStatus("loaded");
    } catch {
      setStatus("denied");
    }
  };

  const isCustom = !fonts.some((f) => f.family === value);

  return (
    <div className="font-picker">
      <div className="field-row field-row-gap">
        <select
          value={isCustom ? "__custom__" : value}
          onChange={(e) => {
            if (e.target.value !== "__custom__") onChange(e.target.value);
          }}
        >
          {isCustom && <option value="__custom__">{t("fontPicker.custom", { value })}</option>}
          {fonts.map((f) => (
            <option key={f.family} value={f.family} style={{ fontFamily: f.family }}>
              {f.label}
            </option>
          ))}
        </select>
        {status !== "loaded" && (
          <button type="button" onClick={loadSystemFonts} disabled={status === "loading"}>
            {status === "loading" ? t("fontPicker.loadingFonts") : t("fontPicker.loadSystemFonts")}
          </button>
        )}
      </div>

      {status === "unsupported" && <p className="hint">{t("fontPicker.unsupportedHint")}</p>}
      {status === "denied" && <p className="hint">{t("fontPicker.deniedHint")}</p>}
      {!supportsLocalFonts && status === "idle" && (
        <p className="hint">{t("fontPicker.curatedHint")}</p>
      )}
    </div>
  );
}
