import { useState } from "react";
import {
  mdiAndroid,
  mdiCheckDecagram,
  mdiChip,
  mdiLaptop,
  mdiMonitor,
  mdiPackageVariantPlus,
  mdiPlay,
  mdiRestart,
  mdiRocketLaunchOutline,
  mdiShieldCheckOutline,
  mdiStop,
} from "@mdi/js";
import type { Shared } from "../App";
import { api } from "../api";
import { formatBytes, t, type Key } from "../i18n";
import { Button, Card, Chip, Dialog, Icon, KeyValue, useAction } from "../ui";

export function Overview(p: Shared) {
  const { busy, run } = useAction(p.setNotice);
  const [confirmContainer, setConfirmContainer] = useState(false);
  const r = p.report!;
  const h = r.host;
  const w = h.waydroid;
  const session = p.status?.session ?? "unknown";
  const running = session === "running" || session === "frozen";
  const ready = w.installed && w.initialized;
  const recommended = r.gpu_plan.assessments.find((a) => a.descriptor.kind === r.gpu_plan.recommended);
  const after = () => p.refreshStatus();

  const hero = !ready ? "setup" : running ? "running" : "ready";
  const sessionLabel = p.statusError ? t("session.unavailable") : t(`session.${session}` as Key);

  return (
    <div className="page">
      <section className={`card hero ${hero === "running" ? "" : "idle"}`}>
        <div className="hero-row">
          <div className="hero-icon">
            <Icon path={hero === "setup" ? mdiRocketLaunchOutline : mdiAndroid} size={40} />
          </div>
          <div className="hero-text">
            <h1 className="headline">{t(`ov.hero.${hero}.title` as Key)}</h1>
            <p className="body-l muted" style={{ margin: "4px 0 0" }}>
              {t(`ov.hero.${hero}.body` as Key)}
            </p>
          </div>
        </div>
        <div className="row">
          {hero === "setup" && (
            <Button kind="filled" icon={mdiRocketLaunchOutline} onClick={() => p.go("setup")}>
              {t("btn.openSetup")}
            </Button>
          )}
          {hero === "ready" && (
            <Button kind="filled" icon={mdiPlay} busy={busy === "start"} onClick={() => run("start", api.start, () => t("toast.started")).then(after)}>
              {t("btn.start")}
            </Button>
          )}
          {hero === "running" && (
            <>
              <Button
                kind="filled"
                icon={mdiPackageVariantPlus}
                onClick={() => {
                  p.setPickApk(true);
                  p.go("apps");
                }}
              >
                {t("btn.installApk")}
              </Button>
              <Button kind="tonal" icon={mdiMonitor} busy={busy === "ui"} onClick={() => run("ui", api.showUi)}>
                {t("btn.showUi")}
              </Button>
              <Button kind="tonal" icon={mdiStop} busy={busy === "stop"} onClick={() => run("stop", api.stop, () => t("toast.stopped")).then(after)}>
                {t("btn.stop")}
              </Button>
            </>
          )}
          {ready && (
            <Button kind="text" icon={mdiRestart} busy={busy === "restart"} onClick={() => run("restart", api.restart, () => t("toast.restarted")).then(after)}>
              {t("btn.restart")}
            </Button>
          )}
        </div>
        {busy === "start" && (
          <>
            <div className="progress" />
            <p className="small muted" style={{ marginBottom: 0 }}>
              {t("ov.starting")}
            </p>
          </>
        )}
      </section>

      <Card
        title={t("ov.gpu.title")}
        icon={mdiShieldCheckOutline}
        actions={
          <Button kind="outlined" icon={mdiCheckDecagram} disabled={!running} busy={busy === "verify"} onClick={() => run("verify", api.verifyGpu).then((v) => v && p.setVerdict(v))}>
            {t("ov.gpu.verify")}
          </Button>
        }
      >
        {!p.verdict ? (
          <div className="row">
            <Chip>{t("ov.gpu.notVerified")}</Chip>
            <span className="muted">{running ? t("ov.gpu.hintRunning") : t("ov.gpu.hintStopped")}</span>
          </div>
        ) : (
          <>
            <div className="row">
              {p.verdict.status === "hardware_verified" && <Chip tone="pass" icon={mdiCheckDecagram}>{t("ov.gpu.hw")}</Chip>}
              {p.verdict.status === "software" && <Chip tone="fail">{t("ov.gpu.sw")}</Chip>}
              {p.verdict.status === "unknown" && <Chip>{t("ov.gpu.unknown")}</Chip>}
              {p.verdict.display.status === "ok" && <Chip tone="pass" icon={mdiMonitor}>{t("ov.display.ok")}</Chip>}
              {p.verdict.display.status === "failed" && <Chip tone="fail" icon={mdiMonitor}>{t("ov.display.failed")}</Chip>}
              {p.verdict.display.status === "unknown" && <Chip icon={mdiMonitor}>{t("ov.display.unknown")}</Chip>}
              <span className="muted">{p.verdict.detail}</span>
            </div>
            {p.verdict.gles_line && <pre className="code mono">{p.verdict.gles_line}</pre>}
            <p className="muted" style={{ marginBottom: 0 }}>
              {p.verdict.display.detail}
            </p>
            {p.verdict.display.evidence.length > 0 && <pre className="code mono">{p.verdict.display.evidence.join("\n")}</pre>}
            {p.verdict.status === "hardware_verified" && p.verdict.display.status === "failed" && <p className="item-remedy">{t("ov.display.failedHint")}</p>}
          </>
        )}
      </Card>

      <div className="grid3">
        <section className="card">
          <div className="tile-head">
            <Icon path={mdiLaptop} />
            <h3 className="title-m">{t("ov.system")}</h3>
          </div>
          <KeyValue
            rows={[
              [t("kv.system"), `${h.os.pretty_name}${h.os.base_release ? ` · ${h.os.base_release}` : ""}`],
              [t("kv.cpu"), `${h.cpu.model ?? "?"} · ${t("ov.threads", { n: h.cpu.logical_cpus })}`],
              [t("kv.ram"), `${formatBytes(h.memory.total_bytes)} (${t("ov.free", { v: formatBytes(h.memory.available_bytes) })})`],
              [t("kv.kernel"), h.kernel.release],
              [t("kv.session"), `${h.session.kind === "wayland" ? "Wayland" : h.session.kind} · ${h.session.compositor ?? h.session.desktop ?? "?"}`],
            ]}
          />
        </section>
        <section className="card">
          <div className="tile-head">
            <Icon path={mdiChip} />
            <h3 className="title-m">{t("ov.graphics")}</h3>
          </div>
          <ul className="list">
            {h.gpu.devices.map((d) => (
              <li key={d.pci_slot ?? d.name} className="list-item center" style={{ padding: "8px 0" }}>
                <div className="item-main">
                  <div className="item-title">{d.name ?? "GPU"}</div>
                  <div className="item-sub small">
                    {d.kernel_driver} · {d.render_node ?? "—"}
                  </div>
                </div>
                {d.connected_outputs.length > 0 && <Chip tone="info">{t("ov.displayChip")}</Chip>}
              </li>
            ))}
          </ul>
          <p className="small muted" style={{ marginBottom: 0 }}>
            {t("ov.recommended", { name: recommended?.descriptor.name ?? "?" })}
          </p>
        </section>
        <section className="card">
          <div className="tile-head">
            <Icon path={mdiAndroid} />
            <h3 className="title-m">{t("android.label")}</h3>
          </div>
          <KeyValue
            rows={[
              [t("kv.runtime"), w.installed ? `Waydroid ${w.version ?? ""}` : <Chip tone="missing">{t("ov.notInstalled")}</Chip>],
              [t("kv.images"), w.initialized ? t("ov.imagesPresent", { n: w.images.length }) : <Chip tone="missing">{t("ov.toDownload")}</Chip>],
              [t("kv.container"), w.container_service.active ?? "—"],
              [t("kv.session"), p.statusError ? "—" : <Chip tone={running ? "pass" : undefined}>{sessionLabel}</Chip>],
              [t("kv.ip"), p.status?.ip_address ?? "—"],
            ]}
          />
          {w.installed && (
            <div className="row" style={{ marginTop: 14 }}>
              <Button kind="text" icon={mdiRestart} busy={busy === "container"} onClick={() => setConfirmContainer(true)}>
                {t("btn.restartContainer")}
              </Button>
            </div>
          )}
        </section>
      </div>

      {confirmContainer && (
        <Dialog
          title={t("dlg.container.title")}
          confirmLabel={t("dlg.container.confirm")}
          onCancel={() => setConfirmContainer(false)}
          onConfirm={() => {
            setConfirmContainer(false);
            void run("container", api.restartContainer, () => t("toast.containerRestarted")).then(after);
          }}
        >
          <p>{t("dlg.container.body")}</p>
        </Dialog>
      )}
    </div>
  );
}
