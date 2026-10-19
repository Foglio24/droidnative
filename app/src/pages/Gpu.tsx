import { useState } from "react";
import { mdiChevronDown, mdiCogOutline, mdiDownload, mdiLightbulbOnOutline, mdiRestart, mdiTune } from "@mdi/js";
import type { Shared } from "../App";
import { api } from "../api";
import { t, type Key } from "../i18n";
import type { BackendAssessment, GpuSelection, NvidiaMode } from "../types";
import { Button, Card, CheckList, Chip, Dialog, Icon, PageHead, useAction } from "../ui";

const READINESS_TONE: Record<BackendAssessment["readiness"], string> = {
  ready: "pass",
  needs_setup: "missing",
  unsupported: "fail",
};

const STABILITY_TONE: Record<BackendAssessment["descriptor"]["stability"], string | undefined> = {
  stable: "info",
  experimental: "warn",
  fallback: undefined,
};

type Pending = { label: string; detail: string; run: () => Promise<string> } | null;

export function Gpu(p: Shared) {
  const { busy, run } = useAction(p.setNotice);
  const [pending, setPending] = useState<Pending>(null);
  const r = p.report!;
  const w = r.host.waydroid;
  const cfg = w.config;
  const configured = cfg?.drm_device ?? null;
  const forcedSoftware = cfg?.properties.some(([k, v]) => k === "ro.hardware.egl" && v === "swiftshader") ?? false;
  const nvidiaInstalled = w.nvidia_setup != null;

  const select = (label: string, sel: GpuSelection) => setPending({ label, detail: t("dlg.gpu.cfgNote"), run: () => api.selectGpu(sel) });

  const apply = async (job: NonNullable<Pending>) => {
    setPending(null);
    await run("select", job.run, (out) => {
      const tail = out ? `\n\n${out.split("\n").slice(-10).join("\n")}` : "";
      return t("gpu.done", { label: job.label }) + tail;
    });
    p.setVerdict(null);
    await Promise.all([p.refreshReport(), p.refreshStatus()]);
  };

  const actionFor = (a: BackendAssessment) => {
    const notInit = !w.initialized;
    switch (a.descriptor.kind) {
      case "mesa_native": {
        const node = a.target_render_node;
        const active = node != null && configured === node && !forcedSoftware && !nvidiaInstalled;
        if (active) return <Chip tone="pass">{t("gpu.configured")}</Chip>;
        if (a.readiness === "unsupported") return <Chip tone="fail">{t("gpu.unavailable")}</Chip>;
        return (
          <Button
            kind="tonal"
            disabled={notInit || !node || nvidiaInstalled}
            busy={busy === "select"}
            onClick={() => node && select(t("gpu.label.mesa", { dev: a.target_device ?? "?", node }), { mode: "render_node", node })}
          >
            {t("gpu.use", { node: node ?? "GPU" })}
          </Button>
        );
      }
      case "software_fallback":
        if (forcedSoftware) return <Chip tone="pass">{t("gpu.configured")}</Chip>;
        return (
          <Button kind="tonal" disabled={notInit || nvidiaInstalled} busy={busy === "select"} onClick={() => select(t("gpu.label.software"), { mode: "software" })}>
            {t("gpu.forceSoftware")}
          </Button>
        );
      default: {
        const mode: NvidiaMode = a.descriptor.kind === "nvidia_accelerated" ? "venus" : "prolapse";
        const modeArg = mode === "venus" ? "venus" : "nvidia-prolapse";
        if (a.readiness === "unsupported") return <Chip tone="fail">{t("gpu.unavailable")}</Chip>;
        if (nvidiaInstalled && w.nvidia_mode === mode) return <Chip tone="pass">{t("gpu.active")}</Chip>;
        if (nvidiaInstalled)
          return (
            <Button
              kind="filled"
              busy={busy === "select"}
              onClick={() =>
                setPending({
                  label: t("gpu.label.mode", { name: a.descriptor.name, mode: modeArg }),
                  detail: t("dlg.gpu.modeNote"),
                  run: () => api.nvidiaSetMode(mode),
                })
              }
            >
              {t("gpu.activate")}
            </Button>
          );
        return (
          <Button
            kind="filled"
            icon={mdiDownload}
            busy={busy === "select"}
            onClick={() =>
              setPending({
                label: t("gpu.label.install", { name: a.descriptor.name }),
                detail: t("dlg.gpu.installNote"),
                run: () => api.nvidiaInstall(mode),
              })
            }
          >
            {t("gpu.installActivate")}
          </Button>
        );
      }
    }
  };

  let current: string;
  if (!w.initialized) current = t("gpu.cfg.notInit");
  else if (nvidiaInstalled)
    current = t("gpu.cfg.nvidia", {
      tag: w.nvidia_stack?.tag ?? "",
      mode: w.nvidia_mode === "prolapse" ? "nvidia-prolapse" : (w.nvidia_mode ?? "?"),
      node: configured ?? "?",
    });
  else if (forcedSoftware) current = t("gpu.cfg.software");
  else if (configured) current = t("gpu.cfg.node", { node: configured });
  else current = t("gpu.cfg.auto");

  const recommended = r.gpu_plan.assessments.find((a) => a.descriptor.kind === r.gpu_plan.recommended);

  return (
    <div className="page">
      <PageHead title={t("gpu.title")} />
      <Card
        title={t("gpu.current")}
        icon={mdiTune}
        actions={
          <Button
            kind="text"
            icon={mdiRestart}
            disabled={!w.initialized || nvidiaInstalled || (!configured && !forcedSoftware)}
            busy={busy === "select"}
            onClick={() => select(t("gpu.label.auto"), { mode: "auto" })}
          >
            {t("gpu.restoreAuto")}
          </Button>
        }
      >
        <p className="body-l" style={{ margin: 0 }}>
          {current}
        </p>
        <p className="small muted" style={{ marginBottom: 0 }}>
          {t("gpu.cfg.note")}
        </p>
        {busy === "select" && <div className="progress" />}
      </Card>

      <div className="recommend">
        <Icon path={mdiLightbulbOnOutline} />
        <div>
          <div className="title-m">
            {t("gpu.recommended")}: {recommended?.descriptor.name}
          </div>
          <div>{r.gpu_plan.reason}</div>
        </div>
      </div>

      {r.gpu_plan.assessments.map((a) => {
        const passing = a.checks.filter((c) => c.status === "pass").length;
        const notable = a.checks.filter((c) => c.status !== "pass");
        return (
          <Card key={a.descriptor.kind} icon={mdiCogOutline} title={a.descriptor.name} actions={actionFor(a)}>
            <div className="row" style={{ marginBottom: 10 }}>
              <Chip tone={STABILITY_TONE[a.descriptor.stability]}>{t(`stability.${a.descriptor.stability}` as Key)}</Chip>
              <Chip tone={READINESS_TONE[a.readiness]}>{t(`readiness.${a.readiness}` as Key)}</Chip>
              {a.descriptor.kind === r.gpu_plan.recommended && <Chip tone="pass">{t("gpu.chip.recommended")}</Chip>}
            </div>
            <p style={{ marginTop: 0 }}>{a.descriptor.summary}</p>
            {a.target_device && (
              <p className="small muted">{t("gpu.device", { dev: `${a.target_device}${a.target_render_node ? ` (${a.target_render_node})` : ""}` })}</p>
            )}
            {notable.length > 0 && <CheckList checks={notable} />}
            <details>
              <summary>
                {t("gpu.checks", { n: a.checks.length })} · {passing} OK
                <span className="chev" style={{ display: "inline-flex" }}>
                  <Icon path={mdiChevronDown} size={20} />
                </span>
              </summary>
              <CheckList checks={a.checks} />
              <div className="two-col">
                <div>
                  <div className="label">{t("gpu.requirements")}</div>
                  <ul>{a.descriptor.requirements.map((x) => <li key={x}>{x}</li>)}</ul>
                </div>
                <div>
                  <div className="label">{t("gpu.drivers")}</div>
                  <ul>{a.descriptor.compatible_drivers.map((x) => <li key={x}>{x}</li>)}</ul>
                </div>
              </div>
              <p className="small">
                {t("gpu.expected")}: <code>{a.descriptor.expected_renderer}</code>
              </p>
              {a.descriptor.upstream && (
                <p className="small muted">
                  {t("gpu.project")}: {a.descriptor.upstream}
                </p>
              )}
            </details>
          </Card>
        );
      })}

      {pending && (
        <Dialog title={t("dlg.gpu.title")} confirmLabel={t("common.continue")} onCancel={() => setPending(null)} onConfirm={() => void apply(pending)}>
          <p className="title-m" style={{ color: "var(--md-on-surface)" }}>
            {pending.label}
          </p>
          <p>{pending.detail}</p>
        </Dialog>
      )}
    </div>
  );
}
