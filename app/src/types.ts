// Mirrors of the serde types in droidnative-core (snake_case enums, Option → null).

export type CheckStatus = "pass" | "info" | "unknown" | "warn" | "missing" | "fail";

export interface Check {
  id: string;
  label: string;
  status: CheckStatus;
  detail: string;
  remedy: string | null;
}

export interface CheckGroup {
  title: string;
  checks: Check[];
}

export type GpuBackendKind = "mesa_native" | "nvidia_accelerated" | "nvidia_experimental" | "software_fallback";
export type Readiness = "ready" | "needs_setup" | "unsupported";

export interface BackendDescriptor {
  kind: GpuBackendKind;
  name: string;
  summary: string;
  stability: "stable" | "experimental" | "fallback";
  requirements: string[];
  compatible_drivers: string[];
  upstream: string | null;
  expected_renderer: string;
}

export interface BackendAssessment {
  descriptor: BackendDescriptor;
  readiness: Readiness;
  target_device: string | null;
  target_render_node: string | null;
  checks: Check[];
}

export interface GpuPlan {
  assessments: BackendAssessment[];
  recommended: GpuBackendKind;
  reason: string;
}

export interface GpuDevice {
  pci_slot: string | null;
  vendor: "nvidia" | "amd" | "intel" | "virt_io" | "other";
  name: string | null;
  kernel_driver: string | null;
  render_node: string | null;
  connected_outputs: string[];
  vram_bytes: number | null;
}

export interface ServiceState {
  active: string | null;
  enabled: string | null;
}

export interface WaydroidInstall {
  installed: boolean;
  binary: string | null;
  version: string | null;
  config_present: boolean;
  initialized: boolean;
  config: { drm_device: string | null; vendor_type: string | null; properties: [string, string][] } | null;
  images: { path: string; size_bytes: number }[];
  container_service: ServiceState;
  nvidia_setup: string | null;
  nvidia_mode: NvidiaMode | null;
  nvidia_stack: { tag: string; mode: NvidiaMode; installed_unix: number } | null;
  distro_package: string | null;
}

export type NvidiaMode = "venus" | "prolapse";

export interface HostReport {
  droidnative_version: string;
  hostname: string | null;
  os: { pretty_name: string; base_release: string | null };
  kernel: { release: string };
  cpu: { model: string | null; logical_cpus: number; arch: string; android_abi: string | null };
  memory: { total_bytes: number; available_bytes: number };
  session: { kind: "wayland" | "x11" | "tty" | "unknown"; desktop: string | null; compositor: string | null };
  gpu: {
    devices: GpuDevice[];
    display_device: string | null;
    nvidia: { version: string | null; module_flavor: string; modeset: boolean | null } | null;
    mesa: { version: string } | null;
  };
  waydroid: WaydroidInstall;
}

export interface CompatibilityReport {
  verdict: "compatible" | "compatible_after_setup" | "incompatible";
  groups: CheckGroup[];
  gpu_plan: GpuPlan;
  host: HostReport;
}

export type StepState = "done" | "pending" | "blocked" | "optional";

export interface SetupStep {
  id: string;
  title: string;
  detail: string;
  state: StepState;
  op: unknown | null;
  action: string | null;
}

export interface SetupView {
  steps: SetupStep[];
  helper: string;
}

export type SessionState = "stopped" | "running" | "frozen" | "unknown";

export interface RuntimeStatus {
  backend: string;
  session: SessionState;
  container: string | null;
  vendor_type: string | null;
  ip_address: string | null;
  session_user: string | null;
  wayland_display: string | null;
}

export interface AndroidApp {
  name: string;
  package: string;
  categories: string[];
}

export type AbiCompat = "universal" | "native" | "needs_translation" | "incompatible";

export interface ApkInfo {
  path: string;
  file_size: number;
  sha256: string;
  package: string;
  version_name: string | null;
  version_code: number | null;
  min_sdk: number | null;
  target_sdk: number | null;
  label: string | null;
  permissions: string[];
  native_abis: string[];
  abi_compat: AbiCompat;
}

export interface InstallOutcome {
  package: string;
  verified: boolean;
  was_update: boolean;
  output: string;
}

export interface DisplayHealth {
  status: "ok" | "failed" | "unknown";
  detail: string;
  evidence: string[];
}

export interface AccelVerdict {
  status: "hardware_verified" | "software" | "unknown";
  vendor: string | null;
  path: string | null;
  gles_line: string | null;
  detail: string;
  display: DisplayHealth;
}

export interface GraphicsConfig {
  egl: string | null;
  gralloc: string | null;
  vulkan: string | null;
  gbm_device: string | null;
}

export type GpuSelection = { mode: "render_node"; node: string } | { mode: "software" } | { mode: "auto" };
