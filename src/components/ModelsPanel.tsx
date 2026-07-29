import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";

interface ModelEntry {
  id: string;
  downloaded: boolean;
  bytesOnDisk: number;
}

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 MB";
  const mb = bytes / (1024 * 1024);
  return mb >= 1024 ? `${(mb / 1024).toFixed(2)} GB` : `${mb.toFixed(1)} MB`;
}

export default function ModelsPanel() {
  const { t } = useTranslation();
  const [models, setModels] = useState<ModelEntry[] | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [downloadingId, setDownloadingId] = useState<string | null>(null);
  const [downloadingProgress, setDownloadingProgress] = useState(0);

  const refresh = async () => {
    const list = await invoke<ModelEntry[]>("list_models");
    setModels(list);
  };

  useEffect(() => {
    void refresh();
  }, []);

  useEffect(() => {
    const unlistenWhisper = listen<{ size: string; progress: number }>(
      "model-download-progress",
      (event) => {
        setDownloadingId(`whisper:${event.payload.size}`);
        setDownloadingProgress(event.payload.progress);
      },
    );
    const unlistenTranslate = listen<{ lang: string; progress: number }>(
      "translate-model-download-progress",
      (event) => {
        setDownloadingId(`translate:${event.payload.lang}`);
        setDownloadingProgress(event.payload.progress);
      },
    );
    return () => {
      unlistenWhisper.then((fn) => fn());
      unlistenTranslate.then((fn) => fn());
    };
  }, []);

  const deleteModel = async (id: string) => {
    setBusyId(id);
    try {
      await invoke("delete_model", { id });
      await refresh();
    } finally {
      setBusyId(null);
    }
  };

  const downloadModel = async (id: string) => {
    const [kind, value] = id.split(":");
    setDownloadingId(id);
    setDownloadingProgress(0);
    try {
      if (kind === "whisper") {
        await invoke("download_whisper_model", { size: value });
      } else {
        await invoke("download_translate_model", { lang: value });
      }
    } finally {
      setDownloadingId((current) => (current === id ? null : current));
      await refresh();
    }
  };

  if (!models) {
    return <p className="hint">{t("models.loading")}</p>;
  }

  const downloaded = models.filter((m) => m.downloaded);
  const totalBytes = downloaded.reduce((sum, m) => sum + m.bytesOnDisk, 0);

  const labelFor = (id: string) => {
    const [kind, value] = id.split(":");
    return kind === "whisper"
      ? t("models.whisperLabel", { size: value })
      : t("models.translateLabel", { lang: t(`languages.${value}`) });
  };

  return (
    <>
      <p className="hint">
        {t("models.intro")} {downloaded.length > 0 && t("models.inUse", { size: formatBytes(totalBytes) })}
      </p>
      <div className="model-list">
        {models.map((m) => {
          const isDownloading = downloadingId === m.id;
          return (
            <div key={m.id} className="model-row">
              <div className="model-row-info">
                <span className="model-row-label">{labelFor(m.id)}</span>
                {isDownloading ? (
                  <div className="level-meter model-row-progress">
                    <div
                      className="level-meter-fill"
                      style={{ width: `${Math.round(downloadingProgress * 100)}%` }}
                    />
                  </div>
                ) : (
                  <span className="model-row-status">
                    {m.downloaded ? formatBytes(m.bytesOnDisk) : t("models.notDownloaded")}
                  </span>
                )}
              </div>
              {m.downloaded ? (
                <button type="button" onClick={() => deleteModel(m.id)} disabled={busyId === m.id}>
                  {busyId === m.id ? t("models.deleting") : t("models.delete")}
                </button>
              ) : (
                <button
                  type="button"
                  className="button-accent"
                  onClick={() => downloadModel(m.id)}
                  disabled={isDownloading}
                >
                  {isDownloading ? t("models.downloading") : t("models.download")}
                </button>
              )}
            </div>
          );
        })}
      </div>
    </>
  );
}
