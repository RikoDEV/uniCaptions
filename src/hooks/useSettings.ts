import { useCallback, useEffect, useState } from "react";
import {
  AppSettings,
  DEFAULT_SETTINGS,
  loadSettings,
  onSettingsChanged,
  saveSettings,
} from "../lib/settings";

export function useSettings() {
  const [settings, setSettings] = useState<AppSettings>(DEFAULT_SETTINGS);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    let unlisten: (() => void) | undefined;

    loadSettings().then((s) => {
      setSettings(s);
      setLoaded(true);
    });

    onSettingsChanged((s) => setSettings(s)).then((fn) => {
      unlisten = fn;
    });

    return () => unlisten?.();
  }, []);

  const update = useCallback((next: AppSettings) => {
    setSettings(next);
    void saveSettings(next);
  }, []);

  return { settings, update, loaded };
}
