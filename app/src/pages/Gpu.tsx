import { useState } from "react";
import type { Shared } from "../App";
import { api } from "../api";
import type { BackendAssessment, GpuSelection, NvidiaMode } from "../types";
import { Button, Card, CheckList, Chip, ConfirmModal, useAction } from "../ui";

const READINESS: Record<BackendAssessment["readiness"], [string, string]> = {
  ready: ["pass", "Pronto"],
  needs_setup: ["missing", "Da installare o attivare"],
  unsupported: ["fail", "Non supportato"],
};

const STABILITY: Record<BackendAssessment["descriptor"]["stability"], [string, string]> = {
  stable: ["info", "Stabile"],
  experimental: ["warn", "Sperimentale"],
  fallback: ["unknown", "Fallback"],
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

  const CFG_NOTE =
    "DroidNative aggiorna /var/lib/waydroid/waydroid.cfg (con copia di sicurezza), rigenera le proprietà Android e riavvia il container se è attivo. Le app aperte verranno chiuse. Verrà chiesta la password di amministratore.";
  const select = (label: string, sel: GpuSelection) => setPending({ label, detail: CFG_NOTE, run: () => api.selectGpu(sel) });

  const apply = async (job: NonNullable<Pending>) => {
    setPending(null);
    await run("select", job.run, (out) =>
      `${job.label}: fatto. Avvia Android e usa «Verifica ora» per confermare il renderer reale.${out ? "\n\n" + out.split("\n").slice(-12).join("\n") : ""}`,
    );
    p.setVerdict(null);
    await Promise.all([p.refreshReport(), p.refreshStatus()]);
  };

  const actionFor = (a: BackendAssessment) => {
    const notInit = !w.initialized;
    switch (a.descriptor.kind) {
      case "mesa_native": {
        const node = a.target_render_node;
        const active = node != null && configured === node && !forcedSoftware;
        return (
          <Button
            kind="primary"
            disabled={notInit || !node || a.readiness === "unsupported" || active || w.nvidia_setup != null}
            busy={busy === "select"}
            title={notInit ? "Inizializza prima Android" : undefined}
            onClick={() => node && select(`Mesa Native su ${a.target_device} (${node})`, { mode: "render_node", node })}
          >
            {active ? "Configurato" : `Usa ${node ?? "GPU"}`}
          </Button>
        );
      }
      case "software_fallback":
        return (
          <Button disabled={notInit || forcedSoftware || w.nvidia_setup != null} busy={busy === "select"} onClick={() => select("Rendering software (SwiftShader)", { mode: "software" })}>
            {forcedSoftware ? "Configurato" : "Forza software"}
          </Button>
        );
      default: {
        const mode: NvidiaMode = a.descriptor.kind === "nvidia_accelerated" ? "venus" : "prolapse";
        const modeArg = mode === "venus" ? "venus" : "nvidia-prolapse";
        if (a.readiness === "unsupported") return <Chip tone="fail">non disponibile</Chip>;
        if (w.nvidia_setup != null && w.nvidia_mode === mode) return <Chip tone="pass">Attivo (da verificare)</Chip>;
        if (w.nvidia_setup != null)
          return (
            <Button
              kind="primary"
              busy={busy === "select"}
              onClick={() =>
                setPending({
                  label: `${a.descriptor.name} (modalità ${modeArg})`,
                  detail: "waydroid-nvidia-setup cambia la modalità e riavvia il container. Verrà chiesta la password di amministratore.",
                  run: () => api.nvidiaSetMode(mode),
                })
              }
            >
              Attiva
            </Button>
          );
        return (
          <Button
            kind="primary"
            busy={busy === "select"}
            onClick={() =>
              setPending({
                label: `Installazione di waydroid-nvidia per ${a.descriptor.name}`,
                detail:
                  "DroidNative scarica il rilascio fissato (~260 MB) e ne verifica hash e commit, rimuove il pacchetto waydroid della distribuzione (immagini e dati Android restano), installa la Waydroid patchata con lo stack grafico NVIDIA e attiva la modalità scelta. Può richiedere diversi minuti. Verrà chiesta la password di amministratore.",
                run: () => api.nvidiaInstall(mode),
              })
            }
          >
            Installa e attiva…
          </Button>
        );
      }
    }
  };

  return (
    <div className="page">
      <h1>Backend GPU</h1>
      <Card
        title="Configurazione attuale"
        actions={
          <Button disabled={!w.initialized || w.nvidia_setup != null || (!configured && !forcedSoftware)} busy={busy === "select"} onClick={() => select("Scelta automatica di Waydroid", { mode: "auto" })}>
            Ripristina automatico
          </Button>
        }
      >
        <p>
          {!w.initialized && "Android non è ancora inizializzato."}
          {w.initialized && forcedSoftware && "Rendering software forzato (ro.hardware.egl=swiftshader)."}
          {w.initialized && w.nvidia_setup && `waydroid-nvidia ${w.nvidia_stack?.tag ?? ""} in modalità ${w.nvidia_mode ?? "?"} (nodo ${configured ?? "?"}).`}
          {w.initialized && !w.nvidia_setup && !forcedSoftware && configured && `Nodo GPU forzato: ${configured}.`}
          {w.initialized && !w.nvidia_setup && !forcedSoftware && !configured && "Scelta automatica di Waydroid: primo nodo render non NVIDIA."}
        </p>
        <p className="muted small">
          La configurazione dice cosa Android dovrebbe usare. L'accelerazione è considerata attiva solo dopo la verifica del renderer reale nella Panoramica.
        </p>
      </Card>

      <div className="recommend">
        Consigliato per questo computer: <strong>{r.gpu_plan.assessments.find((a) => a.descriptor.kind === r.gpu_plan.recommended)?.descriptor.name}</strong> —{" "}
        {r.gpu_plan.reason}
      </div>

      {r.gpu_plan.assessments.map((a) => {
        const [rt, rl] = READINESS[a.readiness];
        const [st, sl] = STABILITY[a.descriptor.stability];
        return (
          <Card
            key={a.descriptor.kind}
            title={
              <>
                {a.descriptor.name} <Chip tone={st}>{sl}</Chip> <Chip tone={rt}>{rl}</Chip>
                {a.descriptor.kind === r.gpu_plan.recommended && <Chip tone="pass">Consigliato</Chip>}
              </>
            }
            actions={actionFor(a)}
          >
            <p>{a.descriptor.summary}</p>
            {a.target_device && (
              <p className="muted small">
                Dispositivo: {a.target_device} {a.target_render_node && `(${a.target_render_node})`}
              </p>
            )}
            <CheckList checks={a.checks} />
            <details>
              <summary>Requisiti e driver compatibili</summary>
              <div className="two-col">
                <div>
                  <h4>Requisiti</h4>
                  <ul>{a.descriptor.requirements.map((x) => <li key={x}>{x}</li>)}</ul>
                </div>
                <div>
                  <h4>Driver compatibili</h4>
                  <ul>{a.descriptor.compatible_drivers.map((x) => <li key={x}>{x}</li>)}</ul>
                </div>
              </div>
              <p className="small">
                Renderer atteso: <code>{a.descriptor.expected_renderer}</code>
              </p>
              {a.descriptor.upstream && <p className="small muted">Progetto: {a.descriptor.upstream}</p>}
            </details>
          </Card>
        );
      })}

      {pending && (
        <ConfirmModal
          title="Cambiare backend GPU?"
          body={
            <>
              <p>
                <strong>{pending.label}</strong>
              </p>
              <p className="muted small">{pending.detail}</p>
            </>
          }
          confirmLabel="Procedi"
          onCancel={() => setPending(null)}
          onConfirm={() => void apply(pending)}
        />
      )}
    </div>
  );
}
