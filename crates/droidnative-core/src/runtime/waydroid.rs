//! Waydroid backend: drives the `waydroid` CLI as the desktop user and reads its config.
//!
//! Facts from Waydroid's source this module relies on:
//! - `waydroid status` prints `Key:\tValue` lines; `Container:` is FROZEN when suspended.
//! - `waydroid app install` logs failures but still exits 0, so installs are verified
//!   by looking the package up in `waydroid app list` afterwards.
//! - `waydroid app launch` starts a *foreground* session if none is running, so the
//!   session is always started here first.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::{
    validate_apk_path, validate_package_name, validate_prop_key, AndroidApp, AndroidRuntime, GraphicsConfig, InstallOutcome,
    RuntimeError, RuntimeStatus, SessionState,
};
use crate::apk::ApkInfo;
use crate::exec::{CmdOutput, CommandRunner, SystemRunner};
use crate::fs::HostFs;
use crate::gpu::{classify_graphics, AccelVerdict};
use crate::host::{Env, Probe};
use crate::ini::Ini;
use crate::privileged::{Escalator, PrivilegedOp};
use crate::tr;

pub const WAYDROID_WORK: &str = "/var/lib/waydroid";
pub const WAYDROID_CFG: &str = "/var/lib/waydroid/waydroid.cfg";
pub const WAYDROID_LOG: &str = "/var/lib/waydroid/waydroid.log";
pub const CONTAINER_UNIT: &str = "waydroid-container.service";
pub const DHCP_LEASES: &str = "/var/lib/misc/dnsmasq.waydroid0.leases";
const PREINSTALLED_IMAGE_DIRS: &[&str] = &["/etc/waydroid-extra/images", "/usr/share/waydroid-extra/images"];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServiceState {
    pub active: Option<String>,
    pub enabled: Option<String>,
}

