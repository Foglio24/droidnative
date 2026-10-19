import { useCallback, useEffect, useState } from "react";
import { api, errorText } from "./api";
import type { AccelVerdict, CompatibilityReport, RuntimeStatus } from "./types";
import { NoticeBar, type Notice } from "./ui";
import { Overview } from "./pages/Overview";
import { Setup } from "./pages/Setup";
import { Gpu } from "./pages/Gpu";
import { Apps } from "./pages/Apps";
import { Compatibility } from "./pages/Compatibility";
import { Diagnostics } from "./pages/Diagnostics";

export type Page = "overview" | "setup" | "gpu" | "apps" | "compat" | "diag";

const PAGES: [Page, string][] = [
  ["overview", "Panoramica"],
  ["setup", "Installazione"],
  ["gpu", "GPU"],
  ["apps", "App Android"],
  ["compat", "Compatibilità"],
  ["diag", "Diagnostica"],
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
}

export default function App() {
  const [page, setPage] = useState<Page>("overview");
  const [report, setReport] = useState<CompatibilityReport | null>(null);
  const [reportError, setReportError] = useState<string | null>(null);
  const [status, setStatus] = useState<RuntimeStatus | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);
  const [verdict, setVerdict] = useState<AccelVerdict | null>(null);
  const [notice, setNotice] = useState<Notice>(null);

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
    const t = window.setInterval(() => void refreshStatus(), 5000);
    return () => window.clearInterval(t);
  }, [refreshReport, refreshStatus]);

  // A stopped session invalidates any earlier acceleration verdict.
  useEffect(() => {
    if (status && status.session === "stopped") setVerdict(null);
  }, [status]);

  const shared: Shared = { report, status, statusError, verdict, setVerdict, refreshReport, refreshStatus, setNotice, go: setPage };
  const session = status?.session ?? "unknown";

  return (
    <div className="shell">
      <nav className="sidebar">
        <div className="brand">
          <img src="/icon.png" alt="" width={34} height={34} />
          <div>
            <div className="brand-name">DroidNative</div>
            <div className="brand-sub">{report ? `v${report.host.droidnative_version}` : "…"}</div>
          </div>
        </div>
        {PAGES.map(([id, label]) => (
          <button key={id} className={`nav ${page === id ? "nav-active" : ""}`} onClick={() => setPage(id)}>
            {label}
          </button>
        ))}
        <div className="sidebar-foot">
          <span className={`dot dot-${session}`} />
          Android: {statusError ? "non disponibile" : { running: "attivo", frozen: "sospeso", stopped: "fermo", unknown: "…" }[session]}
        </div>
      </nav>
      <main className="content">
        <NoticeBar notice={notice} onClose={() => setNotice(null)} />
        {reportError && <div className="notice notice-error"><pre>Analisi del sistema fallita: {reportError}</pre></div>}
        {!report && !reportError && <div className="loading">Analisi del sistema in corso…</div>}
        {report && page === "overview" && <Overview {...shared} />}
        {report && page === "setup" && <Setup {...shared} />}
        {report && page === "gpu" && <Gpu {...shared} />}
        {report && page === "apps" && <Apps {...shared} />}
        {report && page === "compat" && <Compatibility {...shared} />}
        {report && page === "diag" && <Diagnostics {...shared} />}
      </main>
    </div>
  );
}
