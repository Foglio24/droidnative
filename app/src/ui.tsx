import { useCallback, useEffect, useState, type ReactNode } from "react";
import { mdiAlert, mdiCheckCircle, mdiClose, mdiCloseCircle, mdiHelpCircle, mdiInformation, mdiPackageDown } from "@mdi/js";
import { errorText } from "./api";
import { t, type Key } from "./i18n";
import type { Check, CheckStatus } from "./types";

export function Icon({ path, size = 24, title }: { path: string; size?: number; title?: string }) {
  return (
    <svg className="icon" width={size} height={size} viewBox="0 0 24 24" aria-hidden={title ? undefined : true} role={title ? "img" : undefined}>
      {title && <title>{title}</title>}
      <path d={path} fill="currentColor" />
    </svg>
  );
}

const STATUS_ICON: Record<CheckStatus, string> = {
  pass: mdiCheckCircle,
  info: mdiInformation,
  unknown: mdiHelpCircle,
  warn: mdiAlert,
  missing: mdiPackageDown,
  fail: mdiCloseCircle,
};

export const statusLabel = (s: CheckStatus) => t(`status.${s}` as Key);

export function Chip({ tone, icon, children }: { tone?: string; icon?: string; children: ReactNode }) {
  return (
    <span className={`chip ${tone ? `chip-${tone}` : ""} ${icon ? "has-icon" : ""}`}>
      {icon && <Icon path={icon} size={18} />}
      {children}
    </span>
  );
}

export function StatusChip({ status, label }: { status: CheckStatus; label?: string }) {
  return (
    <Chip tone={status === "unknown" ? undefined : status} icon={STATUS_ICON[status]}>
      {label ?? statusLabel(status)}
    </Chip>
  );
}

export function CheckList({ checks }: { checks: Check[] }) {
  return (
    <ul className="list">
      {checks.map((c, i) => (
        <li key={`${c.id}-${i}`} className="list-item">
          <span className={`lead-icon ${c.status}`} title={statusLabel(c.status)}>
            <Icon path={STATUS_ICON[c.status]} size={22} />
          </span>
          <div className="item-main">
            <div className="item-title">{c.label}</div>
            <div className="item-sub">{c.detail}</div>
            {c.remedy && <div className="item-remedy">{c.remedy}</div>}
          </div>
        </li>
      ))}
    </ul>
  );
}

export function Card({
  title,
  icon,
  actions,
  className,
  children,
}: {
  title?: ReactNode;
  icon?: string;
  actions?: ReactNode;
  className?: string;
  children: ReactNode;
}) {
  return (
    <section className={`card ${className ?? ""}`}>
      {(title || actions) && (
        <header className="card-head">
          {title && (
            <div className="card-title">
              {icon && <Icon path={icon} size={22} />}
              <h3 className="title-m">{title}</h3>
            </div>
          )}
          {actions && <div className="card-actions">{actions}</div>}
        </header>
      )}
      {children}
    </section>
  );
}

export function Button({
  onClick,
  busy,
  disabled,
  kind = "tonal",
  icon,
  children,
  title,
}: {
  onClick: () => void;
  busy?: boolean;
  disabled?: boolean;
  kind?: "filled" | "tonal" | "outlined" | "text" | "danger";
  icon?: string;
  children: ReactNode;
  title?: string;
}) {
  return (
    <button className={`btn btn-${kind} ${icon || busy ? "has-icon" : ""}`} onClick={onClick} disabled={disabled || busy} title={title}>
      {busy ? <span className="spinner" aria-hidden /> : icon && <Icon path={icon} size={18} />}
      {children}
    </button>
  );
}

export function IconButton({
  icon,
  label,
  onClick,
  danger,
  busy,
  disabled,
  selected,
}: {
  icon: string;
  label: string;
  onClick: () => void;
  danger?: boolean;
  busy?: boolean;
  disabled?: boolean;
  /** Makes it a toggle (Material 3 tonal icon button). */
  selected?: boolean;
}) {
  return (
    <button
      className={`icon-btn ${danger ? "danger" : ""} ${selected ? "selected" : ""}`}
      onClick={onClick}
      aria-label={label}
      aria-pressed={selected}
      title={label}
      disabled={disabled || busy}
    >
      {busy ? <span className="spinner" aria-hidden /> : <Icon path={icon} size={22} />}
    </button>
  );
}

export type Notice = { kind: "ok" | "error" | "info"; text: string } | null;

/** Errors stay as a banner until dismissed; confirmations are a snackbar that times out. */
export function NoticeArea({ notice, onClose }: { notice: Notice; onClose: () => void }) {
  useEffect(() => {
    if (notice && notice.kind !== "error") {
      const id = window.setTimeout(onClose, 7000);
      return () => window.clearTimeout(id);
    }
  }, [notice, onClose]);
  if (!notice) return null;
  if (notice.kind === "error") {
    return (
      <div className="banner" role="alert">
        <Icon path={mdiAlert} />
        <pre>{notice.text}</pre>
        <IconButton icon={mdiClose} label={t("common.dismiss")} onClick={onClose} />
      </div>
    );
  }
  return (
    <div className="snackbar" role="status">
      <pre>{notice.text}</pre>
      <IconButton icon={mdiClose} label={t("common.dismiss")} onClick={onClose} />
    </div>
  );
}

/** Runs one async action at a time, tracking which one is busy and reporting the outcome. */
export function useAction(setNotice: (n: Notice) => void) {
  const [busy, setBusy] = useState<string | null>(null);
  const run = useCallback(
    async <T,>(key: string, fn: () => Promise<T>, okText?: (r: T) => string | null): Promise<T | undefined> => {
      setBusy(key);
      try {
        const r = await fn();
        const text = okText?.(r);
        if (text) setNotice({ kind: "ok", text });
        return r;
      } catch (e) {
        setNotice({ kind: "error", text: errorText(e) });
        return undefined;
      } finally {
        setBusy(null);
      }
    },
    [setNotice],
  );
  return { busy, run };
}

export function Dialog({
  title,
  children,
  confirmLabel,
  danger,
  onConfirm,
  onCancel,
}: {
  title: string;
  children: ReactNode;
  confirmLabel: string;
  danger?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onCancel();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);
  return (
    <div className="scrim" onClick={onCancel}>
      <div className="dialog" role="dialog" aria-modal="true" aria-label={title} onClick={(e) => e.stopPropagation()}>
        <h2>{title}</h2>
        <div className="dialog-body">{children}</div>
        <div className="dialog-actions">
          <Button kind="text" onClick={onCancel}>
            {t("common.cancel")}
          </Button>
          <Button kind={danger ? "danger" : "filled"} onClick={onConfirm}>
            {confirmLabel}
          </Button>
        </div>
      </div>
    </div>
  );
}

export function KeyValue({ rows }: { rows: [string, ReactNode][] }) {
  return (
    <dl className="kv">
      {rows.map(([k, v]) => (
        <div key={k}>
          <dt>{k}</dt>
          <dd>{v ?? "—"}</dd>
        </div>
      ))}
    </dl>
  );
}

export function PageHead({ title, actions }: { title: string; actions?: ReactNode }) {
  return (
    <header className="page-head">
      <h1 className="headline">{title}</h1>
      {actions && <div className="card-actions">{actions}</div>}
    </header>
  );
}

export function Loading({ text }: { text: string }) {
  return (
    <div className="loading">
      <span className="spinner" aria-hidden />
      {text}
    </div>
  );
}
