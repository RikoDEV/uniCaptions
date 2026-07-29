import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import en from "./en";
import es from "./es";
import fr from "./fr";
import de from "./de";
import pl from "./pl";
import pt from "./pt";
import zh from "./zh";
import ja from "./ja";

export const SUPPORTED_UI_LANGUAGES = ["en", "es", "fr", "de", "pl", "pt", "zh", "ja"] as const;

void i18n.use(initReactI18next).init({
  resources: {
    en: { translation: en },
    es: { translation: es },
    fr: { translation: fr },
    de: { translation: de },
    pl: { translation: pl },
    pt: { translation: pt },
    zh: { translation: zh },
    ja: { translation: ja },
  },
  lng: "en",
  fallbackLng: "en",
  interpolation: { escapeValue: false },
});

export default i18n;
