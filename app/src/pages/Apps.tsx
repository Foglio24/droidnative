import { useCallback, useEffect, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import type { Shared } from "../App";
import { api, bytes, errorText } from "../api";
import type { AndroidApp, ApkInfo } from "../types";
import { Button, Card, Chip, ConfirmModal, KeyValue, useAction } from "../ui";

const ABI_NOTE: Record<ApkInfo["abi_compat"], [string, string]> = {
  universal: ["pass", "Nessun codice nativo: compatibile"],
  native: ["pass", "Codice nativo per questa CPU"],
  needs_translation: ["warn", "Solo codice ARM: serve un layer di traduzione (libhoudini/libndk), non incluso in Waydroid"],
  incompatible: ["fail", "Codice nativo per un'architettura non supportata"],
};

export function Apps(p: Shared) {
  const { busy, run } = useAction(p.setNotice);
  const [apps, setApps] = useState<AndroidApp[] | null>(null);
  const [listError, setListError] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const [apk, setApk] = useState<ApkInfo | null>(null);
  const [removing, setRemoving] = useState<AndroidApp | null>(null);
  const running = p.status?.session === "running" || p.status?.session === "frozen";
  const ready = p.report!.host.waydroid.initialized;

  const load = useCallback(async () => {
    try {
      setApps(await api.apps());
      setListError(null);
    } catch (e) {
      setApps(null);
      setListError(errorText(e));
    }
  }, []);

  useEffect(() => {
    if (running) void load();
  }, [running, load]);

  const shown = useMemo(() => {
    const f = filter.trim().toLowerCase();
    const list = apps ?? [];
    return (f ? list.filter((a) => a.name.toLowerCase().includes(f) || a.package.toLowerCase().includes(f)) : list).sort((a, b) =>
      a.name.localeCompare(b.name),
    );
  }, [apps, filter]);

  const pick = async () => {
    const path = await open({ multiple: false, directory: false, filters: [{ name: "Pacchetto Android", extensions: ["apk"] }] });
    if (typeof path !== "string") return;
    const info = await run("inspect", () => api.inspectApk(path));
    if (info) setApk(info);
  };

  const install = async (info: ApkInfo) => {
    const out = await run("install", () => api.installApk(info.path, info.sha256), (o) =>
      `${o.package} ${o.was_update ? "aggiornato" : "installato"} — presenza verificata nell'elenco delle app`,
    );
    if (out) {
      setApk(null);
      await load();
    }
  };

  if (!running) {
    return (
      <div className="page">
        <h1>App Android</h1>
        <Card>
          <p>{ready ? "Avvia Android per vedere e gestire le app installate." : "Completa prima l'installazione guidata."}</p>
          {ready ? (
            <Button kind="primary" busy={busy === "start"} onClick={() => run("start", api.start, () => "Android è avviato").then(() => p.refreshStatus())}>
              Avvia Android
            </Button>
          ) : (
            <Button kind="primary" onClick={() => p.go("setup")}>
              Installazione guidata
            </Button>
          )}
        </Card>
      </div>
    );
  }

  return (
    <div className="page">
      <h1>App Android</h1>
      <Card
        title="Installa un APK"
        actions={
          <Button kind="primary" busy={busy === "inspect"} disabled={busy === "install"} onClick={() => void pick()}>
            Scegli file APK…
          </Button>
        }
      >
        {!apk && <p className="muted">Il file viene analizzato (manifest, ABI, permessi, SHA-256) prima di chiederti conferma.</p>}
        {apk && (
          <div className="apk">
            <KeyValue
              rows={[
                ["Package", <code>{apk.package}</code>],
                ["Nome", apk.label ?? "—"],
                ["Versione", `${apk.version_name ?? "?"} (${apk.version_code ?? "?"})`],
                ["SDK", `min ${apk.min_sdk ?? "?"} · target ${apk.target_sdk ?? "?"}`],
                ["Dimensione", bytes(apk.file_size)],
                ["ABI", apk.native_abis.length ? apk.native_abis.join(", ") : "nessuna"],
                ["SHA-256", <code className="hash">{apk.sha256}</code>],
              ]}
            />
            <p>
              <Chip tone={ABI_NOTE[apk.abi_compat][0]}>{apk.abi_compat.replace("_", " ")}</Chip> {ABI_NOTE[apk.abi_compat][1]}
            </p>
            {apk.min_sdk != null && apk.min_sdk > 33 && (
              <p>
                <Chip tone="warn">SDK</Chip> Richiede Android API {apk.min_sdk}: l'immagine Waydroid è Android 13 (API 33).
              </p>
            )}
            <details>
              <summary>{apk.permissions.length} permessi richiesti</summary>
              <ul className="small">{apk.permissions.map((x) => <li key={x}>{x}</li>)}</ul>
            </details>
            <div className="row">
              <Button kind="primary" busy={busy === "install"} disabled={apk.abi_compat === "incompatible"} onClick={() => void install(apk)}>
                Installa
              </Button>
              <Button disabled={busy === "install"} onClick={() => setApk(null)}>
                Annulla
              </Button>
            </div>
          </div>
        )}
      </Card>

      <Card
        title={`App installate${apps ? ` (${apps.length})` : ""}`}
        actions={
          <>
            <input className="search" placeholder="Cerca…" value={filter} onChange={(e) => setFilter(e.target.value)} />
            <Button onClick={() => void load()}>Aggiorna</Button>
          </>
        }
      >
        {listError && <div className="notice notice-error"><pre>{listError}</pre></div>}
        {!apps && !listError && <div className="loading">Lettura dell'elenco…</div>}
        {apps && (
          <table className="table">
            <thead>
              <tr>
                <th>Nome</th>
                <th>Package</th>
                <th />
              </tr>
            </thead>
            <tbody>
              {shown.map((a) => (
                <tr key={a.package}>
                  <td>{a.name}</td>
                  <td>
                    <code>{a.package}</code>
                  </td>
                  <td className="actions">
                    <Button busy={busy === `launch:${a.package}`} onClick={() => run(`launch:${a.package}`, () => api.launch(a.package))}>
                      Avvia
                    </Button>
                    <Button kind="danger" onClick={() => setRemoving(a)}>
                      Disinstalla
                    </Button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Card>

      {removing && (
        <ConfirmModal
          title={`Disinstallare ${removing.name}?`}
          body={<p>L'app <code>{removing.package}</code> e i suoi dati verranno rimossi da Android.</p>}
          confirmLabel="Disinstalla"
          danger
          onCancel={() => setRemoving(null)}
          onConfirm={() => {
            const target = removing;
            setRemoving(null);
            void run("remove", () => api.uninstall(target.package), (how) => `${target.package} rimosso e verificato (${how})`).then(load);
          }}
        />
      )}
    </div>
  );
}
