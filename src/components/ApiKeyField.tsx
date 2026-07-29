import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";

interface Props {
  provider: "openai_asr" | "deepl_translate";
  label: string;
}

export default function ApiKeyField({ provider, label }: Props) {
  const { t } = useTranslation();
  const [hasKey, setHasKey] = useState<boolean | null>(null);
  const [value, setValue] = useState("");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    invoke<boolean>("has_api_key", { provider }).then(setHasKey);
  }, [provider]);

  const save = async () => {
    if (!value.trim()) return;
    setSaving(true);
    try {
      await invoke("save_api_key", { provider, key: value.trim() });
      setHasKey(true);
      setValue("");
    } finally {
      setSaving(false);
    }
  };

  const clear = async () => {
    await invoke("delete_api_key", { provider });
    setHasKey(false);
  };

  return (
    <div className="field">
      <span>{label}</span>
      <div className="field-row">
        <input
          type="password"
          value={value}
          onChange={(e) => setValue(e.target.value)}
          placeholder={hasKey ? t("apiKey.placeholderSaved") : t("apiKey.placeholder")}
        />
        <button type="button" onClick={save} disabled={saving || !value.trim()}>
          {t("apiKey.save")}
        </button>
        {hasKey && (
          <button type="button" onClick={clear}>
            {t("apiKey.clear")}
          </button>
        )}
      </div>
      {hasKey !== null && (
        <p className="hint">{hasKey ? t("apiKey.savedHint") : t("apiKey.notSavedHint")}</p>
      )}
    </div>
  );
}
