import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useTranslation } from "react-i18next";

export default function AboutPanel() {
  const { t } = useTranslation();
  const [version, setVersion] = useState("");

  useEffect(() => {
    getVersion().then(setVersion);
  }, []);

  return (
    <div className="about-panel">
      <img src="/app-icon.png" alt="" className="about-icon" />
      <h2>uniCaptions</h2>
      <p className="hint">{version && t("about.version", { version })}</p>
      <p className="about-author">
        {t("about.createdBy")}{" "}
        <a
          href="https://riko.dev"
          onClick={(e) => {
            e.preventDefault();
            void openUrl("https://riko.dev");
          }}
        >
          RikoDEV
        </a>
      </p>
    </div>
  );
}
