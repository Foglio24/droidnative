import { useCallback, useEffect, useState } from "react";
import type { Shared } from "../App";
import { api, errorText } from "../api";
import type { GraphicsConfig } from "../types";
import { Button, Card, KeyValue, useAction } from "../ui";

export function Diagnostics(p: Shared) {
  const { busy, run } = useAction(p.setNotice);
  const [lines, setLines] = useState(200);
  const [logs, setLogs] = useState<string>("");
  const [gfx, setGfx] = useState<GraphicsConfig | null>(null);
  const running = p.status?.session === "running" || p.status?.session === "frozen";

  const loadLogs = useCallback(async () => {
    try {
      setLogs(await api.logs(lines));
    } catch (e) {
      setLogs(errorText(e));
    }
  }, [lines]);

  useEffect(() => {
    void loadLogs();
  }, [loadLogs]);

  const s = p.status;
  return (
    <div className="page">
      <h1>Diagnostica</h1>
      <div className="grid2">
        <Card title="Runtime">
          {p.statusError ? (
            <p className="muted">{p.statusError}</p>
          ) : (
            <KeyValue
              rows={[
                ["Backend", s?.backend ?? "—"],
                ["Sessione", s?.session ?? "—"],
                ["Container", s?.container ?? "—"],
                ["Vendor", s?.vendor_type ?? "—"],
                ["IP", s?.ip_address ?? "—"],
                ["Utente", s?.session_user ?? "—"],
                ["Display Wayland", s?.wayland_display ?? "—"],
              ]}
            />
          )}
        </Card>
        <Card
          title="Configurazione grafica di Android"
          actions={
            <Button disabled={!running} busy={busy === "gfx"} onClick={() => run("gfx", api.graphics).then((g) => g && setGfx(g))}>
              Leggi
            </Button>
          }
        >
          {gfx ? (
            <KeyValue
              rows={[
                ["ro.hardware.egl", gfx.egl],
                ["ro.hardware.gralloc", gfx.gralloc],
                ["ro.hardware.vulkan", gfx.vulkan],
                ["gralloc.gbm.device", gfx.gbm_device],
              ]}
            />
          ) : (
            <p className="muted">{running ? "Proprietà lette da Android (configurazione, non prova di accelerazione)." : "Disponibile con Android avviato."}</p>
          )}
        </Card>
      </div>
      <Card
        title="Log"
        actions={
          <>
            <select value={lines} onChange={(e) => setLines(Number(e.target.value))}>
              {[100, 200, 500, 1000].map((n) => (
                <option key={n} value={n}>
                  ultime {n} righe
                </option>
              ))}
            </select>
            <Button onClick={() => void loadLogs()}>Aggiorna</Button>
          </>
        }
      >
        <pre className="log">{logs || "…"}</pre>
      </Card>
    </div>
  );
}
