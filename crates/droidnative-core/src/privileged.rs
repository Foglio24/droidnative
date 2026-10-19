//! Root operations DroidNative may perform, as a closed set.
//!
//! The unprivileged UI never builds shell strings for root: it asks `pkexec` to run
//! `droidnative priv <op> [validated args]`, and the helper maps each op to fixed argv.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::exec::{CmdOutput, CommandRunner};
use crate::fs::HostFs;
use crate::ini::Ini;
use crate::runtime::waydroid::{CONTAINER_UNIT, WAYDROID_CFG};
use crate::tr;

#[derive(Debug)]
pub enum PrivError {
    AuthDenied,
    NoPkexec,
    Invalid(String),
    Missing(&'static str),
    StepFailed { step: String, output: String },
    Io(std::io::Error),
}

impl std::fmt::Display for PrivError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match self {
            Self::AuthDenied => tr!("autorizzazione negata o annullata dall'utente", "authorization denied or cancelled by the user"),
            Self::NoPkexec => tr!("pkexec non disponibile: installa il pacchetto polkit (pkexec)", "pkexec not available: install the polkit (pkexec) package"),
            Self::Invalid(m) => tr!("operazione non valida: {m}", "invalid operation: {m}"),
            Self::Missing(what) => tr!("{what} non è installato", "{what} is not installed"),
            Self::StepFailed { step, output } => tr!("passo `{step}` fallito: {output}", "step `{step}` failed: {output}"),
            Self::Io(e) => e.to_string(),
        };
        f.write_str(&text)
    }
}

impl std::error::Error for PrivError {}

impl From<std::io::Error> for PrivError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemType {
    Vanilla,
    Gapps,
}

