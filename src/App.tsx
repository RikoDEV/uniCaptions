import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { isEnabled as isAutostartEnabled, enable as enableAutostart, disable as disableAutostart } from "@tauri-apps/plugin-autostart";
import { useSettings } from "./hooks/useSettings";
import { CaptionStyle } from "./lib/settings";
import { SUPPORTED_UI_LANGUAGES } from "./i18n";
import ApiKeyField from "./components/ApiKeyField";
import ModelsPanel from "./components/ModelsPanel";
import AboutPanel from "./components/AboutPanel";
import Toggle from "./components/Toggle";
import FontPicker from "./components/FontPicker";
import CardSelect from "./components/CardSelect";
import {
  IconGeneral,
  IconStyle,
  IconTranslate,
  IconModels,
  IconMic,
  IconSpeaker,
  IconOffline,
  IconCloud,
  IconInfo,
  IconTierTiny,
  IconTierBase,
  IconTierSmall,
  IconTierMedium,
} from "./components/icons";
import { FlagUS, FlagES, FlagFR, FlagDE, FlagPL, FlagPT, FlagCN, FlagJP, FlagGlobe } from "./components/flags";
import "./App.css";

const TABS = ["General", "Caption Style", "Translation", "Models", "About"] as const;
type Tab = (typeof TABS)[number];

const NAV_ITEMS: { id: Tab; labelKey: string; icon: typeof IconGeneral }[] = [
  { id: "General", labelKey: "nav.general", icon: IconGeneral },
  { id: "Caption Style", labelKey: "nav.captionStyle", icon: IconStyle },
  { id: "Translation", labelKey: "nav.translation", icon: IconTranslate },
  { id: "Models", labelKey: "nav.models", icon: IconModels },
  { id: "About", labelKey: "nav.about", icon: IconInfo },
];

const TAB_DESC_KEYS: Record<Tab, string> = {
  General: "tabDesc.general",
  "Caption Style": "tabDesc.captionStyle",
  Translation: "tabDesc.translation",
  Models: "tabDesc.models",
  About: "tabDesc.about",
};

const FLAGS: Record<string, typeof FlagUS> = {
  auto: FlagGlobe,
  en: FlagUS,
  es: FlagES,
  fr: FlagFR,
  de: FlagDE,
  pl: FlagPL,
  pt: FlagPT,
  zh: FlagCN,
  ja: FlagJP,
};

const LANGUAGE_CODES = ["auto", "en", "es", "fr", "de", "pl", "pt", "zh", "ja"];

// Local translation only ships pre-built ONNX models for these targets
// (Helsinki-NLP OPUS-MT en->X). Other targets need the cloud provider.
const LOCAL_TRANSLATION_TARGETS = new Set(["es", "fr", "de", "zh"]);

