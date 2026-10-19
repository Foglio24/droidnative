//! Userspace and kernel graphics stack: NVIDIA driver, Mesa, Vulkan ICDs, OpenGL, udmabuf.

use serde::{Deserialize, Serialize};

use crate::exec::CommandRunner;
use crate::fs::HostFs;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NvidiaModuleFlavor {
    /// open-gpu-kernel-modules: DMA-BUF export, Turing or newer only.
    Open,
    /// Closed kernel module: no DMA-BUF.
    Proprietary,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NvidiaDriver {
    pub version: Option<String>,
    pub module_flavor: NvidiaModuleFlavor,
    /// `nvidia-drm.modeset`; DMA-BUF and sync_fd support need it enabled.
    pub modeset: Option<bool>,
    pub fbdev: Option<bool>,
    pub gbm_backend: bool,
    pub egl_wayland: bool,
}

impl NvidiaDriver {
    /// (major, minor) of the driver version, e.g. 615.71.09 → (615, 71).
    pub fn version_tuple(&self) -> Option<(u32, u32)> {
        let v = self.version.as_deref()?;
        let mut it = v.split('.');
        Some((it.next()?.parse().ok()?, it.next().and_then(|m| m.parse().ok()).unwrap_or(0)))
    }
}

pub fn parse_nvidia_proc_version(text: &str) -> (Option<String>, NvidiaModuleFlavor) {
    let Some(line) = text.lines().find(|l| l.starts_with("NVRM version:")) else {
        return (None, NvidiaModuleFlavor::Unknown);
    };
    let flavor = if line.contains("Open Kernel Module") { NvidiaModuleFlavor::Open } else { NvidiaModuleFlavor::Proprietary };
    let version = line
        .split_whitespace()
        .find(|tok| tok.contains('.') && tok.chars().all(|c| c.is_ascii_digit() || c == '.') && tok.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(str::to_string);
    (version, flavor)
}

pub fn probe_nvidia(fs: &HostFs, multiarch: &str) -> NvidiaDriver {
    let (version, module_flavor) = fs
        .read("/proc/driver/nvidia/version")
        .map(|t| parse_nvidia_proc_version(&t))
        .unwrap_or((None, NvidiaModuleFlavor::Unknown));
    let yn = |p: &str| fs.read_trimmed(p).map(|v| v == "Y" || v == "1");
    let gbm = fs.list(format!("/usr/lib/{multiarch}/gbm"));
    let ext = fs.list("/usr/share/egl/egl_external_platform.d");
    NvidiaDriver {
        version,
        module_flavor,
        modeset: yn("/sys/module/nvidia_drm/parameters/modeset"),
        fbdev: yn("/sys/module/nvidia_drm/parameters/fbdev"),
        gbm_backend: gbm.iter().any(|f| f.starts_with("nvidia")),
        egl_wayland: ext.iter().any(|f| f.contains("nvidia_wayland")),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MesaInfo {
    /// Upstream version, e.g. "26.0.8".
    pub version: String,
    /// Distribution package version, e.g. "26.0.8-1ubuntu0.3".
    pub package_version: Option<String>,
    pub source: String,
}

pub fn probe_mesa(cmd: &dyn CommandRunner) -> Option<MesaInfo> {
    if cmd.which("dpkg-query").is_some() {
        for pkg in ["libgl1-mesa-dri", "libegl-mesa0", "mesa-vulkan-drivers"] {
            if let Ok(out) = cmd.run("dpkg-query", &["-W", "-f=${Status}|${Version}", pkg]) {
                if let Some((status, ver)) = out.stdout.split_once('|') {
                    if out.success() && status.contains("ok installed") && !ver.is_empty() {
                        return Some(MesaInfo { version: upstream_version(ver), package_version: Some(ver.to_string()), source: format!("dpkg:{pkg}") });
                    }
                }
            }
        }
    }
    None
}

/// "1:26.0.8-1ubuntu0.3" → "26.0.8"
pub fn upstream_version(pkg_version: &str) -> String {
    let v = pkg_version.split_once(':').map(|(_, r)| r).unwrap_or(pkg_version);
    let v = v.split('-').next().unwrap_or(v);
    v.split('+').next().unwrap_or(v).split('~').next().unwrap_or(v).to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulkanIcd {
    pub file: String,
    pub library: Option<String>,
    pub api_version: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VulkanDevice {
    pub name: String,
    pub driver: Option<String>,
    pub api_version: Option<String>,
    pub device_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulkanInfo {
    pub loader_version: Option<String>,
    pub icds: Vec<VulkanIcd>,
    /// Physical devices reported by `vulkaninfo --summary` (empty if the tool is missing).
    pub devices: Vec<VulkanDevice>,
    pub vulkaninfo_available: bool,
}

pub fn probe_vulkan(fs: &HostFs, cmd: &dyn CommandRunner) -> VulkanInfo {
    let mut icds = Vec::new();
    for dir in ["/usr/share/vulkan/icd.d", "/etc/vulkan/icd.d", "/usr/local/share/vulkan/icd.d"] {
        for file in fs.list(dir).into_iter().filter(|f| f.ends_with(".json")) {
            let json: Option<serde_json::Value> = fs.read(format!("{dir}/{file}")).and_then(|t| serde_json::from_str(&t).ok());
            let icd = json.as_ref().and_then(|j| j.get("ICD"));
            icds.push(VulkanIcd {
                library: icd.and_then(|i| i.get("library_path")).and_then(|v| v.as_str()).map(str::to_string),
                api_version: icd.and_then(|i| i.get("api_version")).and_then(|v| v.as_str()).map(str::to_string),
                file,
            });
        }
    }
    let loader_version = cmd
        .which("dpkg-query")
        .and_then(|_| cmd.run("dpkg-query", &["-W", "-f=${Version}", "libvulkan1"]).ok())
        .filter(|o| o.success() && !o.stdout.trim().is_empty())
        .map(|o| upstream_version(o.stdout.trim()));
    let vulkaninfo_available = cmd.which("vulkaninfo").is_some();
    let devices = if vulkaninfo_available {
        cmd.run("vulkaninfo", &["--summary"]).ok().map(|o| parse_vulkaninfo_summary(&o.stdout)).unwrap_or_default()
    } else {
        Vec::new()
    };
    VulkanInfo { loader_version, icds, devices, vulkaninfo_available }
}

pub fn parse_vulkaninfo_summary(text: &str) -> Vec<VulkanDevice> {
    let mut devices = Vec::new();
    let mut current: Option<VulkanDevice> = None;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("GPU") && t.ends_with(':') {
            if let Some(d) = current.take() {
                devices.push(d);
            }
            current = Some(VulkanDevice::default());
            continue;
        }
        let (Some(dev), Some((k, v))) = (current.as_mut(), t.split_once('=')) else { continue };
        let v = v.trim().to_string();
        match k.trim() {
            "deviceName" => dev.name = v,
            "driverName" => dev.driver = Some(v),
            "apiVersion" => dev.api_version = Some(v),
            "deviceType" => dev.device_type = Some(v.trim_start_matches("PHYSICAL_DEVICE_TYPE_").to_string()),
            _ => {}
        }
    }
    devices.extend(current);
    devices.retain(|d| !d.name.is_empty());
    devices
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenGlInfo {
    pub source: String,
    pub vendor: Option<String>,
    pub renderer: Option<String>,
    pub version: Option<String>,
    pub es_version: Option<String>,
}

/// Host OpenGL as seen by the desktop (needs `glxinfo`/`eglinfo` from mesa-utils).
pub fn probe_opengl(cmd: &dyn CommandRunner) -> Option<OpenGlInfo> {
    for tool in ["glxinfo", "eglinfo"] {
        if cmd.which(tool).is_none() {
            continue;
        }
        if let Ok(out) = cmd.run(tool, &["-B"]) {
            if out.success() {
                if let Some(info) = parse_gl_info(tool, &out.stdout) {
                    return Some(info);
                }
            }
        }
    }
    None
}

pub fn parse_gl_info(source: &str, text: &str) -> Option<OpenGlInfo> {
    let find = |keys: &[&str]| {
        text.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            keys.iter().any(|key| k.trim().eq_ignore_ascii_case(key)).then(|| v.trim().to_string())
        })
    };
    let renderer = find(&["OpenGL renderer string", "OpenGL core profile renderer", "OpenGL ES profile renderer"]);
    renderer.as_ref()?;
    Some(OpenGlInfo {
        source: source.to_string(),
        vendor: find(&["OpenGL vendor string", "OpenGL core profile vendor", "OpenGL ES profile vendor"]),
        renderer,
        version: find(&["OpenGL version string", "OpenGL core profile version"]),
        es_version: find(&["OpenGL ES profile version string", "OpenGL ES profile version"]),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceAccess {
    pub path: String,
    pub exists: bool,
    /// The current user can open it read-write.
    pub accessible: bool,
}

impl DeviceAccess {
    pub fn check(fs: &HostFs, host_path: &str) -> Self {
        let mapped = fs.path(host_path);
        let exists = mapped.exists();
        let accessible = exists && {
            use std::os::unix::ffi::OsStrExt;
            std::ffi::CString::new(mapped.as_os_str().as_bytes())
                // SAFETY: valid NUL-terminated path; access(2) has no other preconditions.
                .map(|c| unsafe { libc::access(c.as_ptr(), libc::R_OK | libc::W_OK) } == 0)
                .unwrap_or(false)
        };
        Self { path: host_path.to_string(), exists, accessible }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::{CmdOutput, FakeRunner};

    #[test]
    fn nvidia_open_module_version() {
        let (v, f) = parse_nvidia_proc_version(
            "NVRM version: NVIDIA UNIX Open Kernel Module for x86_64  615.71.09  Release Build  (dvs-builder@U22)  Fri Sep  4 22:33:33 UTC 2026\nGCC version:  gcc version 15.2.0\n",
        );
        assert_eq!(v.as_deref(), Some("615.71.09"));
        assert_eq!(f, NvidiaModuleFlavor::Open);
        let drv = NvidiaDriver { version: v, module_flavor: f, modeset: Some(true), fbdev: None, gbm_backend: true, egl_wayland: true };
        assert_eq!(drv.version_tuple(), Some((615, 71)));
    }

    #[test]
    fn nvidia_closed_module_version() {
        let (v, f) = parse_nvidia_proc_version("NVRM version: NVIDIA UNIX x86_64 Kernel Module  550.107.02  Wed Jul 24 23:53:00 UTC 2024\n");
        assert_eq!(v.as_deref(), Some("550.107.02"));
        assert_eq!(f, NvidiaModuleFlavor::Proprietary);
    }

    #[test]
    fn debian_version_to_upstream() {
        assert_eq!(upstream_version("26.0.8-1ubuntu0.3"), "26.0.8");
        assert_eq!(upstream_version("1:1.4.341.0-1"), "1.4.341.0");
        assert_eq!(upstream_version("24.2.8+ds-1~bpo12+1"), "24.2.8");
    }

    #[test]
    fn mesa_from_dpkg() {
        let r = FakeRunner::new()
            .with_program("dpkg-query")
            .respond("dpkg-query -W -f=${Status}|${Version} libgl1-mesa-dri", CmdOutput::ok("install ok installed|26.0.8-1ubuntu0.3"));
        let m = probe_mesa(&r).unwrap();
        assert_eq!(m.version, "26.0.8");
        assert_eq!(m.source, "dpkg:libgl1-mesa-dri");
    }

    #[test]
    fn vulkaninfo_summary() {
        let text = "Devices:\n========\nGPU0:\n\tapiVersion         = 1.4.312\n\tdeviceType         = PHYSICAL_DEVICE_TYPE_DISCRETE_GPU\n\tdeviceName         = NVIDIA GeForce RTX 4070 Ti SUPER\n\tdriverName         = NVIDIA\nGPU1:\n\tapiVersion         = 1.4.318\n\tdeviceType         = PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU\n\tdeviceName         = AMD Radeon Graphics (RADV RAPHAEL_MENDOCINO)\n\tdriverName         = radv\n";
        let d = parse_vulkaninfo_summary(text);
        assert_eq!(d.len(), 2);
        assert_eq!(d[0].name, "NVIDIA GeForce RTX 4070 Ti SUPER");
        assert_eq!(d[1].driver.as_deref(), Some("radv"));
        assert_eq!(d[1].device_type.as_deref(), Some("INTEGRATED_GPU"));
    }

    #[test]
    fn glxinfo_parsing() {
        let text = "name of display: :0\nOpenGL vendor string: NVIDIA Corporation\nOpenGL renderer string: NVIDIA GeForce RTX 4070 Ti SUPER/PCIe/SSE2\nOpenGL version string: 4.6.0 NVIDIA 615.71.09\nOpenGL ES profile version string: OpenGL ES 3.2 NVIDIA 615.71.09\n";
        let gl = parse_gl_info("glxinfo", text).unwrap();
        assert_eq!(gl.version.as_deref(), Some("4.6.0 NVIDIA 615.71.09"));
        assert_eq!(gl.es_version.as_deref(), Some("OpenGL ES 3.2 NVIDIA 615.71.09"));
    }

    #[test]
    fn vulkan_icds_from_json() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("usr/share/vulkan/icd.d");
        std::fs::create_dir_all(&p).unwrap();
        std::fs::write(p.join("nvidia_icd.json"), r#"{"file_format_version":"1.0.1","ICD":{"library_path":"libGLX_nvidia.so.0","api_version":"1.4.312"}}"#).unwrap();
        let v = probe_vulkan(&HostFs::rooted(d.path()), &FakeRunner::new());
        assert_eq!(v.icds.len(), 1);
        assert_eq!(v.icds[0].api_version.as_deref(), Some("1.4.312"));
        assert!(!v.vulkaninfo_available);
    }
}
