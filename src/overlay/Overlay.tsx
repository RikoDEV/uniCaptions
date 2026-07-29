import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useSettings } from "../hooks/useSettings";
import { onCaptionUpdate } from "../lib/captions";

function hexToRgba(hex: string, alpha: number): string {
  const clean = hex.replace("#", "");
  const bigint = parseInt(clean.length === 3 ? clean.split("").map((c) => c + c).join("") : clean, 16);
  const r = (bigint >> 16) & 255;
  const g = (bigint >> 8) & 255;
  const b = bigint & 255;
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

export default function Overlay() {
  const { settings, loaded } = useSettings();
  const [original, setOriginal] = useState("");
  const [translated, setTranslated] = useState<string | null>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const unlistenPromise = onCaptionUpdate((event) => {
      setOriginal(event.text);
      setTranslated(event.translatedText);
    });
    return () => {
      unlistenPromise.then((fn) => fn());
    };
  }, []);

  const hasText = original.trim().length > 0;

  useEffect(() => {
    if (!loaded) return;
    // With no caption visible there is nothing to grab or click, so the
    // overlay must always be click-through here regardless of the user's
    // click-through preference — otherwise the invisible window still eats
    // scroll/hover/cursor over its whole area.
    void invoke("set_overlay_click_through", { ignore: hasText ? settings.clickThrough : true });
  }, [loaded, hasText, settings.clickThrough]);

  if (!loaded) return null;

  const style = settings.captionStyle;

  return (
    <div ref={containerRef} className="overlay-root" data-tauri-drag-region>
      {hasText && (
        <div
          className="overlay-caption-box"
          data-tauri-drag-region
          style={{
            fontFamily: style.fontFamily,
            fontSize: `${style.fontSize}px`,
            fontWeight: style.fontWeight,
            color: style.textColor,
            backgroundColor: hexToRgba(style.backgroundColor, style.backgroundOpacity),
            textShadow: style.outline
              ? "1px 1px 2px rgba(0,0,0,0.8), -1px -1px 2px rgba(0,0,0,0.8)"
              : "none",
            WebkitLineClamp: style.maxLines,
          }}
        >
          {style.showOriginal && (
            <div className="overlay-line" data-tauri-drag-region>
              {original}
            </div>
          )}
          {style.showTranslation && translated && (
            <div className="overlay-line overlay-line-translated" data-tauri-drag-region>
              {translated}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
