import { invoke } from "@tauri-apps/api/core";
import type {
  AccelVerdict,
  AndroidApp,
  ApkInfo,
  CompatibilityReport,
  GpuSelection,
  GraphicsConfig,
  InstallOutcome,
  NvidiaMode,
  RuntimeStatus,
  SetupView,
  WindowMode,
} from "./types";

export const api = {
  report: () => invoke<CompatibilityReport>("get_report"),
  setup: () => invoke<SetupView>("get_setup"),
  runSetupStep: (id: string) => invoke<string>("run_setup_step", { id }),
  selectGpu: (selection: GpuSelection) => invoke<string>("select_gpu", { selection }),
  nvidiaInstall: (mode: NvidiaMode) => invoke<string>("nvidia_install", { mode }),
  nvidiaSetMode: (mode: NvidiaMode) => invoke<string>("nvidia_set_mode", { mode }),
  status: () => invoke<RuntimeStatus>("runtime_status"),
  start: () => invoke<RuntimeStatus>("session_start"),
  stop: () => invoke<void>("session_stop"),
  restart: () => invoke<RuntimeStatus>("session_restart"),
  restartContainer: () => invoke<string>("container_restart"),
  showUi: () => invoke<void>("show_full_ui"),
  apps: () => invoke<AndroidApp[]>("list_apps"),
  inspectApk: (path: string) => invoke<ApkInfo>("inspect_apk_file", { path }),
  installApk: (path: string, expectedSha256: string) => invoke<InstallOutcome>("install_apk", { path, expectedSha256 }),
  launch: (pkg: string) => invoke<void>("launch_app", { package: pkg }),
  uninstall: (pkg: string) => invoke<string>("uninstall_app", { package: pkg }),
  fakeTouch: () => invoke<string[]>("fake_touch_list"),
  setFakeTouch: (pkg: string, enabled: boolean) => invoke<string[]>("set_fake_touch", { package: pkg, enabled }),
  windowMode: () => invoke<WindowMode>("window_mode"),
  setWindowMode: (mode: WindowMode) => invoke<void>("set_window_mode", { mode }),
  graphics: () => invoke<GraphicsConfig>("graphics_config"),
  verifyGpu: () => invoke<AccelVerdict>("verify_gpu"),
  googleDeviceId: () => invoke<string>("google_device_id"),
  logs: (lines: number) => invoke<string>("read_logs", { lines }),
};

export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return JSON.stringify(e);
}