impl ServiceState {
    pub fn is_active(&self) -> bool {
        self.active.as_deref() == Some("active")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaydroidImage {
    pub path: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WaydroidConfig {
    pub arch: Option<String>,
    pub vendor_type: Option<String>,
    pub images_path: Option<String>,
    pub drm_device: Option<String>,
    pub suspend_action: Option<String>,
    pub system_datetime: Option<String>,
    pub properties: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaydroidInstall {
    pub installed: bool,
    pub binary: Option<String>,
    pub version: Option<String>,
    pub config_present: bool,
    /// Config plus extracted rootfs directory, the same test `waydroid` itself uses.
    pub initialized: bool,
    pub config: Option<WaydroidConfig>,
    pub images: Vec<WaydroidImage>,
    pub container_service: ServiceState,
    pub user_data_present: bool,
    /// `waydroid-nvidia-setup` from Shiro836/waydroid-nvidia.
    pub nvidia_setup: Option<String>,
    /// User unit of the Venus render server (waydroid-nvidia `main`).
    pub venus_service: ServiceState,
    /// IPv4 address dnsmasq leased to Android: proof the container network works.
    pub dhcp_lease: Option<String>,
    /// dpkg status of the distribution's `waydroid` package (None when not managed by dpkg).
    pub distro_package: Option<String>,
    /// `nvidia_driver_mode` written by waydroid-nvidia-setup.
    pub nvidia_mode: Option<crate::nvidia_stack::NvidiaMode>,
    /// Release DroidNative installed (state file written by the privileged helper).
    pub nvidia_stack: Option<crate::nvidia_stack::StackState>,
    /// VANILLA / GAPPS / FOSS, from the system OTA channel.
    pub system_image: Option<crate::extras::SystemImage>,
    /// libndk ARM translation present in the overlay and enabled.
    pub arm_translation: bool,
    /// venus mode with drm_device on another GPU's node: waydroid-nvidia refuses to start the container.
    pub stale_render_node: Option<crate::nvidia_stack::StaleRenderNode>,
}

pub fn parse_config(text: &str) -> WaydroidConfig {
    let ini = Ini::parse(text);
    let get = |k: &str| ini.get("waydroid", k).map(str::to_string);
    WaydroidConfig {
        arch: get("arch"),
        vendor_type: get("vendor_type"),
        images_path: get("images_path"),
        drm_device: get("drm_device"),
        suspend_action: get("suspend_action"),
        system_datetime: get("system_datetime"),
        properties: ini.entries("properties"),
    }
}

fn service_state(cmd: &dyn CommandRunner, user: bool, unit: &str) -> ServiceState {
    if cmd.which("systemctl").is_none() {
        return ServiceState::default();
    }
    let query = |verb: &str| {
        let mut args = Vec::new();
        if user {
            args.push("--user");
        }
        args.extend([verb, unit]);
        cmd.run("systemctl", &args).ok().map(|o| o.stdout.trim().to_string()).filter(|s| !s.is_empty())
    };
    ServiceState { active: query("is-active"), enabled: query("is-enabled") }
}

pub fn detect_install(probe: &Probe) -> WaydroidInstall {
    let fs = probe.fs;
    let binary = probe.cmd.which("waydroid");
    let version = binary.as_ref().and_then(|b| {
        probe.cmd.run(&b.to_string_lossy(), &["-V"]).ok().filter(|o| o.success()).map(|o| o.combined().trim().to_string())
    });
    let config_text = fs.read(WAYDROID_CFG);
    let config = config_text.as_deref().map(parse_config);

    let mut image_dirs: Vec<String> = Vec::new();
    if let Some(p) = config.as_ref().and_then(|c| c.images_path.clone()) {
        image_dirs.push(p);
    }
    image_dirs.push(format!("{WAYDROID_WORK}/images"));
    image_dirs.extend(PREINSTALLED_IMAGE_DIRS.iter().map(|s| s.to_string()));
    image_dirs.dedup();
    let mut images = Vec::new();
    for dir in &image_dirs {
        for name in ["system.img", "vendor.img"] {
            let host_path = format!("{dir}/{name}");
            if let Ok(meta) = std::fs::metadata(fs.path(&host_path)) {
                if !images.iter().any(|i: &WaydroidImage| i.path == host_path) {
                    images.push(WaydroidImage { path: host_path, size_bytes: meta.len() });
                }
            }
        }
    }

    let user_data_present = probe.env.get("HOME").map(|h| fs.exists(format!("{h}/.local/share/waydroid"))).unwrap_or(false);

    WaydroidInstall {
        installed: binary.is_some(),
        binary: binary.map(|b| b.to_string_lossy().into_owned()),
        version,
        config_present: config.is_some(),
        initialized: config.is_some() && fs.exists(format!("{WAYDROID_WORK}/rootfs")),
        config,
        images,
        container_service: service_state(probe.cmd, false, CONTAINER_UNIT),
        user_data_present,
        nvidia_setup: probe.cmd.which("waydroid-nvidia-setup").map(|p| p.to_string_lossy().into_owned()),
        venus_service: service_state(probe.cmd, true, "wd-venus.service"),
        dhcp_lease: fs.read(DHCP_LEASES).and_then(|t| parse_lease(&t)),
        distro_package: probe
            .cmd
            .which("dpkg-query")
            .and_then(|_| probe.cmd.run("dpkg-query", &["-W", "-f=${Status}", "waydroid"]).ok())
            .filter(|o| o.success())
            .map(|o| o.stdout.trim().to_string())
            .filter(|s| s.contains("installed") || s.contains("config-files")),
        nvidia_mode: config_text.as_deref().and_then(crate::nvidia_stack::configured_mode),
        nvidia_stack: crate::nvidia_stack::read_state(fs),
        system_image: config_text.as_deref().map(crate::extras::system_image),
        arm_translation: config_text.as_deref().map(|t| crate::extras::arm_translation_installed(fs, t)).unwrap_or(false),
        stale_render_node: config_text.as_deref().and_then(|t| crate::nvidia_stack::stale_render_node(fs, t)),
    }
}

/// dnsmasq lease line: `<expiry> <mac> <ip> <hostname> <client-id>`.
pub fn parse_lease(text: &str) -> Option<String> {
    text.lines().filter_map(|l| l.split_whitespace().nth(2)).find(|ip| ip.parse::<std::net::Ipv4Addr>().is_ok()).map(str::to_string)
}

pub fn parse_status(text: &str) -> RuntimeStatus {
    let field = |name: &str| {
        text.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            (k.trim() == name).then(|| v.trim().to_string()).filter(|v| !v.is_empty())
        })
    };
    let container = field("Container");
    let session = match field("Session").as_deref() {
        Some("RUNNING") if container.as_deref() == Some("FROZEN") => SessionState::Frozen,
        Some("RUNNING") => SessionState::Running,
        Some("STOPPED") => SessionState::Stopped,
        _ => SessionState::Unknown,
    };
    RuntimeStatus {
        backend: "waydroid".into(),
        session,
        container,
        vendor_type: field("Vendor type"),
        ip_address: field("IP address").filter(|ip| ip != "UNKNOWN"),
        session_user: field("Session user"),
        wayland_display: field("Wayland display"),
    }
}

pub fn parse_app_list(text: &str) -> Vec<AndroidApp> {
    let mut apps: Vec<AndroidApp> = Vec::new();
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("Name: ") {
            apps.push(AndroidApp { name: name.trim().to_string(), package: String::new(), categories: Vec::new(), icon: None });
        } else if let Some(pkg) = line.strip_prefix("packageName: ") {
            if let Some(app) = apps.last_mut() {
                app.package = pkg.trim().to_string();
            }
        } else if line.starts_with('\t') || line.starts_with("    ") {
            if let Some(app) = apps.last_mut() {
                app.categories.push(line.trim().to_string());
            }
        }
    }
    apps.retain(|a| !a.package.is_empty());
    apps
}

pub struct WaydroidRuntime {
    cmd: Arc<dyn CommandRunner>,
    fs: HostFs,
    state_dir: PathBuf,
    /// `$XDG_DATA_HOME` (or ~/.local/share): where Waydroid writes per-app desktop entries.
    data_home: PathBuf,
    pub start_timeout: Duration,
    pub install_timeout: Duration,
    /// How long to wait for Android to commit an install (dex2oat included) or a removal.
    pub verify_timeout: Duration,
    pub poll_interval: Duration,
}

impl WaydroidRuntime {
    pub fn new(cmd: Arc<dyn CommandRunner>, fs: HostFs, state_dir: PathBuf, data_home: PathBuf) -> Self {
        Self {
            cmd,
            fs,
            state_dir,
            data_home,
            start_timeout: Duration::from_secs(150),
            install_timeout: Duration::from_secs(240),
            verify_timeout: Duration::from_secs(180),
            poll_interval: Duration::from_millis(1000),
        }
    }

