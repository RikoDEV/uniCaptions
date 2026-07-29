import type { ComponentType } from "react";

type IconComponent = ComponentType<{ className?: string }>;

interface Option<T extends string> {
  value: T;
  label: string;
  icon?: IconComponent;
  description?: string;
  /** true = downloaded (shows a small badge), false = not yet downloaded, omit = n/a */
  downloaded?: boolean;
  /** 0-1 while this option is downloading; replaces the label with a progress bar */
  progress?: number | null;
}

interface Props<T extends string> {
  value: T;
  onChange: (value: T) => void;
  options: Option<T>[];
  wrap?: boolean;
}

export default function CardSelect<T extends string>({ value, onChange, options, wrap }: Props<T>) {
  return (
    <div className={wrap ? "card-select card-select-wrap" : "card-select"}>
      {options.map((opt) => {
        const isDownloading = opt.progress != null && opt.progress < 1;
        return (
          <button
            key={opt.value}
            type="button"
            className={opt.value === value ? "card-option card-option-active" : "card-option"}
            onClick={() => onChange(opt.value)}
            data-tooltip={opt.description}
          >
            {opt.downloaded && (
              <span className="card-option-badge" title="Downloaded">
                <svg viewBox="0 0 24 24" width="10" height="10" fill="none" stroke="currentColor" strokeWidth={2.5} strokeLinecap="round" strokeLinejoin="round">
                  <path d="M12 4v11" />
                  <path d="M7 11l5 5 5-5" />
                  <path d="M5 19h14" />
                </svg>
              </span>
            )}
            {opt.icon && <opt.icon className="card-option-icon" />}
            {isDownloading ? (
              <span className="card-option-progress">
                <span
                  className="card-option-progress-fill"
                  style={{ width: `${Math.round((opt.progress ?? 0) * 100)}%` }}
                />
              </span>
            ) : (
              <span>{opt.label}</span>
            )}
          </button>
        );
      })}
    </div>
  );
}