function App() {
  const { t, i18n } = useTranslation();
  const { settings, update, loaded } = useSettings();
  const [tab, setTab] = useState<Tab>("General");
  const [micActive, setMicActive] = useState(false);
  const [micLevel, setMicLevel] = useState(0);
  const [captioningActive, setCaptioningActive] = useState(false);
  const [captioningBusy, setCaptioningBusy] = useState(false);
  const [downloadProgress, setDownloadProgress] = useState<number | null>(null);
  const [autostart, setAutostart] = useState(false);
  const [whisperModelStatus, setWhisperModelStatus] = useState<Record<string, boolean>>({});
  const [downloadingModel, setDownloadingModel] = useState<string | null>(null);
  const [downloadingModelProgress, setDownloadingModelProgress] = useState(0);

  useEffect(() => {
    if (loaded) void i18n.changeLanguage(settings.uiLanguage);
  }, [loaded, settings.uiLanguage, i18n]);

  const languageOptions = (includeAuto: boolean) =>
    LANGUAGE_CODES.filter((c) => includeAuto || c !== "auto").map((code) => ({
      value: code,
      label: t(`languages.${code}`),
      icon: FLAGS[code],
    }));

  const refreshWhisperModels = () => {
    invoke<{ id: string; downloaded: boolean }[]>("list_models").then((models) => {
      const status: Record<string, boolean> = {};
      for (const m of models) {
        if (m.id.startsWith("whisper:")) {
          status[m.id.slice("whisper:".length)] = m.downloaded;
        }
      }
      setWhisperModelStatus(status);
    });
  };

  useEffect(() => {
    refreshWhisperModels();
  }, []);

  useEffect(() => {
    const unlistenPromise = listen<{ size: string; progress: number }>(
      "model-download-progress",
      (event) => {
        setDownloadingModel(event.payload.size);
        setDownloadingModelProgress(event.payload.progress);
        if (event.payload.progress >= 1) {
          setWhisperModelStatus((prev) => ({ ...prev, [event.payload.size]: true }));
          setDownloadingModel(null);
        }
      },
    );
    return () => {
      unlistenPromise.then((fn) => fn());
    };
  }, []);

  const selectAsrModel = async (v: typeof settings.asrModel) => {
    if (whisperModelStatus[v]) {
      update({ ...settings, asrModel: v });
      return;
    }
    update({ ...settings, asrModel: v });
    setDownloadingModel(v);
    setDownloadingModelProgress(0);
    try {
      await invoke("download_whisper_model", { size: v });
    } finally {
      refreshWhisperModels();
    }
  };

  useEffect(() => {
    isAutostartEnabled().then(setAutostart);
  }, []);

  const toggleAutostart = async (checked: boolean) => {
    if (checked) {
      await enableAutostart();
    } else {
      await disableAutostart();
    }
    setAutostart(checked);
  };

  useEffect(() => {
    const unlistenPromise = listen<number>("audio-level", (event) => {
      setMicLevel(event.payload);
    });
    return () => {
      unlistenPromise.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    const unlistenPromise = listen<{ size: string; progress: number }>(
      "model-download-progress",
      (event) => {
        setDownloadProgress(event.payload.progress);
      },
    );
    return () => {
      unlistenPromise.then((fn) => fn());
    };
  }, []);

  useEffect(() => {
    return () => {
      if (micActive) void invoke("stop_audio_capture");
      if (captioningActive) void invoke("stop_captioning");
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const toggleMic = async () => {
    if (micActive) {
      await invoke("stop_audio_capture");
      setMicActive(false);
      setMicLevel(0);
    } else {
      await invoke("start_audio_capture", { source: settings.audioSource });
      setMicActive(true);
    }
  };

  const toggleCaptioning = async () => {
    if (captioningActive) {
      await invoke("stop_captioning");
      setCaptioningActive(false);
      setDownloadProgress(null);
      return;
    }
    setCaptioningBusy(true);
    try {
      await invoke("start_captioning", {
        language: settings.sourceLanguage,
        targetLanguage: settings.translationEnabled ? settings.targetLanguage : null,
        audioSource: settings.audioSource,
        asrProvider: settings.asrProvider,
        asrModel: settings.asrModel,
        translateProvider: settings.translateProvider,
      });
      setCaptioningActive(true);
    } finally {
      setCaptioningBusy(false);
      setDownloadProgress(null);
    }
  };

  const hasAutoStarted = useRef(false);
  useEffect(() => {
    if (!loaded || hasAutoStarted.current || !settings.autoStartCaptioning) return;
    hasAutoStarted.current = true;
    void toggleCaptioning();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [loaded, settings.autoStartCaptioning]);

  if (!loaded) {
    return (
      <main className="container">
        <p>{t("common.loading")}</p>
      </main>
    );
  }

  const setStyle = (patch: Partial<CaptionStyle>) => {
    update({ ...settings, captionStyle: { ...settings.captionStyle, ...patch } });
  };

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <img src="/app-icon.png" alt="" className="brand-mark" />
          <div>
            <div className="brand-title">uniCaptions</div>
            <div className="brand-subtitle">{t("brand.subtitle")}</div>
          </div>
        </div>

        <nav className="nav">
          {NAV_ITEMS.map((item) => (
            <button
              key={item.id}
              className={item.id === tab ? "nav-item active" : "nav-item"}
              onClick={() => setTab(item.id)}
            >
              <item.icon className="nav-icon" />
              <span>{t(item.labelKey)}</span>
            </button>
          ))}
        </nav>

        <div className="sidebar-footer">
          <div className="sidebar-toggle-row">
            <div className="sidebar-toggle-label">
              <span className={captioningActive ? "status-dot status-live" : "status-dot"} />
              <span>
                {captioningBusy
                  ? t("sidebar.starting")
                  : captioningActive
                    ? t("sidebar.live")
                    : t("sidebar.idle")}
              </span>
            </div>
            <Toggle
              checked={captioningActive}
              onChange={() => void toggleCaptioning()}
              disabled={micActive || captioningBusy}
              label={t("sidebar.idle")}
            />
          </div>
          {downloadProgress !== null && downloadProgress < 1 && (
            <div className="level-meter">
              <div
                className="level-meter-fill"
                style={{ width: `${Math.round(downloadProgress * 100)}%` }}
              />
            </div>
          )}
        </div>
      </aside>

      <main className="content">
        <div className="content-inner">
          <header className="content-header">
            <h1>{t(NAV_ITEMS.find((n) => n.id === tab)!.labelKey)}</h1>
            <p className="subtitle">{t(TAB_DESC_KEYS[tab])}</p>
          </header>

          {tab === "General" && (
            <>
              <section className="panel">
                <span className="panel-title">{t("general.audioTitle")}</span>

                <div className="field">
                  <span>{t("general.audioSource")}</span>
                  <CardSelect
                    value={settings.audioSource}
                    onChange={(v) => update({ ...settings, audioSource: v })}
                    options={[
                      { value: "microphone", label: t("general.microphone"), icon: IconMic },
                      { value: "system", label: t("general.systemAudio"), icon: IconSpeaker },
                    ]}
                  />
                </div>

                <div className="field">
                  <span>
                    {t("general.audioTestLabel", {
                      source:
                        settings.audioSource === "microphone"
                          ? t("general.microphone")
                          : t("general.systemAudio"),
                    })}
                  </span>
                  <div className="field-row field-row-gap field-row-margin-bottom">
                    <button type="button" onClick={toggleMic} disabled={captioningActive}>
                      {micActive ? t("general.audioTestStop") : t("general.audioTestStart")}
                    </button>
                    <div className="level-meter">
                      <div
                        className="level-meter-fill"
                        style={{ width: `${Math.min(100, micLevel * 400)}%` }}
                      />
                    </div>
                  </div>
                  <p className="hint">{t("general.audioTestHint")}</p>
                </div>

                <div className="field">
                  <span>{t("general.spokenLanguage")}</span>
                  <CardSelect
                    value={settings.sourceLanguage}
                    onChange={(v) => update({ ...settings, sourceLanguage: v })}
                    options={languageOptions(true)}
                    wrap
                  />
                </div>
              </section>

              <section className="panel">
                <span className="panel-title">{t("general.speechTitle")}</span>

                <div className="field">
                  <span>{t("general.provider")}</span>
                  <CardSelect
                    value={settings.asrProvider}
                    onChange={(v) => update({ ...settings, asrProvider: v })}
                    options={[
                      { value: "local", label: t("general.providerLocal"), icon: IconOffline },
                      { value: "cloud", label: t("general.providerCloud"), icon: IconCloud },
                    ]}
                  />
                </div>

                {settings.asrProvider === "local" && (
                  <div className="field">
                    <span>{t("general.model")}</span>
                    <CardSelect
                      value={settings.asrModel}
                      onChange={selectAsrModel}
                      options={[
                        {
                          value: "tiny",
                          label: t("general.modelTiny"),
                          icon: IconTierTiny,
                          description: t("general.modelTinyDesc"),
                          downloaded: whisperModelStatus.tiny,
                          progress: downloadingModel === "tiny" ? downloadingModelProgress : null,
                        },
                        {
                          value: "base",
                          label: t("general.modelBase"),
                          icon: IconTierBase,
                          description: t("general.modelBaseDesc"),
                          downloaded: whisperModelStatus.base,
                          progress: downloadingModel === "base" ? downloadingModelProgress : null,
                        },
                        {
                          value: "small",
                          label: t("general.modelSmall"),
                          icon: IconTierSmall,
                          description: t("general.modelSmallDesc"),
                          downloaded: whisperModelStatus.small,
                          progress: downloadingModel === "small" ? downloadingModelProgress : null,
                        },
                        {
                          value: "medium",
                          label: t("general.modelMedium"),
                          icon: IconTierMedium,
                          description: t("general.modelMediumDesc"),
                          downloaded: whisperModelStatus.medium,
                          progress: downloadingModel === "medium" ? downloadingModelProgress : null,
                        },
                      ]}
                    />
                  </div>
                )}

                {settings.asrProvider === "cloud" && (
                  <ApiKeyField provider="openai_asr" label={t("general.openaiKeyLabel")} />
                )}

                <p className="hint">{t("general.speechHint")}</p>
              </section>

              <section className="panel">
                <span className="panel-title">{t("general.appBehaviorTitle")}</span>

                <div className="field">
                  <div className="field-row">
                    <span>{t("general.clickThrough")}</span>
                    <Toggle
                      checked={settings.clickThrough}
                      onChange={(v) => update({ ...settings, clickThrough: v })}
                      label={t("general.clickThrough")}
                    />
                  </div>
                  <p className="hint">{t("general.clickThroughHint")}</p>
                </div>

                <div className="field field-row">
                  <span>{t("general.launchAtStartup")}</span>
                  <Toggle checked={autostart} onChange={toggleAutostart} label={t("general.launchAtStartup")} />
                </div>

                <div className="field field-row">
                  <span>{t("general.autoStartCaptioning")}</span>
                  <Toggle
                    checked={settings.autoStartCaptioning}
                    onChange={(v) => update({ ...settings, autoStartCaptioning: v })}
                    label={t("general.autoStartCaptioning")}
                  />
                </div>

                <div className="field">
                  <span>{t("general.uiLanguage")}</span>
                  <CardSelect
                    value={settings.uiLanguage}
                    onChange={(v) => update({ ...settings, uiLanguage: v })}
                    options={SUPPORTED_UI_LANGUAGES.map((code) => ({
                      value: code,
                      label: t(`languages.${code}`),
                      icon: FLAGS[code],
                    }))}
                    wrap
                  />
                </div>
              </section>
            </>
          )}

          {tab === "Caption Style" && (
            <section className="panel">
              <div className="field">
                <span>{t("captionStyle.fontFamily")}</span>
                <FontPicker
                  value={settings.captionStyle.fontFamily}
                  onChange={(fontFamily) => setStyle({ fontFamily })}
                />
              </div>

              <label className="field">
                <span>{t("captionStyle.fontSize", { size: settings.captionStyle.fontSize })}</span>
                <input
                  type="range"
                  min={14}
                  max={64}
                  value={settings.captionStyle.fontSize}
                  onChange={(e) => setStyle({ fontSize: Number(e.target.value) })}
                />
              </label>

              <label className="field">
                <span>{t("captionStyle.fontWeight")}</span>
                <select
                  value={settings.captionStyle.fontWeight}
                  onChange={(e) => setStyle({ fontWeight: Number(e.target.value) })}
                >
                  <option value={400}>{t("captionStyle.weightRegular")}</option>
                  <option value={600}>{t("captionStyle.weightSemibold")}</option>
                  <option value={700}>{t("captionStyle.weightBold")}</option>
                  <option value={800}>{t("captionStyle.weightExtraBold")}</option>
                </select>
              </label>

              <label className="field field-row">
                <span>{t("captionStyle.textColor")}</span>
                <input
                  type="color"
                  value={settings.captionStyle.textColor}
                  onChange={(e) => setStyle({ textColor: e.target.value })}
                />
              </label>

              <label className="field field-row">
                <span>{t("captionStyle.backgroundColor")}</span>
                <input
                  type="color"
                  value={settings.captionStyle.backgroundColor}
                  onChange={(e) => setStyle({ backgroundColor: e.target.value })}
                />
              </label>

              <label className="field">
                <span>
                  {t("captionStyle.backgroundOpacity", {
                    opacity: Math.round(settings.captionStyle.backgroundOpacity * 100),
                  })}
                </span>
                <input
                  type="range"
                  min={0}
                  max={1}
                  step={0.05}
                  value={settings.captionStyle.backgroundOpacity}
                  onChange={(e) => setStyle({ backgroundOpacity: Number(e.target.value) })}
                />
              </label>

              <label className="field">
                <span>{t("captionStyle.maxLines", { lines: settings.captionStyle.maxLines })}</span>
                <input
                  type="range"
                  min={1}
                  max={5}
                  value={settings.captionStyle.maxLines}
                  onChange={(e) => setStyle({ maxLines: Number(e.target.value) })}
                />
              </label>

              <div className="field field-row">
                <span>{t("captionStyle.textOutline")}</span>
                <Toggle
                  checked={settings.captionStyle.outline}
                  onChange={(v) => setStyle({ outline: v })}
                  label={t("captionStyle.textOutline")}
                />
              </div>

              <p className="hint">{t("captionStyle.dragHint")}</p>
            </section>
          )}

          {tab === "Translation" && (
            <section className="panel">
              <div className="field field-row">
                <span>{t("translation.enable")}</span>
                <Toggle
                  checked={settings.translationEnabled}
                  onChange={(v) => update({ ...settings, translationEnabled: v })}
                  label={t("translation.enable")}
                />
              </div>

              <label className="field">
                <span>{t("translation.translateInto")}</span>
                <select
                  value={settings.targetLanguage}
                  onChange={(e) => update({ ...settings, targetLanguage: e.target.value })}
                  disabled={!settings.translationEnabled}
                >
                  {languageOptions(false).map((l) => (
                    <option key={l.value} value={l.value}>
                      {l.label}
                    </option>
                  ))}
                </select>
              </label>

              <label className="field">
                <span>{t("translation.provider")}</span>
                <select
                  value={settings.translateProvider}
                  onChange={(e) =>
                    update({
                      ...settings,
                      translateProvider: e.target.value as typeof settings.translateProvider,
                    })
                  }
                  disabled={!settings.translationEnabled}
                >
                  <option value="local">{t("translation.providerLocal")}</option>
                  <option value="cloud">{t("translation.providerCloud")}</option>
                </select>
              </label>

              <div className="field field-row">
                <span>{t("translation.showOriginal")}</span>
                <Toggle
                  checked={settings.captionStyle.showOriginal}
                  onChange={(v) => setStyle({ showOriginal: v })}
                  disabled={!settings.translationEnabled}
                  label={t("translation.showOriginal")}
                />
              </div>

              {settings.translateProvider === "cloud" && settings.translationEnabled && (
                <ApiKeyField provider="deepl_translate" label={t("translation.deeplKeyLabel")} />
              )}

              {settings.translateProvider === "local" &&
                settings.translationEnabled &&
                !LOCAL_TRANSLATION_TARGETS.has(settings.targetLanguage) && (
                  <p className="hint">{t("translation.localUnsupportedHint")}</p>
                )}

              <p className="hint">{t("translation.localAssumptionHint")}</p>
            </section>
          )}

          {tab === "Models" && (
            <section className="panel">
              <ModelsPanel />
            </section>
          )}

          {tab === "About" && (
            <>
              <section className="panel">
                <AboutPanel />
              </section>

              <section className="panel">
                <span className="credits-title">{t("credits.title")}</span>
                <ul className="credits-list">
                  <li>
                    <span>{t("credits.tauri")}</span>
                    <span className="hint">{t("credits.tauriDesc")}</span>
                  </li>
                  <li>
                    <span>{t("credits.whisperCpp")}</span>
                    <span className="hint">{t("credits.whisperCppDesc")}</span>
                  </li>
                  <li>
                    <span>{t("credits.onnx")}</span>
                    <span className="hint">{t("credits.onnxDesc")}</span>
                  </li>
                  <li>
                    <span>{t("credits.opusMt")}</span>
                    <span className="hint">{t("credits.opusMtDesc")}</span>
                  </li>
                  <li>
                    <span>{t("credits.react")}</span>
                    <span className="hint">{t("credits.reactDesc")}</span>
                  </li>
                </ul>
              </section>
            </>
          )}
        </div>
      </main>
    </div>
  );
}

export default App;
