import { useCallback, useEffect, useState } from "react";
import { mdiAndroid, mdiExpansionCardVariant, mdiFileDocumentOutline, mdiRefresh } from "@mdi/js";
import type { Shared } from "../App";
import { api, errorText } from "../api";
import { t } from "../i18n";
import type { GraphicsConfig } from "../types";
import { Button, Card, IconButton, KeyValue, PageHead, useAction } from "../ui";

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
      <PageHead title={t("diag.title")} />
      <div className="grid2">
        <Card title={t("diag.runtime")} icon={mdiAndroid}>
          {p.statusError ? (
            <p className="muted">{p.statusError}</p>
          ) : (
            <KeyValue
              rows={[
                [t("diag.backend"), s?.backend ?? "—"],
                [t("diag.session"), s?.session ?? "—"],
                [t("kv.container"), s?.container ?? "—"],
                [t("diag.vendor"), s?.vendor_type ?? "—"],
                [t("kv.ip"), s?.ip_address ?? "—"],
                [t("diag.user"), s?.session_user ?? "—"],
                [t("diag.wayland"), s?.wayland_display ?? "—"],
              ]}
            />
          )}
        </Card>
        <Card
          title={t("diag.gfx")}
          icon={mdiExpansionCardVariant}
          actions={
            <Button kind="tonal" disabled={!running} busy={busy === "gfx"} onClick={() => run("gfx", api.graphics).then((g) => g && setGfx(g))}>
              {t("diag.read")}
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
            <p className="muted">{running ? t("diag.gfx.hintRunning") : t("diag.gfx.hintStopped")}</p>
          )}
        </Card>
      </div>
      <Card
        title={t("diag.logs")}
        icon={mdiFileDocumentOutline}
        actions={
          <>
            <select value={lines} onChange={(e) => setLines(Number(e.target.value))}>
              {[100, 200, 500, 1000].map((n) => (
                <option key={n} value={n}>
                  {t("diag.lines", { n })}
                </option>
              ))}
            </select>
            <IconButton icon={mdiRefresh} label={t("common.refresh")} onClick={() => void loadLogs()} />
          </>
        }
      >
        <pre className="log mono">{logs || "…"}</pre>
      </Card>
    </div>
  );
}
