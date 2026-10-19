import { useCallback, useEffect, useState } from "react";
import type { Shared } from "../App";
import { api, errorText } from "../api";
import type { SetupStep, SetupView } from "../types";
import { Button, Card, Chip, ConfirmModal, useAction } from "../ui";

const STATE: Record<SetupStep["state"], [string, string]> = {
  done: ["pass", "Completato"],
  pending: ["warn", "Da eseguire"],
  blocked: ["unknown", "In attesa"],
  optional: ["info", "Facoltativo"],
};

export function Setup(p: Shared) {
  const { busy, run } = useAction(p.setNotice);
  const [view, setView] = useState<SetupView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [confirm, setConfirm] = useState<SetupStep | null>(null);

  const load = useCallback(async () => {
    try {
      setView(await api.setup());
      setError(null);
    } catch (e) {
      setError(errorText(e));
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const execute = async (step: SetupStep) => {
    setConfirm(null);
    await run(step.id, () => api.runSetupStep(step.id), (out) => `${step.title}: completato\n\n${out}`.trim());
    await Promise.all([load(), p.refreshReport(), p.refreshStatus()]);
  };

  return (
    <div className="page">
      <h1>Installazione guidata</h1>
      <p className="lead">
        Ogni passo mostra cosa farà prima di farlo. Le operazioni di sistema passano da polkit: il sistema chiede la password di amministratore e
        DroidNative esegue solo l'operazione indicata.
      </p>
      {error && <div className="notice notice-error"><pre>{error}</pre></div>}
      {!view && !error && <div className="loading">Calcolo dei passi…</div>}
      {view && (
        <ol className="steps">
          {view.steps.map((s, i) => {
            const [tone, label] = STATE[s.state];
            const runnable = s.action != null && (s.state === "pending" || s.state === "optional");
            return (
              <li key={s.id} className={`step step-${s.state}`}>
                <div className="step-num">{s.state === "done" ? "✓" : i + 1}</div>
                <Card
                  title={
                    <>
                      {s.title} <Chip tone={tone}>{label}</Chip>
                    </>
                  }
                  actions={
                    runnable ? (
                      <Button kind={s.state === "pending" ? "primary" : "secondary"} busy={busy === s.id} disabled={busy !== null} onClick={() => setConfirm(s)}>
                        Esegui
                      </Button>
                    ) : undefined
                  }
                >
                  <p className="muted">{s.detail}</p>
                  {busy === s.id && <p className="small">Operazione in corso: download e installazione possono richiedere diversi minuti.</p>}
                </Card>
              </li>
            );
          })}
        </ol>
      )}
      {view && <p className="muted small">Helper privilegiato: {view.helper}</p>}
      {confirm && (
        <ConfirmModal
          title={confirm.title}
          body={
            <>
              <p>DroidNative eseguirà come amministratore:</p>
              <pre className="code">{confirm.action}</pre>
              <p className="muted small">Ti verrà chiesta la password nella finestra di autenticazione del sistema.</p>
            </>
          }
          confirmLabel="Procedi"
          onCancel={() => setConfirm(null)}
          onConfirm={() => void execute(confirm)}
        />
      )}
    </div>
  );
}
