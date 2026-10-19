import { useState } from "react";
import type { Shared } from "../App";
import { api, bytes } from "../api";
import { Button, Card, Chip, ConfirmModal, KeyValue, useAction } from "../ui";

const SESSION_LABEL = { running: "Attivo", frozen: "Sospeso", stopped: "Fermo", unknown: "Sconosciuto" } as const;

export function Overview(p: Shared) {
  const { busy, run } = useAction(p.setNotice);
  const [confirmContainer, setConfirmContainer] = useState(false);
  const r = p.report!;
  const h = r.host;
  const w = h.waydroid;
  const session = p.status?.session ?? "unknown";
  const running = session === "running" || session === "frozen";
  const recommended = r.gpu_plan.assessments.find((a) => a.descriptor.kind === r.gpu_plan.recommended);
  const ready = w.installed && w.initialized;

  const after = async () => {
    await p.refreshStatus();
  };

  return (
    <div className="page">
      <h1>Panoramica</h1>
      <div className="grid3">
        <Card title="Questo computer">
          <KeyValue
            rows={[
              ["Sistema", `${h.os.pretty_name}${h.os.base_release ? ` · ${h.os.base_release}` : ""}`],
              ["CPU", `${h.cpu.model ?? "?"} · ${h.cpu.logical_cpus} thread`],
              ["RAM", `${bytes(h.memory.total_bytes)} (${bytes(h.memory.available_bytes)} liberi)`],
              ["Kernel", h.kernel.release],
              ["Sessione", `${h.session.kind === "wayland" ? "Wayland" : h.session.kind} · ${h.session.compositor ?? h.session.desktop ?? "?"}`],
            ]}
          />
        </Card>
        <Card title="Grafica">
          <ul className="plain">
            {h.gpu.devices.map((d) => (
              <li key={d.pci_slot ?? d.name}>
                <strong>{d.name ?? "GPU"}</strong>{" "}
                <span className="muted">
                  {d.kernel_driver} · {d.render_node ?? "—"}
                </span>{" "}
                {d.connected_outputs.length > 0 && <Chip tone="info">display</Chip>}
              </li>
            ))}
          </ul>
          <p className="muted small">
            Backend consigliato: <strong>{recommended?.descriptor.name}</strong>
          </p>
        </Card>
        <Card title="Android">
          <KeyValue
            rows={[
              ["Runtime", w.installed ? `Waydroid ${w.version ?? ""}` : <Chip tone="missing">non installato</Chip>],
              ["Immagini", w.initialized ? `${w.images.length} presenti` : <Chip tone="missing">da scaricare</Chip>],
              ["Container", w.container_service.active ?? "—"],
              ["Sessione", p.statusError ? "—" : <Chip tone={running ? "pass" : "info"}>{SESSION_LABEL[session]}</Chip>],
              ["IP", p.status?.ip_address ?? "—"],
            ]}
          />
        </Card>
      </div>

      {!ready && (
        <Card title="Prima di iniziare">
          <p>Android non è ancora pronto su questo computer. L'installazione guidata esegue i passi mancanti e chiede la password di amministratore solo quando serve.</p>
          <Button kind="primary" onClick={() => p.go("setup")}>
            Apri l'installazione guidata
          </Button>
        </Card>
      )}

      <Card title="Sessione Android">
        <div className="row">
          <Button kind="primary" disabled={!ready || running} busy={busy === "start"} onClick={() => run("start", api.start, () => "Android è avviato").then(after)}>
            Avvia
          </Button>
          <Button disabled={!running} busy={busy === "stop"} onClick={() => run("stop", api.stop, () => "Android è stato arrestato").then(after)}>
            Arresta
          </Button>
          <Button disabled={!ready} busy={busy === "restart"} onClick={() => run("restart", api.restart, () => "Sessione riavviata").then(after)}>
            Riavvia
          </Button>
          <Button disabled={!running} busy={busy === "ui"} onClick={() => run("ui", api.showUi)}>
            Mostra Android
          </Button>
          <Button disabled={!w.installed} busy={busy === "container"} onClick={() => setConfirmContainer(true)}>
            Riavvia container…
          </Button>
        </div>
        {busy === "start" && <p className="muted small">Avvio del container e boot di Android: può richiedere fino a un paio di minuti al primo avvio.</p>}
      </Card>

      <Card
        title="Accelerazione GPU"
        actions={
          <Button disabled={!running} busy={busy === "verify"} onClick={() => run("verify", api.verifyGpu).then((v) => v && p.setVerdict(v))}>
            Verifica ora
          </Button>
        }
      >
        {!p.verdict && (
          <p>
            <Chip tone="unknown">Non verificata</Chip>{" "}
            <span className="muted">
              {running
                ? "DroidNative legge il renderer reale di SurfaceFlinger (richiede autenticazione)."
                : "Avvia Android per verificare quale GPU viene usata davvero."}
            </span>
          </p>
        )}
        {p.verdict && (
          <>
            <p>
              {p.verdict.status === "hardware_verified" && <Chip tone="pass">Accelerazione hardware verificata</Chip>}
              {p.verdict.status === "software" && <Chip tone="fail">Rendering software</Chip>}
              {p.verdict.status === "unknown" && <Chip tone="unknown">Non determinabile</Chip>} {p.verdict.detail}
            </p>
            {p.verdict.gles_line && <pre className="code">{p.verdict.gles_line}</pre>}
            <p>
              {p.verdict.display.status === "ok" && <Chip tone="pass">Visualizzazione sul desktop OK</Chip>}
              {p.verdict.display.status === "failed" && <Chip tone="fail">Visualizzazione sul desktop fallita</Chip>}
              {p.verdict.display.status === "unknown" && <Chip tone="unknown">Visualizzazione non verificata</Chip>} {p.verdict.display.detail}
            </p>
            {p.verdict.display.evidence.length > 0 && <pre className="code">{p.verdict.display.evidence.join("\n")}</pre>}
            {p.verdict.status === "hardware_verified" && p.verdict.display.status === "failed" && (
              <p className="small">
                Android usa la GPU, ma il compositor del desktop non riesce a mostrarne i buffer: le app si chiudono appena aprono una finestra. Scegli un
                altro backend nella pagina GPU.
              </p>
            )}
          </>
        )}
      </Card>

      {confirmContainer && (
        <ConfirmModal
          title="Riavviare il container?"
          body={<p>Le app Android in esecuzione verranno chiuse. Verrà chiesta la password di amministratore.</p>}
          confirmLabel="Riavvia container"
          onCancel={() => setConfirmContainer(false)}
          onConfirm={() => {
            setConfirmContainer(false);
            void run("container", api.restartContainer, () => "Container riavviato").then(after);
          }}
        />
      )}
    </div>
  );
}
