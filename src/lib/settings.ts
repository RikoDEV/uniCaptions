import { load, Store } from "@tauri-apps/plugin-store";
import { emit, listen } from "@tauri-apps/api/event";

export interface CaptionStyle {
  fontFamily: string;
  fontSize: number;
  fontWeight: number;
  textColor: string;
  backgroundColor: string;
  backgroundOpacity: number;
  outline: boolean;
  maxLines: number;
  showOriginal: boolean;
  showTranslation: boolean;
}

export interface AppSettings {
  captionStyle: CaptionStyle;
  audioSource: "microphone" | "system";
  sourceLanguage: string;
  targetLanguage: string;
  translationEnabled: boolean;
  asrProvider: "local" | "cloud";
  translateProvider: "local" | "cloud";
  asrModel: "tiny" | "base" | "small" | "medium";
  clickThrough: boolean;
  uiLanguage: string;
  autoStartCaptioning: boolean;
}

export const DEFAULT_SETTINGS: AppSettings = {
  captionStyle: {
    fontFamily: "Inter, system-ui, sans-serif",
    fontSize: 28,
    fontWeight: 600,
    textColor: "#ffffff",
    backgroundColor: "#000000",
    backgroundOpacity: 0.55,
    outline: true,
    maxLines: 2,
    showOriginal: true,
    showTranslation: true,
  },
  audioSource: "microphone",
  sourceLanguage: "auto",
  targetLanguage: "es",
  translationEnabled: false,
  asrProvider: "local",
  translateProvider: "local",
  asrModel: "base",
  clickThrough: true,
  uiLanguage: "en",
  autoStartCaptioning: false,
};

const SETTINGS_KEY = "settings";
const SETTINGS_CHANGED_EVENT = "settings-changed";

let storePromise: Promise<Store> | null = null;
function getStore(): Promise<Store> {
  if (!storePromise) {
    storePromise = load("settings.json", { autoSave: true });
  }
  return storePromise;
}

function mergeWithDefaults(saved: Partial<AppSettings> | null): AppSettings {
  if (!saved) return DEFAULT_SETTINGS;
  return {
    ...DEFAULT_SETTINGS,
    ...saved,
    captionStyle: { ...DEFAULT_SETTINGS.captionStyle, ...saved.captionStyle },
  };
}

export async function loadSettings(): Promise<AppSettings> {
  const store = await getStore();
  const saved = await store.get<AppSettings>(SETTINGS_KEY);
  return mergeWithDefaults(saved ?? null);
}

export async function saveSettings(settings: AppSettings): Promise<void> {
  const store = await getStore();
  await store.set(SETTINGS_KEY, settings);
  await store.save();
  await emit(SETTINGS_CHANGED_EVENT, settings);
}

export function onSettingsChanged(cb: (settings: AppSettings) => void) {
  return listen<AppSettings>(SETTINGS_CHANGED_EVENT, (event) => cb(event.payload));
}
