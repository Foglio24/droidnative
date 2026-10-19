import { useCallback, useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { mdiCheck, mdiContentCopy, mdiGoogle, mdiIdentifier, mdiOpenInNew, mdiPlay } from "@mdi/js";
import type { Shared } from "../App";
import { api, errorText } from "../api";
import { t, type Key } from "../i18n";
import type { SetupStep, SetupView } from "../types";
import { Button, Card, Chip, Dialog, Icon, Loading, PageHead, useAction } from "../ui";

const UNCERTIFIED_URL = "https://www.google.com/android/uncertified";

function GoogleRegistration(p: Shared) {
  const { busy, run } = useAction(p.setNotice);
  const [id, setId] = useState<string | null>(null);
  return (
    <Card title={t("google.title")} icon={mdiGoogle}>
      <p style={{ marginTop: 0 }}>{t("google.body")}</p>
      <pre className="code" style={{ fontFamily: "inherit" }}>
        {t("google.steps")}
      </pre>
      {id && (
        <p className="title-m" style={{ marginBottom: 0 }}>
          {t("google.id", { id })}
        </p>
      )}
      <div className="row" style={{ marginTop: 14 }}>
        <Button kind="tonal" icon={mdiIdentifier} busy={busy === "id"} onClick={() => run("id", api.googleDeviceId).then((v) => v && setId(v))}>
          {t("google.show")}
        </Button>
        {id && (
          <Button kind="text" icon={mdiContentCopy} onClick={() => run("copy", () => navigator.clipboard.writeText(id), () => t("google.copied"))}>
            {t("google.copy")}
          </Button>
        )}
        <Button kind="filled" icon={mdiOpenInNew} onClick={() => run("open", () => openUrl(UNCERTIFIED_URL))}>
          {t("google.open")}
        </Button>
      </div>
    </Card>
  );
}

const TONE: Record<SetupStep["state"], string | undefined> = {
  done: "pass",
  pending: "missing",
  blocked: undefined,
  optional: "info",
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
    await run(step.id, () => api.runSetupStep(step.id), (out) => `${t("setup.completed", { title: step.title })}\n\n${out}`.trim());
    await Promise.all([load(), p.refreshReport(), p.refreshStatus()]);
  };

  return (
    <div className="page">
      <PageHead title={t("setup.title")} />
      <p className="lead">{t("setup.lead")}</p>
      {error && <p className="item-remedy">{error}</p>}
      {!view && !error && <Loading text={t("setup.computing")} />}
      {view && (
        <ol className="steps">
          {view.steps.map((s, i) => {
            const runnable = s.action != null && (s.state === "pending" || s.state === "optional");
            return (
              <li key={s.id} className={`step ${s.state}`}>
                <div className="step-num">{s.state === "done" ? <Icon path={mdiCheck} size={22} /> : i + 1}</div>
                <div className="item-main">
                  <div className="card-title">
                    <span className="title-m">{s.title}</span>
                    <Chip tone={TONE[s.state]}>{t(`state.${s.state}` as Key)}</Chip>
                  </div>
                  <div className="item-sub" style={{ marginTop: 4 }}>
                    {s.detail}
                  </div>
                  {busy === s.id && (
                    <>
                      <div className="progress" />
                      <p className="small muted" style={{ marginBottom: 0 }}>
                        {t("setup.running")}
                      </p>
                    </>
                  )}
                </div>
                {runnable && (
                  <Button kind={s.state === "pending" ? "filled" : "tonal"} icon={mdiPlay} busy={busy === s.id} disabled={busy !== null} onClick={() => setConfirm(s)}>
                    {t("btn.run")}
                  </Button>
                )}
              </li>
            );
          })}
        </ol>
      )}
      {p.report?.host.waydroid.system_image === "gapps" && <GoogleRegistration {...p} />}
      {view && <p className="small muted">{t("setup.helper", { path: view.helper })}</p>}
      {confirm && (
        <Dialog title={confirm.title} confirmLabel={t("common.continue")} onCancel={() => setConfirm(null)} onConfirm={() => void execute(confirm)}>
          <p>{t("dlg.run.intro")}</p>
          <pre className="code mono">{confirm.action}</pre>
          <p className="small">{t("dlg.run.password")}</p>
        </Dialog>
      )}
    </div>
  );
}
