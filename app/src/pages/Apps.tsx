import { useCallback, useEffect, useMemo, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { convertFileSrc } from "@tauri-apps/api/core";
import {
  mdiDeleteOutline,
  mdiDockWindow,
  mdiFolderOpenOutline,
  mdiGestureTap,
  mdiMagnify,
  mdiOpenInNew,
  mdiPackageVariantPlus,
  mdiPlay,
  mdiRefresh,
  mdiRocketLaunchOutline,
  mdiWindowRestore,
} from "@mdi/js";
import type { Shared } from "../App";
import { api, errorText } from "../api";
import { formatBytes, t, type Key } from "../i18n";
import type { AndroidApp, ApkInfo, WindowMode } from "../types";
import { Button, Card, Chip, Dialog, Icon, IconButton, KeyValue, Loading, PageHead, useAction } from "../ui";

const ABI_TONE: Record<ApkInfo["abi_compat"], string> = {
  universal: "pass",
  native: "pass",
  needs_translation: "warn",
  translated: "info",
  incompatible: "fail",
};

function WindowModeCard(p: Shared) {
  const { busy, run } = useAction(p.setNotice);
  const [mode, setMode] = useState<WindowMode | null>(null);
  const [asking, setAsking] = useState<WindowMode | null>(null);

  useEffect(() => {
    api.windowMode().then(setMode, () => setMode(null));
  }, []);

  const change = async (m: WindowMode) => {
    setAsking(null);
    const ok = await run("windows", async () => (await api.setWindowMode(m), true), () => t("windows.changed"));
    if (ok) {
      setMode(m);
      await p.refreshStatus();
    }
  };

  const w = p.report?.host.waydroid;
  const compositor = p.report?.host.session.compositor ?? "?";
  // waydroid-nvidia's per-app windows leave trails outside KWin (see nvidia_stack::per_app_windows_supported).
  const multiBlocked = !!w?.nvidia_setup && compositor !== "KWin";

  const option = (m: WindowMode, icon: string, label: string) => (
    <button
      aria-pressed={mode === m}
      disabled={busy !== null || mode === null || (m === "multi" && multiBlocked)}
      onClick={() => mode !== m && setAsking(m)}
    >
      <Icon path={icon} size={20} />
      {label}
    </button>
  );

  return (
    <Card title={t("windows.title")} icon={mdiWindowRestore}>
      <div className="segmented">
        {option("single", mdiWindowRestore, t("windows.single"))}
        {option("multi", mdiDockWindow, t("windows.multi"))}
      </div>
      <p className="small muted" style={{ marginBottom: 0 }}>
        {mode === null ? t("windows.unknown") : t(mode === "multi" ? "windows.multi.desc" : "windows.single.desc")}
      </p>
      {multiBlocked && (
        <p className="small" style={{ marginBottom: 0 }}>
          {t(mode === "multi" ? "windows.multi.leave" : "windows.multi.unsupported", { compositor })}
        </p>
      )}
      {busy === "windows" && <div className="progress" />}
      {asking && (
        <Dialog title={t("windows.confirm.title")} confirmLabel={t("common.continue")} onCancel={() => setAsking(null)} onConfirm={() => void change(asking)}>
          <p>{t("windows.confirm.body")}</p>
        </Dialog>
      )}
    </Card>
  );
}

function AppAvatar({ app }: { app: AndroidApp }) {
  const [broken, setBroken] = useState(false);
  if (app.icon && !broken) {
    return <img className="app-avatar" src={convertFileSrc(app.icon)} alt="" onError={() => setBroken(true)} />;
  }
  return <span className="app-avatar">{app.name.slice(0, 1).toUpperCase()}</span>;
}

export function Apps(p: Shared) {
  const { busy, run } = useAction(p.setNotice);
  const [apps, setApps] = useState<AndroidApp[] | null>(null);
  const [listError, setListError] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const [apk, setApk] = useState<ApkInfo | null>(null);
  const [removing, setRemoving] = useState<AndroidApp | null>(null);
  const [touch, setTouch] = useState<string[]>([]);
  const running = p.status?.session === "running" || p.status?.session === "frozen";
  const ready = p.report!.host.waydroid.initialized;

  const load = useCallback(async () => {
    try {
      setApps(await api.apps());
      setListError(null);
      setTouch(await api.fakeTouch().catch(() => []));
    } catch (e) {
      setApps(null);
      setListError(errorText(e));
    }
  }, []);

  useEffect(() => {
    if (running) void load();
  }, [running, load]);

  const pick = useCallback(async () => {
    const path = await open({ multiple: false, directory: false, filters: [{ name: t("apps.filter"), extensions: ["apk"] }] });
    if (typeof path !== "string") return;
    const info = await run("inspect", () => api.inspectApk(path));
    if (info) setApk(info);
  }, [run]);

  // "Install APK" on the overview lands here and opens the picker right away.
  useEffect(() => {
    if (p.pickApk && running) {
      p.setPickApk(false);
      void pick();
    }
  }, [p, running, pick]);

  const shown = useMemo(() => {
    const f = filter.trim().toLowerCase();
    const list = apps ?? [];
    return (f ? list.filter((a) => a.name.toLowerCase().includes(f) || a.package.toLowerCase().includes(f)) : list).sort((a, b) =>
      a.name.localeCompare(b.name),
    );
  }, [apps, filter]);

  const install = async (info: ApkInfo) => {
    const out = await run("install", () => api.installApk(info.path, info.sha256), (o) => t(o.was_update ? "apps.updated" : "apps.installed", { pkg: o.package }));
    if (out) {
      setApk(null);
      await load();
    }
  };

  if (!running) {
    return (
      <div className="page">
        <PageHead title={t("apps.title")} />
        <section className="card hero idle">
          <p className="body-l" style={{ marginTop: 0 }}>
            {ready ? t("apps.notRunning") : t("apps.notReady")}
          </p>
          <div className="row">
            {ready ? (
              <Button kind="filled" icon={mdiPlay} busy={busy === "start"} onClick={() => run("start", api.start, () => t("toast.started")).then(() => p.refreshStatus())}>
                {t("btn.start")}
              </Button>
            ) : (
              <Button kind="filled" icon={mdiRocketLaunchOutline} onClick={() => p.go("setup")}>
                {t("btn.openSetup")}
              </Button>
            )}
          </div>
          {busy === "start" && <div className="progress" />}
        </section>
      </div>
    );
  }

  return (
    <div className="page">
      <PageHead
        title={t("apps.title")}
        actions={
          <Button kind="filled" icon={mdiPackageVariantPlus} busy={busy === "inspect"} disabled={busy === "install"} onClick={() => void pick()}>
            {t("btn.installApk")}
          </Button>
        }
      />

      {apk && (
        <Card title={apk.label ?? apk.package} icon={mdiPackageVariantPlus}>
          <KeyValue
            rows={[
              [t("apk.package"), <code>{apk.package}</code>],
              [t("apk.version"), `${apk.version_name ?? "?"} (${apk.version_code ?? "?"})`],
              [t("apk.sdk"), `min ${apk.min_sdk ?? "?"} · target ${apk.target_sdk ?? "?"}`],
              [t("apk.size"), formatBytes(apk.file_size)],
              [t("apk.abi"), apk.native_abis.length ? apk.native_abis.join(", ") : t("common.none")],
              ["SHA-256", <code style={{ fontSize: 11.5 }}>{apk.sha256}</code>],
            ]}
          />
          <div className="row" style={{ marginTop: 14 }}>
            <Chip tone={ABI_TONE[apk.abi_compat]}>{t(`apk.compat.${apk.abi_compat}` as Key)}</Chip>
            {apk.min_sdk != null && apk.min_sdk > 33 && <Chip tone="warn">{t("apk.sdkWarn", { sdk: apk.min_sdk })}</Chip>}
          </div>
          <details>
            <summary>{t("apk.permissions", { n: apk.permissions.length })}</summary>
            <ul className="small mono">{apk.permissions.map((x) => <li key={x}>{x}</li>)}</ul>
          </details>
          <div className="row" style={{ marginTop: 16 }}>
            <Button kind="filled" busy={busy === "install"} disabled={apk.abi_compat === "incompatible"} onClick={() => void install(apk)}>
              {t("btn.install")}
            </Button>
            <Button kind="text" disabled={busy === "install"} onClick={() => setApk(null)}>
              {t("common.cancel")}
            </Button>
          </div>
          {busy === "install" && <div className="progress" />}
        </Card>
      )}

      <WindowModeCard {...p} />

      {!apk && (
        <section className="card outlined">
          <div className="row">
            <Icon path={mdiFolderOpenOutline} />
            <span className="muted">{t("apps.install.hint")}</span>
          </div>
        </section>
      )}

      <Card
        title={apps ? `${t("apps.list")} · ${t("apps.count", { n: apps.length })}` : t("apps.list")}
        actions={
          <>
            <label className="search">
              <Icon path={mdiMagnify} />
              <input placeholder={t("apps.search")} value={filter} onChange={(e) => setFilter(e.target.value)} />
            </label>
            <IconButton icon={mdiRefresh} label={t("common.refresh")} onClick={() => void load()} />
          </>
        }
      >
        {listError && <p className="item-remedy">{listError}</p>}
        {!apps && !listError && <Loading text={t("apps.reading")} />}
        {apps && shown.length === 0 && <div className="empty">{t("apps.empty")}</div>}
        {apps && shown.length > 0 && (
          <ul className="list">
            {shown.map((a) => (
              <li key={a.package} className="list-item center">
                <AppAvatar app={a} />
                <div className="item-main">
                  <div className="item-title">{a.name}</div>
                  <div className="item-sub small mono">{a.package}</div>
                </div>
                <div className="item-trailing">
                  <Button kind="tonal" icon={mdiOpenInNew} busy={busy === `launch:${a.package}`} onClick={() => run(`launch:${a.package}`, () => api.launch(a.package))}>
                    {t("btn.launch")}
                  </Button>
                  <IconButton
                    icon={mdiGestureTap}
                    label={t("apps.touch")}
                    selected={touch.includes(a.package)}
                    busy={busy === `touch:${a.package}`}
                    onClick={() => {
                      const on = !touch.includes(a.package);
                      void run(`touch:${a.package}`, () => api.setFakeTouch(a.package, on), () => t(on ? "apps.touch.on" : "apps.touch.off", { name: a.name })).then(
                        (list) => list && setTouch(list),
                      );
                    }}
                  />
                  <IconButton icon={mdiDeleteOutline} label={t("btn.uninstall")} danger onClick={() => setRemoving(a)} busy={busy === `remove:${a.package}`} />
                </div>
              </li>
            ))}
          </ul>
        )}
      </Card>

      {removing && (
        <Dialog
          title={t("dlg.uninstall.title", { name: removing.name })}
          confirmLabel={t("btn.uninstall")}
          danger
          onCancel={() => setRemoving(null)}
          onConfirm={() => {
            const target = removing;
            setRemoving(null);
            void run(`remove:${target.package}`, () => api.uninstall(target.package), (how) => t("apps.removed", { pkg: target.package, how })).then(load);
          }}
        >
          <p>{t("dlg.uninstall.body", { pkg: removing.package })}</p>
        </Dialog>
      )}
    </div>
  );
}
