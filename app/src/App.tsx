import { useCallback, useEffect, useState } from "react";
import {
  mdiApps,
  mdiClipboardCheckOutline,
  mdiDownloadCircleOutline,
  mdiExpansionCardVariant,
  mdiStethoscope,
  mdiViewDashboardOutline,
} from "@mdi/js";
import { api, errorText } from "./api";
import { t, type Key } from "./i18n";
import type { AccelVerdict, CompatibilityReport, RuntimeStatus } from "./types";
import { Icon, Loading, NoticeArea, type Notice } from "./ui";
import { Overview } from "./pages/Overview";
import { Setup } from "./pages/Setup";
import { Gpu } from "./pages/Gpu";
import { Apps } from "./pages/Apps";
import { Compatibility } from "./pages/Compatibility";
import { Diagnostics } from "./pages/Diagnostics";

export type Page = "overview" | "apps" | "gpu" | "setup" | "compat" | "diag";

const PAGES: [Page, Key, string][] = [
  ["overview", "nav.overview", mdiViewDashboardOutline],
  ["apps", "nav.apps", mdiApps],
  ["gpu", "nav.gpu", mdiExpansionCardVariant],
  ["setup", "nav.setup", mdiDownloadCircleOutline],
  ["compat", "nav.compat", mdiClipboardCheckOutline],
  ["diag", "nav.diag", mdiStethoscope],
];

export interface Shared {
  report: CompatibilityReport | null;
  status: RuntimeStatus | null;
  statusError: string | null;
  verdict: AccelVerdict | null;
  setVerdict: (v: AccelVerdict | null) => void;
  refreshReport: () => Promise<void>;
  refreshStatus: () => Promise<void>;
  setNotice: (n: Notice) => void;
  go: (p: Page) => void;
  /** Set by "Install APK" on the overview: the apps page opens the file picker once. */
  pickApk: boolean;
  setPickApk: (v: boolean) => void;
}

export default function App() {
  const [page, setPage] = useState<Page>("overview");
  const [report, setReport] = useState<CompatibilityReport | null>(null);
  const [reportError, setReportError] = useState<string | null>(null);
  const [status, setStatus] = useState<RuntimeStatus | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);
  const [verdict, setVerdict] = useState<AccelVerdict | null>(null);
  const [notice, setNotice] = useState<Notice>(null);
  const [pickApk, setPickApk] = useState(false);

  const refreshReport = useCallback(async () => {
    try {
      setReport(await api.report());
      setReportError(null);
    } catch (e) {
      setReportError(errorText(e));
    }
  }, []);

  const refreshStatus = useCallback(async () => {
    try {
      setStatus(await api.status());
      setStatusError(null);
    } catch (e) {
      setStatus(null);
      setStatusError(errorText(e));
    }
  }, []);

  useEffect(() => {
    void refreshReport();
    void refreshStatus();
    const id = window.setInterval(() => void refreshStatus(), 5000);
    return () => window.clearInterval(id);
  }, [refreshReport, refreshStatus]);

  // A stopped session invalidates any earlier acceleration verdict.
  useEffect(() => {
    if (status && status.session === "stopped") setVerdict(null);
  }, [status]);

  const closeNotice = useCallback(() => setNotice(null), []);
  const shared: Shared = { report, status, statusError, verdict, setVerdict, refreshReport, refreshStatus, setNotice, go: setPage, pickApk, setPickApk };
  const session = status?.session ?? "unknown";
  const sessionText = statusError ? t("session.unavailable") : t(`session.${session}` as Key);

  return (
    <div className="shell">
      <nav className="drawer" aria-label="DroidNative">
        <div className="brand">
          <img src="/icon.png" alt="" width={40} height={40} />
          <div>
            <div className="brand-name">DroidNative</div>
            <div className="brand-sub">{t("app.tagline")}</div>
          </div>
        </div>
        {PAGES.map(([id, label, icon]) => (
          <button key={id} className={`nav-item ${page === id ? "active" : ""}`} onClick={() => setPage(id)} aria-current={page === id ? "page" : undefined}>
            <Icon path={icon} />
            {t(label)}
          </button>
        ))}
        <div className="drawer-foot">
          <span className={`dot ${statusError ? "" : session}`} />
          {t("android.label")}: {sessionText}
          {report && <span style={{ marginLeft: "auto" }}>v{report.host.droidnative_version}</span>}
        </div>
      </nav>
      <main className="content">
        <div className="page" style={{ marginBottom: notice?.kind === "error" ? 16 : 0 }}>
          {notice?.kind === "error" && <NoticeArea notice={notice} onClose={closeNotice} />}
          {reportError && <NoticeArea notice={{ kind: "error", text: t("error.analysis", { msg: reportError }) }} onClose={() => setReportError(null)} />}
        </div>
        {!report && !reportError && <Loading text={t("loading.analysis")} />}
        {report && page === "overview" && <Overview {...shared} />}
        {report && page === "apps" && <Apps {...shared} />}
        {report && page === "gpu" && <Gpu {...shared} />}
        {report && page === "setup" && <Setup {...shared} />}
        {report && page === "compat" && <Compatibility {...shared} />}
        {report && page === "diag" && <Diagnostics {...shared} />}
      </main>
      {notice && notice.kind !== "error" && <NoticeArea notice={notice} onClose={closeNotice} />}
    </div>
  );
}