    pub fn local() -> Self {
        let env = Env::from_process();
        Self::new(Arc::new(SystemRunner), HostFs::real(), default_state_dir(&env), default_data_home(&env))
    }

    /// Waydroid's session writes this file from its `packageStateChanged` callback, i.e. only
    /// after Android has committed the package (dex2oat finished, package no longer frozen),
    /// and deletes it on removal. The app list alone shows a package while it is still frozen.
    pub fn desktop_entry(&self, package: &str) -> PathBuf {
        self.data_home.join("applications").join(format!("waydroid.{package}.desktop"))
    }

    fn committed_since(&self, package: &str, since: std::time::SystemTime) -> bool {
        std::fs::metadata(self.desktop_entry(package)).and_then(|m| m.modified()).map(|t| t >= since).unwrap_or(false)
    }

    pub fn session_log(&self) -> PathBuf {
        self.state_dir.join("waydroid-session.log")
    }

    fn binary(&self) -> Result<String, RuntimeError> {
        self.cmd.which("waydroid").map(|p| p.to_string_lossy().into_owned()).ok_or(RuntimeError::NotInstalled("Waydroid"))
    }

    fn run(&self, args: &[&str], timeout: Duration) -> Result<CmdOutput, RuntimeError> {
        let bin = self.binary()?;
        Ok(self.cmd.run_with_timeout(&bin, args, timeout)?)
    }

    fn run_checked(&self, args: &[&str], timeout: Duration) -> Result<CmdOutput, RuntimeError> {
        let out = self.run(args, timeout)?;
        if !out.success() {
            return Err(RuntimeError::failed(&format!("waydroid {}", args.join(" ")), &out));
        }
        Ok(out)
    }

    fn ensure_running(&self) -> Result<RuntimeStatus, RuntimeError> {
        let st = self.status()?;
        match st.session {
            SessionState::Running | SessionState::Frozen => Ok(st),
            _ => Err(RuntimeError::SessionNotRunning),
        }
    }

    fn wait_for<T>(&self, timeout: Duration, what: &str, mut check: impl FnMut() -> Option<T>) -> Result<T, RuntimeError> {
        let start = Instant::now();
        loop {
            if let Some(v) = check() {
                return Ok(v);
            }
            if start.elapsed() >= timeout {
                return Err(RuntimeError::Timeout(what.to_string()));
            }
            std::thread::sleep(self.poll_interval);
        }
    }

