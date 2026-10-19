import { useCallback, useState, type ReactNode } from "react";
import { errorText } from "./api";
import type { Check, CheckStatus } from "./types";

const STATUS_LABEL: Record<CheckStatus, string> = {
  pass: "OK",
  info: "Info",
  unknown: "?",
  warn: "Attenzione",
  missing: "Da installare",
  fail: "Non supportato",
};

export function Chip({ tone, children }: { tone: string; children: ReactNode }) {
  return <span className={`chip chip-${tone}`}>{children}</span>;
}

export function StatusChip({ status }: { status: CheckStatus }) {
  return <Chip tone={status}>{STATUS_LABEL[status]}</Chip>;
}

export function CheckList({ checks }: { checks: Check[] }) {
  return (
    <ul className="checks">
      {checks.map((c, i) => (
        <li key={`${c.id}-${i}`} className={`check check-${c.status}`}>
          <StatusChip status={c.status} />
          <div>
            <div className="check-label">{c.label}</div>
            <div className="check-detail">{c.detail}</div>
            {c.remedy && <div className="check-remedy">↳ {c.remedy}</div>}
          </div>
        </li>
      ))}
    </ul>
  );
}

export function Card({ title, actions, children }: { title?: ReactNode; actions?: ReactNode; children: ReactNode }) {
  return (
    <section className="card">
      {(title || actions) && (
        <header className="card-head">
          {title && <h3>{title}</h3>}
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
  kind = "secondary",
  children,
  title,
}: {
  onClick: () => void;
  busy?: boolean;
  disabled?: boolean;
  kind?: "primary" | "secondary" | "danger";
  children: ReactNode;
  title?: string;
}) {
  return (
    <button className={`btn btn-${kind}`} onClick={onClick} disabled={disabled || busy} title={title}>
      {busy && <span className="spinner" aria-hidden />}
      {children}
    </button>
  );
}

export type Notice = { kind: "ok" | "error" | "info"; text: string } | null;

export function NoticeBar({ notice, onClose }: { notice: Notice; onClose: () => void }) {
  if (!notice) return null;
  return (
    <div className={`notice notice-${notice.kind}`} role="status">
      <pre>{notice.text}</pre>
      <button className="notice-close" onClick={onClose} aria-label="Chiudi">
        ×
      </button>
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

export function ConfirmModal({
  title,
  body,
  confirmLabel,
  danger,
  onConfirm,
  onCancel,
}: {
  title: string;
  body: ReactNode;
  confirmLabel: string;
  danger?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  return (
    <div className="modal-backdrop" onClick={onCancel}>
      <div className="modal" role="dialog" aria-modal="true" onClick={(e) => e.stopPropagation()}>
        <h3>{title}</h3>
        <div className="modal-body">{body}</div>
        <div className="modal-actions">
          <Button onClick={onCancel}>Annulla</Button>
          <Button kind={danger ? "danger" : "primary"} onClick={onConfirm}>
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