impl SystemType {
    pub fn as_arg(self) -> &'static str {
        match self {
            Self::Vanilla => "VANILLA",
            Self::Gapps => "GAPPS",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", content = "node", rename_all = "snake_case")]
pub enum GpuSelection {
    /// Force a Mesa render node (`/dev/dri/renderD*`).
    RenderNode(String),
    /// Force SwiftShader.
    Software,
    /// Let Waydroid choose (its default: first non-NVIDIA render node).
    Auto,
}

/// Suites published at https://repo.waydro.id/dists/ (and accepted by its setup script).
pub const WAYDROID_REPO_SUITES: &[&str] = &[
    "focal", "jammy", "lunar", "mantic", "noble", "oracular", "plucky", "questing", "resolute", "buster", "bullseye", "bookworm",
    "trixie", "forky", "sid",
];
pub const WAYDROID_REPO_URL: &str = "https://repo.waydro.id/";
pub const WAYDROID_KEY_URL: &str = "https://repo.waydro.id/waydroid.gpg";
/// Primary key fingerprint; matches `SignWith: 7CE0331F71E0A238` in the repository's Release files.
pub const WAYDROID_KEY_FPR: &str = "7CE0331F71E0A238BB1002D70E406D181DCEE19C";
pub const WAYDROID_KEYRING: &str = "/usr/share/keyrings/waydroid.gpg";
pub const WAYDROID_SOURCES: &str = "/etc/apt/sources.list.d/waydroid.list";
pub const DIAGNOSTIC_PACKAGES: &[&str] = &["mesa-utils", "vulkan-tools", "wayland-utils"];
/// Minimal UFW rules for Waydroid's bridge: DHCP and DNS from Android to the host, and
/// forwarding of connections Android opens. Nothing is opened towards Android from outside.
pub const UFW_CONTAINER_RULES: &[&[&str]] = &[
    &["allow", "in", "on", "waydroid0", "to", "any", "port", "67", "proto", "udp"],
    &["allow", "in", "on", "waydroid0", "to", "any", "port", "53"],
    &["route", "allow", "in", "on", "waydroid0"],
];

pub fn validate_suite(suite: &str) -> Result<(), PrivError> {
    if WAYDROID_REPO_SUITES.contains(&suite) {
        Ok(())
    } else {
        Err(PrivError::Invalid(tr!("release {suite:?} non pubblicata su repo.waydro.id", "release {suite:?} is not published on repo.waydro.id")))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PrivilegedOp {
    /// Add repo.waydro.id (key fingerprint pinned) and install the `waydroid` package.
    InstallWaydroid { suite: String },
    /// mesa-utils, vulkan-tools, wayland-utils from the distribution archive.
    InstallDiagnostics,
    /// Add [`UFW_CONTAINER_RULES`] so Android gets DHCP, DNS and outbound connectivity.
    AllowContainerNetwork,
    LoadBinder,
    EnableContainerService,
    RestartContainerService,
    StopContainerService,
    InitWaydroid { system_type: SystemType },
    SelectGpu { selection: GpuSelection },
    GraphicsDiagnostics,
    /// `pm uninstall` inside the container: fallback for Waydroid's IPlatform.removeApp, which
    /// throws IllegalArgumentException (-3) on the Android 13 images.
    UninstallPackage { package: String },
    /// Replace the distro waydroid with a pinned waydroid-nvidia release and activate `mode`.
    InstallNvidiaStack { tag: String, mode: crate::nvidia_stack::NvidiaMode },
    /// Switch an installed waydroid-nvidia between venus and nvidia-prolapse.
    NvidiaSetMode { mode: crate::nvidia_stack::NvidiaMode },
}

impl PrivilegedOp {
    /// Arguments after `droidnative priv`.
    pub fn to_args(&self) -> Vec<String> {
        let v = |s: &[&str]| s.iter().map(|x| x.to_string()).collect();
        match self {
            Self::InstallWaydroid { suite } => v(&["install-waydroid", "--suite", suite]),
            Self::InstallDiagnostics => v(&["install-diagnostics"]),
            Self::AllowContainerNetwork => v(&["allow-container-network"]),
            Self::LoadBinder => v(&["load-binder"]),
            Self::EnableContainerService => v(&["enable-container"]),
            Self::RestartContainerService => v(&["restart-container"]),
            Self::StopContainerService => v(&["stop-container"]),
            Self::InitWaydroid { system_type } => v(&["init", "--system-type", system_type.as_arg()]),
            Self::SelectGpu { selection: GpuSelection::RenderNode(n) } => v(&["select-gpu", "--render-node", n]),
            Self::SelectGpu { selection: GpuSelection::Software } => v(&["select-gpu", "--software"]),
            Self::SelectGpu { selection: GpuSelection::Auto } => v(&["select-gpu", "--auto"]),
            Self::GraphicsDiagnostics => v(&["graphics-diagnostics"]),
            Self::UninstallPackage { package } => v(&["uninstall-package", package]),
            Self::InstallNvidiaStack { tag, mode } => v(&["install-nvidia-stack", "--tag", tag, "--mode", mode.as_arg()]),
            Self::NvidiaSetMode { mode } => v(&["nvidia-mode", "--mode", mode.as_arg()]),
        }
    }

    pub fn description(&self) -> String {
        match self {
            Self::InstallWaydroid { suite } => {
                let key = &WAYDROID_KEY_FPR[24..];
                tr!("Aggiungere repo.waydro.id ({suite}, chiave {key}) e installare waydroid", "Add repo.waydro.id ({suite}, key {key}) and install waydroid")
            }
            Self::InstallDiagnostics => {
                let pkgs = DIAGNOSTIC_PACKAGES.join(", ");
                tr!("Installare gli strumenti diagnostici ({pkgs})", "Install the diagnostic tools ({pkgs})")
            }
            Self::AllowContainerNetwork => {
                let rules = UFW_CONTAINER_RULES.iter().map(|r| format!("ufw {}", r.join(" "))).collect::<Vec<_>>().join("\n");
                tr!("Aggiungere al firewall UFW:\n{rules}", "Add to the UFW firewall:\n{rules}")
            }
            Self::LoadBinder => tr!("Caricare il modulo kernel binder_linux", "Load the binder_linux kernel module"),
            Self::EnableContainerService => tr!("Abilitare e avviare {CONTAINER_UNIT}", "Enable and start {CONTAINER_UNIT}"),
            Self::RestartContainerService => tr!("Riavviare {CONTAINER_UNIT}", "Restart {CONTAINER_UNIT}"),
            Self::StopContainerService => tr!("Arrestare {CONTAINER_UNIT}", "Stop {CONTAINER_UNIT}"),
            Self::InitWaydroid { system_type } => {
                let t = system_type.as_arg();
                tr!("Scaricare e verificare le immagini Android ({t})", "Download and verify the Android images ({t})")
            }
            Self::SelectGpu { selection: GpuSelection::RenderNode(n) } => tr!("Usare la GPU {n} per Android", "Use GPU {n} for Android"),
            Self::SelectGpu { selection: GpuSelection::Software } => tr!("Forzare il rendering software (SwiftShader)", "Force software rendering (SwiftShader)"),
            Self::SelectGpu { selection: GpuSelection::Auto } => tr!("Ripristinare la scelta GPU automatica di Waydroid", "Restore Waydroid's automatic GPU choice"),
            Self::GraphicsDiagnostics => tr!(
                "Leggere il renderer usato da SurfaceFlinger e gli errori di visualizzazione",
                "Read the renderer SurfaceFlinger uses and the display errors"
            ),
            Self::UninstallPackage { package } => tr!("Disinstallare {package} con il gestore pacchetti di Android", "Uninstall {package} with Android's package manager"),
            Self::InstallNvidiaStack { tag, mode } => {
                let mode = mode.as_arg();
                tr!(
                    "Installare waydroid-nvidia {tag} (modalità {mode}):\n- scaricare il rilascio e verificarne gli hash fissati in DroidNative (~260 MB)\n- rimuovere (purge) il pacchetto waydroid della distribuzione; immagini e dati Android restano\n- installare la Waydroid patchata e lo stack grafico NVIDIA con l'installer ufficiale\n- attivare la modalità con waydroid-nvidia-setup",
                    "Install waydroid-nvidia {tag} ({mode} mode):\n- download the release and check it against the hashes pinned in DroidNative (~260 MB)\n- purge the distribution's waydroid package; Android images and data are kept\n- install the patched Waydroid and the NVIDIA graphics stack with the official installer\n- activate the mode with waydroid-nvidia-setup"
                )
            }
            Self::NvidiaSetMode { mode } => {
                let mode = mode.as_arg();
                tr!("Passare waydroid-nvidia alla modalità {mode}", "Switch waydroid-nvidia to {mode} mode")
            }
        }
    }

    fn timeout(&self) -> Duration {
        match self {
            // Image download: ~1 GB over a slow link; apt installs pull LXC and Python deps.
            Self::InitWaydroid { .. } | Self::InstallWaydroid { .. } | Self::InstallDiagnostics => Duration::from_secs(3600),
            Self::InstallNvidiaStack { .. } => Duration::from_secs(7200),
            Self::NvidiaSetMode { .. } => Duration::from_secs(1800),
            Self::SelectGpu { .. } => Duration::from_secs(300),
            _ => Duration::from_secs(120),
        }
    }
}

pub trait Escalator: Send + Sync {
    fn run(&self, op: &PrivilegedOp) -> Result<CmdOutput, PrivError>;
}

/// Runs `pkexec <helper> priv <op>`; polkit shows the authentication dialog.
pub struct PkexecEscalator {
    pub helper: PathBuf,
    pub cmd: Arc<dyn CommandRunner>,
}

impl Escalator for PkexecEscalator {
    fn run(&self, op: &PrivilegedOp) -> Result<CmdOutput, PrivError> {
        let pkexec = self.cmd.which("pkexec").ok_or(PrivError::NoPkexec)?;
        if !self.helper.is_absolute() {
            return Err(PrivError::Invalid(tr!("il helper deve avere un percorso assoluto", "the helper needs an absolute path")));
        }
        let helper = self.helper.to_string_lossy().into_owned();
        let args = op.to_args();
        let mut argv: Vec<&str> = vec![&helper, "priv", "--lang", crate::i18n::lang().code()];
        argv.extend(args.iter().map(String::as_str));
        let out = self.cmd.run_with_timeout(&pkexec.to_string_lossy(), &argv, op.timeout())?;
        // pkexec: 126 = dialog dismissed / not authorized, 127 = authentication failed.
        if matches!(out.status, Some(126) | Some(127)) && !out.stderr.contains("droidnative:") {
            return Err(PrivError::AuthDenied);
        }
        Ok(out)
    }
}

/// Non-interactive alternative for terminals and SSH sessions without a polkit agent:
/// `sudo -n <helper> priv <op>`. Same helper, same closed set of operations.
pub struct SudoEscalator {
    pub helper: PathBuf,
    pub cmd: Arc<dyn CommandRunner>,
}

impl Escalator for SudoEscalator {
    fn run(&self, op: &PrivilegedOp) -> Result<CmdOutput, PrivError> {
        let sudo = self.cmd.which("sudo").ok_or(PrivError::Missing("sudo"))?;
        if !self.helper.is_absolute() {
            return Err(PrivError::Invalid(tr!("il helper deve avere un percorso assoluto", "the helper needs an absolute path")));
        }
        let helper = self.helper.to_string_lossy().into_owned();
        let args = op.to_args();
        let mut argv: Vec<&str> = vec!["-n", &helper, "priv", "--lang", crate::i18n::lang().code()];
        argv.extend(args.iter().map(String::as_str));
        let out = self.cmd.run_with_timeout(&sudo.to_string_lossy(), &argv, op.timeout())?;
        if !out.success() && out.stderr.contains("a password is required") {
            return Err(PrivError::AuthDenied);
        }
        Ok(out)
    }
}

/// `DROIDNATIVE_ESCALATOR=sudo` selects sudo; anything else (the default) uses pkexec/polkit.
pub fn escalator_from_env(helper: PathBuf, cmd: Arc<dyn CommandRunner>) -> Box<dyn Escalator> {
    match std::env::var("DROIDNATIVE_ESCALATOR").as_deref() {
        Ok("sudo") => Box::new(SudoEscalator { helper, cmd }),
        _ => Box::new(PkexecEscalator { helper, cmd }),
    }
}

/// Validate a render node the way the root helper must: exact `/dev/dri/renderD<N>` form,
/// existing, and not NVIDIA (Waydroid's Mesa path ignores NVIDIA nodes).
pub fn validate_render_node(fs: &HostFs, node: &str) -> Result<(), PrivError> {
    let n = node.strip_prefix("/dev/dri/renderD").ok_or_else(|| PrivError::Invalid(format!("nodo non valido: {node:?}")))?;
    if n.is_empty() || !n.chars().all(|c| c.is_ascii_digit()) {
        return Err(PrivError::Invalid(tr!("nodo non valido: {node:?}", "invalid node: {node:?}")));
    }
    if !fs.exists(node) {
        return Err(PrivError::Invalid(tr!("{node} non esiste", "{node} does not exist")));
    }
    let name = node.rsplit('/').next().unwrap_or_default();
    let driver = fs
        .read(format!("/sys/class/drm/{name}/device/uevent"))
        .and_then(|u| crate::fs::lookup(&crate::fs::parse_key_values(&u), "DRIVER").map(str::to_string));
    if driver.as_deref() == Some("nvidia") {
        return Err(PrivError::Invalid(tr!(
            "{node} è una GPU NVIDIA: Mesa non può usarla, scegli il backend NVIDIA Accelerated",
            "{node} is an NVIDIA GPU: Mesa cannot use it, pick the NVIDIA Accelerated backend"
        )));
    }
    Ok(())
}

const SOFTWARE_PROPS: &[(&str, &str)] = &[("ro.hardware.gralloc", "default"), ("ro.hardware.egl", "swiftshader")];

/// Pure config transformation for a GPU selection (unit-tested; no I/O).
pub fn apply_gpu_selection(cfg_text: &str, selection: &GpuSelection) -> String {
    let mut ini = Ini::parse(cfg_text);
    let drop_software_overrides = |ini: &mut Ini| {
        for (k, v) in SOFTWARE_PROPS {
            if ini.get("properties", k) == Some(*v) {
                ini.remove("properties", k);
            }
        }
    };
    match selection {
        GpuSelection::RenderNode(node) => {
            ini.set("waydroid", "drm_device", node);
            drop_software_overrides(&mut ini);
        }
        GpuSelection::Software => {
            ini.remove("waydroid", "drm_device");
            for (k, v) in SOFTWARE_PROPS {
                ini.set("properties", k, v);
            }
        }
        GpuSelection::Auto => {
            ini.remove("waydroid", "drm_device");
            drop_software_overrides(&mut ini);
        }
    }
    ini.render()
}

/// Write `content` to `path` atomically, keeping a one-time backup of the original.
fn replace_file(path: &Path, content: &str) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let backup = path.with_extension("cfg.droidnative.bak");
    if path.exists() && !backup.exists() {
        std::fs::copy(path, &backup)?;
    }
    let tmp = path.with_extension("cfg.droidnative.tmp");
    std::fs::write(&tmp, content)?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644))?;
    std::fs::rename(&tmp, path)
}

/// Executed as root by `droidnative priv`. Every external program is called with fixed argv.
pub fn execute(op: &PrivilegedOp, fs: &HostFs, cmd: &dyn CommandRunner) -> Result<CmdOutput, PrivError> {
    let step = |program: &str, args: &[&str], timeout: Duration| -> Result<CmdOutput, PrivError> {
        let out = cmd.run_with_timeout(program, args, timeout)?;
        if !out.success() {
            return Err(PrivError::StepFailed {
                step: format!("{program} {}", args.join(" ")),
                output: if out.timed_out { "timeout".into() } else { out.combined() },
            });
        }
        Ok(out)
    };
    let waydroid = || cmd.which("waydroid").map(|p| p.to_string_lossy().into_owned()).ok_or(PrivError::Missing("Waydroid"));

    match op {
        PrivilegedOp::InstallWaydroid { suite } => {
            validate_suite(suite)?;
            let keyring = fs.path(WAYDROID_KEYRING);
            let tmp = keyring.with_extension("gpg.droidnative.tmp");
            let tmp_s = tmp.to_string_lossy().into_owned();
            step("curl", &["-fsS", "--proto", "=https", "--tlsv1.2", "-m", "60", WAYDROID_KEY_URL, "-o", &tmp_s], Duration::from_secs(90))?;
            let fpr = step("gpg", &["--show-keys", "--with-colons", "--with-fingerprint", &tmp_s], Duration::from_secs(30))?;
            let first_fpr = fpr.stdout.lines().find(|l| l.starts_with("fpr:")).and_then(|l| l.split(':').nth(9)).unwrap_or_default().to_string();
            if first_fpr != WAYDROID_KEY_FPR {
                let _ = std::fs::remove_file(&tmp);
                return Err(PrivError::StepFailed {
                    step: tr!("verifica chiave repo.waydro.id", "repo.waydro.id key check"),
                    output: tr!("impronta {first_fpr:?} diversa da quella attesa {WAYDROID_KEY_FPR}", "fingerprint {first_fpr:?} differs from the expected {WAYDROID_KEY_FPR}"),
                });
            }
            std::fs::rename(&tmp, &keyring)?;
            let line = format!("deb [signed-by={WAYDROID_KEYRING}] {WAYDROID_REPO_URL} {suite} main\n");
            std::fs::write(fs.path(WAYDROID_SOURCES), &line)?;
            let mut log = step("apt-get", &["update"], Duration::from_secs(900))?;
            let install = step("env", &["DEBIAN_FRONTEND=noninteractive", "apt-get", "install", "-y", "waydroid"], op.timeout())?;
            log.stdout = format!("{}: {first_fpr}\n{line}{}\n{}", tr!("chiave verificata", "key verified"), log.stdout, install.stdout);
            Ok(log)
        }
        PrivilegedOp::InstallDiagnostics => {
            let mut args = vec!["DEBIAN_FRONTEND=noninteractive", "apt-get", "install", "-y"];
            args.extend(DIAGNOSTIC_PACKAGES);
            step("env", &args, op.timeout())
        }
        PrivilegedOp::InstallNvidiaStack { tag, mode } => {
            let release = crate::nvidia_stack::find_release(tag)
                .ok_or_else(|| PrivError::Invalid(tr!("rilascio waydroid-nvidia {tag:?} non fissato in DroidNative", "waydroid-nvidia release {tag:?} is not pinned in DroidNative")))?;
            crate::nvidia_stack::install(release, *mode, fs, cmd)
        }
        PrivilegedOp::NvidiaSetMode { mode } => crate::nvidia_stack::set_mode(*mode, fs, cmd),
        PrivilegedOp::AllowContainerNetwork => {
            let mut log = CmdOutput::ok("");
            for rule in UFW_CONTAINER_RULES {
                let out = step("ufw", rule, op.timeout())?;
                log.stdout.push_str(&out.stdout);
            }
            Ok(log)
        }
        PrivilegedOp::LoadBinder => step("modprobe", &["binder_linux"], op.timeout()),
        PrivilegedOp::EnableContainerService => step("systemctl", &["enable", "--now", CONTAINER_UNIT], op.timeout()),
        PrivilegedOp::RestartContainerService => step("systemctl", &["restart", CONTAINER_UNIT], op.timeout()),
        PrivilegedOp::StopContainerService => step("systemctl", &["stop", CONTAINER_UNIT], op.timeout()),
        PrivilegedOp::InitWaydroid { system_type } => step(&waydroid()?, &["init", "-s", system_type.as_arg()], op.timeout()),
        PrivilegedOp::GraphicsDiagnostics => {
            let bin = waydroid()?;
            let mut out = step(&bin, &["shell", "--", "dumpsys", "SurfaceFlinger"], op.timeout())?;
            // Display-path failures (dmabuf import refused by the host compositor, hwcomposer
            // aborts) only show up in logcat; logd is restarted with the container.
            let log = step(&bin, &["shell", "--", "logcat", "-d", "-v", "time", "-s", "wayland-hwc:E", "hwcomposer:E", "DEBUG:F"], op.timeout())?;
            out.stdout.push_str(crate::gpu::verify::DISPLAY_LOG_MARKER);
            out.stdout.push_str(&log.stdout);
            Ok(out)
        }
        PrivilegedOp::UninstallPackage { package } => {
            if crate::runtime::validate_package_name(package).is_err() {
                return Err(PrivError::Invalid(tr!("nome package non valido: {package:?}", "invalid package name: {package:?}")));
            }
            let out = step(&waydroid()?, &["shell", "--", "pm", "uninstall", package], op.timeout())?;
            if !out.stdout.contains("Success") {
                return Err(PrivError::StepFailed { step: format!("pm uninstall {package}"), output: out.combined() });
            }
            Ok(out)
        }
        PrivilegedOp::SelectGpu { selection } => {
            if let GpuSelection::RenderNode(node) = selection {
                validate_render_node(fs, node)?;
            }
            let bin = waydroid()?;
            let cfg_path = fs.path(WAYDROID_CFG);
            let current = std::fs::read_to_string(&cfg_path)
                .map_err(|_| PrivError::Invalid(tr!("waydroid.cfg assente: inizializza prima Android", "waydroid.cfg missing: initialize Android first")))?;
            replace_file(&cfg_path, &apply_gpu_selection(&current, selection))?;
            // Regenerates the base props (ro.hardware.egl/gralloc) from the new config.
            let mut log = step(&bin, &["upgrade", "-o"], op.timeout())?;
            let active = cmd.run("systemctl", &["is-active", CONTAINER_UNIT]).map(|o| o.stdout.trim() == "active").unwrap_or(false);
            if active {
                let restart = step("systemctl", &["restart", CONTAINER_UNIT], op.timeout())?;
                log.stdout.push_str(&restart.stdout);
            }
            Ok(log)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::FakeRunner;

    const CFG: &str = "[waydroid]\narch = x86_64\nsuspend_action = freeze\n\n[properties]\nro.hardware.gralloc = default\nro.hardware.egl = swiftshader\npersist.waydroid.multi_windows = true\n";

    #[test]
    fn select_render_node_removes_software_overrides() {
        let out = apply_gpu_selection(CFG, &GpuSelection::RenderNode("/dev/dri/renderD128".into()));
        let ini = Ini::parse(&out);
        assert_eq!(ini.get("waydroid", "drm_device"), Some("/dev/dri/renderD128"));
        assert_eq!(ini.get("properties", "ro.hardware.egl"), None);
        assert_eq!(ini.get("properties", "persist.waydroid.multi_windows"), Some("true"));
    }

    #[test]
    fn select_software_and_auto() {
        let sw = apply_gpu_selection("[waydroid]\ndrm_device = /dev/dri/renderD128\n", &GpuSelection::Software);
        let ini = Ini::parse(&sw);
        assert_eq!(ini.get("waydroid", "drm_device"), None);
        assert_eq!(ini.get("properties", "ro.hardware.egl"), Some("swiftshader"));
        let auto = Ini::parse(&apply_gpu_selection(&sw, &GpuSelection::Auto));
        assert_eq!(auto.get("properties", "ro.hardware.egl"), None);
    }

    #[test]
    fn user_set_egl_override_is_kept_by_auto() {
        let cfg = "[properties]\nro.hardware.egl = angle\n";
        let out = Ini::parse(&apply_gpu_selection(cfg, &GpuSelection::Auto));
        assert_eq!(out.get("properties", "ro.hardware.egl"), Some("angle"));
    }

    #[test]
    fn render_node_validation() {
        let dir = tempfile::tempdir().unwrap();
        crate::gpu::fixtures::hybrid_nvidia_display(dir.path());
        let fs = HostFs::rooted(dir.path());
        assert!(validate_render_node(&fs, "/dev/dri/renderD128").is_ok());
        assert!(matches!(validate_render_node(&fs, "/dev/dri/renderD129"), Err(PrivError::Invalid(m)) if m.contains("NVIDIA")));
        assert!(validate_render_node(&fs, "/dev/dri/renderD999").is_err());
        assert!(validate_render_node(&fs, "/dev/dri/../../etc/shadow").is_err());
        assert!(validate_render_node(&fs, "/dev/dri/card1").is_err());
    }

    #[test]
    fn select_gpu_end_to_end_with_fake_commands() {
        let dir = tempfile::tempdir().unwrap();
        crate::gpu::fixtures::hybrid_nvidia_display(dir.path());
        let cfg = dir.path().join("var/lib/waydroid/waydroid.cfg");
        std::fs::create_dir_all(cfg.parent().unwrap()).unwrap();
        std::fs::write(&cfg, CFG).unwrap();
        let runner = FakeRunner::new()
            .with_program("waydroid")
            .respond("/usr/bin/waydroid upgrade -o", CmdOutput::ok("upgraded\n"))
            .respond("systemctl is-active waydroid-container.service", CmdOutput::ok("active\n"))
            .respond("systemctl restart waydroid-container.service", CmdOutput::ok(""));
        let op = PrivilegedOp::SelectGpu { selection: GpuSelection::RenderNode("/dev/dri/renderD128".into()) };
        execute(&op, &HostFs::rooted(dir.path()), &runner).unwrap();
        let written = std::fs::read_to_string(&cfg).unwrap();
        assert!(written.contains("drm_device = /dev/dri/renderD128"));
        assert!(cfg.with_extension("cfg.droidnative.bak").exists());
        assert_eq!(runner.calls().last().unwrap(), "systemctl restart waydroid-container.service");
    }

    fn install_fixture(fpr: &str) -> (tempfile::TempDir, FakeRunner) {
        let dir = tempfile::tempdir().unwrap();
        for d in ["usr/share/keyrings", "etc/apt/sources.list.d"] {
            std::fs::create_dir_all(dir.path().join(d)).unwrap();
        }
        let tmp = dir.path().join("usr/share/keyrings/waydroid.gpg.droidnative.tmp");
        std::fs::write(&tmp, b"key").unwrap();
        let tmp_s = tmp.to_string_lossy().into_owned();
        let runner = FakeRunner::new()
            .respond(&format!("curl -fsS --proto =https --tlsv1.2 -m 60 {WAYDROID_KEY_URL} -o {tmp_s}"), CmdOutput::ok(""))
            .respond(
                &format!("gpg --show-keys --with-colons --with-fingerprint {tmp_s}"),
                CmdOutput::ok(format!("pub:-:3072:1:0E406D181DCEE19C:1691096347:::-:::scESC:\nfpr:::::::::{fpr}:\nsub:-:3072\nfpr:::::::::B7B62E863405C6E4E18DA95C2BF5573548220A06:\n")),
            )
            .respond("apt-get update", CmdOutput::ok("Hit:1 https://repo.waydro.id resolute InRelease\n"))
            .respond("env DEBIAN_FRONTEND=noninteractive apt-get install -y waydroid", CmdOutput::ok("Setting up waydroid\n"));
        (dir, runner)
    }

    #[test]
    fn install_waydroid_pins_key_fingerprint() {
        let (dir, runner) = install_fixture(WAYDROID_KEY_FPR);
        let op = PrivilegedOp::InstallWaydroid { suite: "resolute".into() };
        execute(&op, &HostFs::rooted(dir.path()), &runner).unwrap();
        assert!(dir.path().join("usr/share/keyrings/waydroid.gpg").exists());
        let list = std::fs::read_to_string(dir.path().join("etc/apt/sources.list.d/waydroid.list")).unwrap();
        assert_eq!(list, "deb [signed-by=/usr/share/keyrings/waydroid.gpg] https://repo.waydro.id/ resolute main\n");
    }

    #[test]
    fn install_waydroid_rejects_wrong_key_and_bad_suite() {
        let (dir, runner) = install_fixture("0000000000000000000000000000000000000000");
        let op = PrivilegedOp::InstallWaydroid { suite: "resolute".into() };
        match execute(&op, &HostFs::rooted(dir.path()), &runner) {
            Err(PrivError::StepFailed { output, .. }) => assert!(output.contains("impronta")),
            other => panic!("unexpected {other:?}"),
        }
        assert!(!dir.path().join("etc/apt/sources.list.d/waydroid.list").exists());
        assert!(!runner.calls().iter().any(|c| c.contains("apt-get")));

        let bad = PrivilegedOp::InstallWaydroid { suite: "resolute main\ndeb http://evil".into() };
        assert!(matches!(execute(&bad, &HostFs::rooted(dir.path()), &runner), Err(PrivError::Invalid(_))));
    }

    #[test]
    fn container_network_rules_are_scoped_to_the_bridge() {
        let runner = FakeRunner::new()
            .respond("ufw allow in on waydroid0 to any port 67 proto udp", CmdOutput::ok("Rule added\n"))
            .respond("ufw allow in on waydroid0 to any port 53", CmdOutput::ok("Rule added\n"))
            .respond("ufw route allow in on waydroid0", CmdOutput::ok("Rule added\n"));
        execute(&PrivilegedOp::AllowContainerNetwork, &HostFs::rooted("/nonexistent"), &runner).unwrap();
        assert_eq!(runner.calls().len(), 3);
        assert!(runner.calls().iter().all(|c| c.contains("waydroid0")));
    }

    #[test]
    fn op_args_round_trip_shape() {
        let op = PrivilegedOp::SelectGpu { selection: GpuSelection::RenderNode("/dev/dri/renderD128".into()) };
        assert_eq!(op.to_args(), vec!["select-gpu", "--render-node", "/dev/dri/renderD128"]);
        assert_eq!(PrivilegedOp::InitWaydroid { system_type: SystemType::Gapps }.to_args(), vec!["init", "--system-type", "GAPPS"]);
    }

    #[test]
    fn pkexec_dismissed_maps_to_auth_denied() {
        let runner = Arc::new(
            FakeRunner::new()
                .with_program("pkexec")
                .respond("/usr/bin/pkexec /usr/bin/droidnative priv --lang it load-binder", CmdOutput::failed(126, "Error executing command as another user: Not authorized")),
        );
        let esc = PkexecEscalator { helper: PathBuf::from("/usr/bin/droidnative"), cmd: runner };
        assert!(matches!(esc.run(&PrivilegedOp::LoadBinder), Err(PrivError::AuthDenied)));
    }

    #[test]
    fn sudo_escalator_is_non_interactive() {
        let runner = Arc::new(
            FakeRunner::new()
                .with_program("sudo")
                .respond("/usr/bin/sudo -n /usr/bin/droidnative priv --lang it load-binder", CmdOutput::failed(1, "sudo: a password is required\n"))
                .respond("/usr/bin/sudo -n /usr/bin/droidnative priv --lang it graphics-diagnostics", CmdOutput::ok("GLES: AMD\n")),
        );
        let esc = SudoEscalator { helper: PathBuf::from("/usr/bin/droidnative"), cmd: runner };
        assert!(matches!(esc.run(&PrivilegedOp::LoadBinder), Err(PrivError::AuthDenied)));
        assert!(esc.run(&PrivilegedOp::GraphicsDiagnostics).unwrap().success());
    }
}