    fn package_present(&self, package: &str) -> Result<bool, RuntimeError> {
        Ok(self.list_apps()?.iter().any(|a| a.package == package))
    }
}

pub fn default_data_home(env: &Env) -> PathBuf {
    env.get("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| env.get("HOME").map(|h| Path::new(&h).join(".local/share")))
        .unwrap_or_else(std::env::temp_dir)
}

pub fn default_state_dir(env: &Env) -> PathBuf {
    env.get("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| env.get("HOME").map(|h| Path::new(&h).join(".local/state")))
        .unwrap_or_else(std::env::temp_dir)
        .join("droidnative")
}

impl AndroidRuntime for WaydroidRuntime {
    fn id(&self) -> &'static str {
        "waydroid"
    }

    fn status(&self) -> Result<RuntimeStatus, RuntimeError> {
        let out = self.run(&["status"], Duration::from_secs(15))?;
        // Before `waydroid init` the only output is `Waydroid is not initialized, run "waydroid init"`.
        if out.combined().contains("not initialized") {
            return Err(RuntimeError::NotInitialized);
        }
        Ok(parse_status(&out.stdout))
    }

    fn start_session(&self) -> Result<RuntimeStatus, RuntimeError> {
        if !self.fs.exists(WAYDROID_CFG) {
            return Err(RuntimeError::NotInitialized);
        }
        let current = self.status()?;
        if current.session == SessionState::Running {
            return Ok(current);
        }
        if current.session == SessionState::Frozen {
            // Any IPlatform call unfreezes; a property read is the cheapest.
            let _ = self.run(&["prop", "get", "sys.boot_completed"], Duration::from_secs(20));
            return self.status();
        }
        // Without this the session start below would only time out.
        if let Some(stale) = self.fs.read(WAYDROID_CFG).and_then(|t| crate::nvidia_stack::stale_render_node(&self.fs, &t)) {
            return Err(RuntimeError::StaleRenderNode(stale));
        }
        // waydroid-nvidia's venus mode renders through the user-level Venus server; the session
        // cannot draw without it (upstream: `systemctl --user enable --now wd-venus.service`).
        let venus = self.fs.read(WAYDROID_CFG).and_then(|t| crate::nvidia_stack::configured_mode(&t)) == Some(crate::nvidia_stack::NvidiaMode::Venus);
        if venus {
            let active = self.cmd.run("systemctl", &["--user", "is-active", "wd-venus.service"]).map(|o| o.stdout.trim() == "active").unwrap_or(false);
            if !active {
                let out = self.cmd.run_with_timeout("systemctl", &["--user", "enable", "--now", "wd-venus.service"], Duration::from_secs(30))?;
                if !out.success() {
                    return Err(RuntimeError::failed("systemctl --user enable --now wd-venus.service", &out));
                }
            }
        }
        let bin = self.binary()?;
        self.cmd.spawn_detached(&bin, &["session", "start"], &self.session_log())?;
        self.wait_for(self.start_timeout, &tr!("avvio della sessione Waydroid", "Waydroid session start"), || {
            self.status().ok().filter(|s| s.session == SessionState::Running)
        })?;
        // RUNNING means the container is up; Android needs a few more seconds to finish booting.
        self.wait_for(self.start_timeout, &tr!("boot di Android (sys.boot_completed)", "Android boot (sys.boot_completed)"), || {
            self.run(&["prop", "get", "sys.boot_completed"], Duration::from_secs(20))
                .ok()
                .filter(|o| o.stdout.trim() == "1")
        })?;
        self.status()
    }

    fn stop_session(&self) -> Result<(), RuntimeError> {
        self.run_checked(&["session", "stop"], Duration::from_secs(60))?;
        self.wait_for(Duration::from_secs(30), &tr!("arresto della sessione", "session stop"), || {
            self.status().ok().filter(|s| s.session == SessionState::Stopped)
        })?;
        Ok(())
    }

    fn show_full_ui(&self) -> Result<(), RuntimeError> {
        self.ensure_running()?;
        let bin = self.binary()?;
        self.cmd.spawn_detached(&bin, &["show-full-ui"], &self.session_log())?;
        Ok(())
    }

    fn list_apps(&self) -> Result<Vec<AndroidApp>, RuntimeError> {
        self.ensure_running()?;
        let out = self.run_checked(&["app", "list"], Duration::from_secs(30))?;
        let mut apps = parse_app_list(&out.stdout);
        let icons = self.data_home.join("waydroid/data/icons");
        for app in &mut apps {
            let icon = icons.join(format!("{}.png", app.package));
            app.icon = icon.is_file().then(|| icon.to_string_lossy().into_owned());
        }
        Ok(apps)
    }

    fn install_apk(&self, apk: &ApkInfo) -> Result<InstallOutcome, RuntimeError> {
        validate_package_name(&apk.package)?;
        let path = validate_apk_path(Path::new(&apk.path))?;
        self.ensure_running()?;
        let was_update = self.package_present(&apk.package)?;
        let path_str = path.to_string_lossy().into_owned();
        // Filesystem timestamps can be coarser than the clock; allow one second of slack.
        let started = std::time::SystemTime::now() - Duration::from_secs(1);
        let out = self.run(&["app", "install", &path_str], self.install_timeout)?;
        if out.timed_out {
            return Err(RuntimeError::Timeout(tr!("installazione APK", "APK installation")));
        }
        let committed = self
            .wait_for(self.verify_timeout, &tr!("conferma dell'installazione", "install confirmation"), || {
                (self.committed_since(&apk.package, started) && self.package_present(&apk.package).unwrap_or(false)).then_some(())
            })
            .is_ok();
        if !committed {
            let listed = self.package_present(&apk.package).unwrap_or(false);
            let why = if listed {
                tr!(
                    "elencato ma l'installazione non è stata confermata da Android (ancora in compilazione o fallita)",
                    "listed, but Android did not confirm the installation (still compiling or failed)"
                )
            } else {
                tr!("non risulta installato dopo `waydroid app install`", "not installed after `waydroid app install`")
            };
            let output = if out.combined().is_empty() { tr!("(vuoto)", "(empty)") } else { out.combined() };
            let pkg = &apk.package;
            return Err(RuntimeError::Verification(tr!("{pkg}: {why}. Output di waydroid: {output}", "{pkg}: {why}. waydroid output: {output}")));
        }
        Ok(InstallOutcome { package: apk.package.clone(), verified: true, was_update, output: out.combined() })
    }

    fn uninstall(&self, package: &str, escalator: &dyn Escalator) -> Result<String, RuntimeError> {
        validate_package_name(package)?;
        self.ensure_running()?;
        let out = self.run(&["app", "remove", package], Duration::from_secs(120))?;
        let mut method = "waydroid app remove".to_string();
        // Waydroid exits 0 either way; IPlatform.removeApp throws IllegalArgumentException (-3)
        // on the Android 13 images, logged as "Failed with code" / "Failed to uninstall".
        if out.combined().contains("Failed") {
            let pm = escalator.run(&PrivilegedOp::UninstallPackage { package: package.to_string() })?;
            if !pm.success() {
                return Err(RuntimeError::failed(&format!("pm uninstall {package}"), &pm));
            }
            method = format!("pm uninstall (waydroid app remove: {})", out.combined().lines().last().unwrap_or("error").trim());
        }
        let gone = self
            .wait_for(self.verify_timeout, &tr!("rimozione del package", "package removal"), || {
                (!self.desktop_entry(package).exists() && self.package_present(package).ok() == Some(false)).then_some(())
            })
            .is_ok();
        if !gone {
            let output = out.combined();
            return Err(RuntimeError::Verification(tr!("{package} è ancora installato. Output: {output}", "{package} is still installed. Output: {output}")));
        }
        Ok(method)
    }

    fn launch(&self, package: &str) -> Result<(), RuntimeError> {
        validate_package_name(package)?;
        if self.status()?.session == SessionState::Stopped {
            self.start_session()?;
        }
        self.run_checked(&["app", "launch", package], Duration::from_secs(30))?;
        Ok(())
    }

    fn get_prop(&self, key: &str) -> Result<Option<String>, RuntimeError> {
        validate_prop_key(key)?;
        self.ensure_running()?;
        let out = self.run_checked(&["prop", "get", key], Duration::from_secs(20))?;
        // Log lines from waydroid are prefixed with a timestamp in brackets; the value is the last plain line.
        Ok(out.stdout.lines().rev().map(str::trim).find(|l| !l.is_empty() && !l.starts_with('[')).map(str::to_string))
    }

    fn graphics_config(&self) -> Result<GraphicsConfig, RuntimeError> {
        Ok(GraphicsConfig {
            egl: self.get_prop("ro.hardware.egl")?,
            gralloc: self.get_prop("ro.hardware.gralloc")?,
            vulkan: self.get_prop("ro.hardware.vulkan")?,
            gbm_device: self.get_prop("gralloc.gbm.device")?,
        })
    }

    fn verify_acceleration(&self, escalator: &dyn Escalator) -> Result<AccelVerdict, RuntimeError> {
        self.ensure_running()?;
        let out = escalator.run(&PrivilegedOp::GraphicsDiagnostics)?;
        if !out.success() {
            return Err(RuntimeError::failed(&tr!("diagnostica grafica (dumpsys SurfaceFlinger, logcat)", "graphics diagnostics (dumpsys SurfaceFlinger, logcat)"), &out));
        }
        Ok(classify_graphics(&out.stdout))
    }

    fn log_tail(&self, lines: usize) -> Result<String, RuntimeError> {
        let tail = |text: String| {
            let v: Vec<&str> = text.lines().collect();
            v[v.len().saturating_sub(lines)..].join("\n")
        };
        let mut out = String::new();
        if let Ok(t) = std::fs::read_to_string(self.session_log()) {
            out.push_str(&format!("== {} ==\n{}\n", self.session_log().display(), tail(t)));
        }
        if let Some(t) = self.fs.read(WAYDROID_LOG) {
            out.push_str(&format!("== {WAYDROID_LOG} ==\n{}\n", tail(t)));
        }
        if out.is_empty() {
            out.push_str(&tr!("Nessun log disponibile.", "No logs available."));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apk::AbiCompat;
    use crate::exec::FakeRunner;

    const RUNNING: &str = "Session:\tRUNNING\nContainer:\tRUNNING\nVendor type:\tMAINLINE\nIP address:\t192.168.240.112\nSession user:\tuser(1000)\nWayland display:\twayland-0\n";
    const STOPPED: &str = "Session:\tSTOPPED\nVendor type:\tMAINLINE\n";
    const APPS: &str = "Name: Settings\npackageName: com.android.settings\ncategories:\n\tandroid.intent.category.LAUNCHER\nName: Files\npackageName: com.android.documentsui\ncategories:\n";

    fn runtime(runner: FakeRunner, cfg: bool) -> (WaydroidRuntime, Arc<FakeRunner>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        if cfg {
            let p = dir.path().join("var/lib/waydroid");
            std::fs::create_dir_all(&p).unwrap();
            std::fs::write(p.join("waydroid.cfg"), "[waydroid]\narch = x86_64\n").unwrap();
        }
        let runner = Arc::new(runner.with_program("waydroid"));
        let mut rt = WaydroidRuntime::new(runner.clone(), HostFs::rooted(dir.path()), dir.path().join("state"), dir.path().join("share"));
        rt.poll_interval = Duration::from_millis(1);
        rt.start_timeout = Duration::from_millis(200);
        rt.verify_timeout = Duration::from_millis(50);
        (rt, runner, dir)
    }

    fn apk(dir: &Path, package: &str) -> ApkInfo {
        let path = dir.join("app.apk");
        std::fs::write(&path, b"x").unwrap();
        ApkInfo {
            path: path.to_string_lossy().into_owned(),
            file_size: 1,
            sha256: String::new(),
            package: package.into(),
            version_name: None,
            version_code: None,
            min_sdk: None,
            target_sdk: None,
            label: None,
            permissions: vec![],
            native_abis: vec![],
            abi_compat: AbiCompat::Universal,
        }
    }

    struct NoEscalation;
    impl Escalator for NoEscalation {
        fn run(&self, _op: &PrivilegedOp) -> Result<CmdOutput, crate::privileged::PrivError> {
            Err(crate::privileged::PrivError::AuthDenied)
        }
    }

    struct RecordingEscalator(std::sync::Mutex<Vec<PrivilegedOp>>);
    impl Escalator for RecordingEscalator {
        fn run(&self, op: &PrivilegedOp) -> Result<CmdOutput, crate::privileged::PrivError> {
            self.0.lock().unwrap().push(op.clone());
            Ok(CmdOutput::ok("Success\n"))
        }
    }

    #[test]
    fn uninstall_falls_back_to_pm_when_iplatform_fails() {
        let with = format!("{APPS}Name: F-Droid\npackageName: org.fdroid.fdroid\ncategories:\n");
        let runner = FakeRunner::new()
            .respond("/usr/bin/waydroid status", CmdOutput::ok(RUNNING))
            .respond(
                "/usr/bin/waydroid app remove org.fdroid.fdroid",
                CmdOutput { status: Some(0), stdout: String::new(), stderr: "[10:48:59] Failed with code: -3\n[10:48:59] Failed to uninstall package: org.fdroid.fdroid\n".into(), timed_out: false },
            )
            .respond("/usr/bin/waydroid app list", CmdOutput::ok(with))
            .respond("/usr/bin/waydroid app list", CmdOutput::ok(APPS));
        let (rt, _, _d) = runtime(runner, true);
        let esc = RecordingEscalator(Default::default());
        let method = rt.uninstall("org.fdroid.fdroid", &esc).unwrap();
        assert!(method.starts_with("pm uninstall"));
        assert_eq!(esc.0.lock().unwrap().as_slice(), [PrivilegedOp::UninstallPackage { package: "org.fdroid.fdroid".into() }]);
    }

    #[test]
    fn status_parsing() {
        let s = parse_status(RUNNING);
        assert_eq!(s.session, SessionState::Running);
        assert_eq!(s.ip_address.as_deref(), Some("192.168.240.112"));
        assert_eq!(parse_status(STOPPED).session, SessionState::Stopped);
        let frozen = RUNNING.replace("Container:\tRUNNING", "Container:\tFROZEN");
        assert_eq!(parse_status(&frozen).session, SessionState::Frozen);
    }

    #[test]
    fn app_list_parsing() {
        let apps = parse_app_list(APPS);
        assert_eq!(apps.len(), 2);
        assert_eq!(apps[0].package, "com.android.settings");
        assert_eq!(apps[0].categories, vec!["android.intent.category.LAUNCHER"]);
        assert!(apps[1].categories.is_empty());
    }

    #[test]
    fn config_parsing() {
        let c = parse_config("[waydroid]\narch = x86_64\ndrm_device = /dev/dri/renderD128\n\n[properties]\nro.hardware.gralloc = default\n");
        assert_eq!(c.drm_device.as_deref(), Some("/dev/dri/renderD128"));
        assert_eq!(c.properties, vec![("ro.hardware.gralloc".to_string(), "default".to_string())]);
    }

    #[test]
    fn not_installed() {
        let rt = WaydroidRuntime::new(Arc::new(FakeRunner::new()), HostFs::rooted("/nonexistent"), PathBuf::from("/tmp"), PathBuf::from("/tmp"));
        assert!(matches!(rt.status(), Err(RuntimeError::NotInstalled(_))));
    }

    #[test]
    fn uninitialized_status_is_an_explicit_error() {
        let runner = FakeRunner::new().respond("/usr/bin/waydroid status", CmdOutput::ok("Waydroid is not initialized, run \"waydroid init\"\n"));
        let (rt, _, _d) = runtime(runner, false);
        assert!(matches!(rt.status(), Err(RuntimeError::NotInitialized)));
    }

    #[test]
    fn start_requires_initialization() {
        let (rt, _, _d) = runtime(FakeRunner::new(), false);
        assert!(matches!(rt.start_session(), Err(RuntimeError::NotInitialized)));
    }

    #[test]
    fn start_session_spawns_and_waits_for_boot() {
        let runner = FakeRunner::new()
            .respond("/usr/bin/waydroid status", CmdOutput::ok(STOPPED))
            .respond("/usr/bin/waydroid status", CmdOutput::ok(STOPPED))
            .respond("/usr/bin/waydroid status", CmdOutput::ok(RUNNING))
            .respond("/usr/bin/waydroid prop get sys.boot_completed", CmdOutput::ok(""))
            .respond("/usr/bin/waydroid prop get sys.boot_completed", CmdOutput::ok("1\n"));
        let (rt, runner, _d) = runtime(runner, true);
        let st = rt.start_session().unwrap();
        assert_eq!(st.session, SessionState::Running);
        assert!(runner.calls().contains(&"spawn /usr/bin/waydroid session start".to_string()));
    }

    #[test]
    fn venus_mode_starts_the_render_server_first() {
        let runner = FakeRunner::new()
            .respond("/usr/bin/waydroid status", CmdOutput::ok(STOPPED))
            .respond("/usr/bin/waydroid status", CmdOutput::ok(RUNNING))
            .respond("/usr/bin/waydroid prop get sys.boot_completed", CmdOutput::ok("1"))
            .respond("systemctl --user is-active wd-venus.service", CmdOutput { status: Some(3), stdout: "inactive\n".into(), ..Default::default() })
            .respond("systemctl --user enable --now wd-venus.service", CmdOutput::ok(""));
        let (rt, runner, d) = runtime(runner, true);
        std::fs::write(d.path().join("var/lib/waydroid/waydroid.cfg"), "[waydroid]\nnvidia_driver_mode = venus\n").unwrap();
        rt.start_session().unwrap();
        let calls = runner.calls();
        let venus = calls.iter().position(|c| c.contains("enable --now wd-venus")).unwrap();
        let spawn = calls.iter().position(|c| c.contains("session start")).unwrap();
        assert!(venus < spawn);
    }

    #[test]
    fn start_session_times_out() {
        let runner = FakeRunner::new().respond("/usr/bin/waydroid status", CmdOutput::ok(STOPPED));
        let (rt, _, _d) = runtime(runner, true);
        assert!(matches!(rt.start_session(), Err(RuntimeError::Timeout(_))));
    }

    #[test]
    fn install_is_verified_against_app_list() {
        let d = tempfile::tempdir().unwrap();
        let info = apk(d.path(), "org.example.new");
        let path = std::fs::canonicalize(&info.path).unwrap();
        let after = format!("{APPS}Name: New\npackageName: org.example.new\ncategories:\n");
        let runner = FakeRunner::new()
            .respond("/usr/bin/waydroid status", CmdOutput::ok(RUNNING))
            .respond("/usr/bin/waydroid app list", CmdOutput::ok(APPS))
            .respond("/usr/bin/waydroid app list", CmdOutput::ok(after))
            .respond(&format!("/usr/bin/waydroid app install {}", path.display()), CmdOutput::ok(""));
        let (rt, _, _d) = runtime(runner, true);
        mark_committed(&rt, "org.example.new");
        let out = rt.install_apk(&info).unwrap();
        assert!(out.verified);
        assert!(!out.was_update);
    }

    /// What Waydroid's session does once Android commits the package.
    fn mark_committed(rt: &WaydroidRuntime, package: &str) {
        let entry = rt.desktop_entry(package);
        std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
        let f = std::fs::File::create(&entry).unwrap();
        f.set_modified(std::time::SystemTime::now() + Duration::from_secs(5)).unwrap();
    }

    #[test]
    fn listed_but_uncommitted_install_is_not_success() {
        // Seen on real hardware: getAppsInfo lists the package while dex2oat still runs and the
        // package is frozen, so launching it fails with "Package ... is currently frozen!".
        let d = tempfile::tempdir().unwrap();
        let info = apk(d.path(), "org.fdroid.fdroid");
        let path = std::fs::canonicalize(&info.path).unwrap();
        let listed = format!("{APPS}Name: F-Droid\npackageName: org.fdroid.fdroid\ncategories:\n");
        let runner = FakeRunner::new()
            .respond("/usr/bin/waydroid status", CmdOutput::ok(RUNNING))
            .respond("/usr/bin/waydroid app list", CmdOutput::ok(APPS))
            .respond("/usr/bin/waydroid app list", CmdOutput::ok(listed))
            .respond(&format!("/usr/bin/waydroid app install {}", path.display()), CmdOutput::ok(""));
        let (rt, _, _d) = runtime(runner, true);
        match rt.install_apk(&info) {
            Err(RuntimeError::Verification(msg)) => assert!(msg.contains("non è stata confermata")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn stale_desktop_entry_does_not_confirm_an_update() {
        let d = tempfile::tempdir().unwrap();
        let info = apk(d.path(), "com.android.settings");
        let path = std::fs::canonicalize(&info.path).unwrap();
        let runner = FakeRunner::new()
            .respond("/usr/bin/waydroid status", CmdOutput::ok(RUNNING))
            .respond("/usr/bin/waydroid app list", CmdOutput::ok(APPS))
            .respond(&format!("/usr/bin/waydroid app install {}", path.display()), CmdOutput::ok(""));
        let (rt, _, _d) = runtime(runner, true);
        let entry = rt.desktop_entry("com.android.settings");
        std::fs::create_dir_all(entry.parent().unwrap()).unwrap();
        let f = std::fs::File::create(&entry).unwrap();
        f.set_modified(std::time::SystemTime::now() - Duration::from_secs(3600)).unwrap();
        assert!(matches!(rt.install_apk(&info), Err(RuntimeError::Verification(_))));
    }

    #[test]
    fn silent_install_failure_is_reported() {
        // waydroid exits 0 and only logs the error.
        let d = tempfile::tempdir().unwrap();
        let info = apk(d.path(), "org.example.broken");
        let path = std::fs::canonicalize(&info.path).unwrap();
        let runner = FakeRunner::new()
            .respond("/usr/bin/waydroid status", CmdOutput::ok(RUNNING))
            .respond("/usr/bin/waydroid app list", CmdOutput::ok(APPS))
            .respond(
                &format!("/usr/bin/waydroid app install {}", path.display()),
                CmdOutput { status: Some(0), stdout: String::new(), stderr: "[10:00:00] Failed to access IPlatform service\n".into(), timed_out: false },
            );
        let (mut rt, _, _d) = runtime(runner, true);
        rt.install_timeout = Duration::from_secs(1);
        match rt.install_apk(&info) {
            Err(RuntimeError::Verification(msg)) => assert!(msg.contains("IPlatform")),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn rejects_bad_package_before_running_anything() {
        let (rt, runner, _d) = runtime(FakeRunner::new(), true);
        assert!(matches!(rt.launch("--help"), Err(RuntimeError::InvalidInput(_))));
        assert!(matches!(rt.uninstall("a;b", &NoEscalation), Err(RuntimeError::InvalidInput(_))));
        assert!(runner.calls().is_empty());
    }

    #[test]
    fn launch_starts_session_first_when_stopped() {
        let runner = FakeRunner::new()
            .respond("/usr/bin/waydroid status", CmdOutput::ok(STOPPED))
            .respond("/usr/bin/waydroid status", CmdOutput::ok(STOPPED))
            .respond("/usr/bin/waydroid status", CmdOutput::ok(RUNNING))
            .respond("/usr/bin/waydroid prop get sys.boot_completed", CmdOutput::ok("1"))
            .respond("/usr/bin/waydroid app launch org.fdroid.fdroid", CmdOutput::ok(""));
        let (rt, runner, _d) = runtime(runner, true);
        rt.launch("org.fdroid.fdroid").unwrap();
        let calls = runner.calls();
        let spawn = calls.iter().position(|c| c.contains("session start")).unwrap();
        let launch = calls.iter().position(|c| c.contains("app launch")).unwrap();
        assert!(spawn < launch);
    }

    #[test]
    fn detects_missing_install() {
        let dir = tempfile::tempdir().unwrap();
        let fs = HostFs::rooted(dir.path());
        let env = Env::from_pairs(&[("HOME", "/home/u")]);
        let runner = FakeRunner::new();
        let inst = detect_install(&Probe { fs: &fs, cmd: &runner, env: &env });
        assert!(!inst.installed && !inst.initialized && inst.images.is_empty());
    }
}
