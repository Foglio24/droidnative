//! Deciding whether Android really renders on the GPU.
//!
//! The source of truth is the `GLES:` line SurfaceFlinger prints in `dumpsys SurfaceFlinger`:
//! it names the EGL implementation actually loaded by the compositor, so a misconfigured
//! stack that silently fell back to SwiftShader cannot pass as accelerated.

use serde::{Deserialize, Serialize};

use super::GpuVendor;
use crate::tr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccelStatus {
    /// SurfaceFlinger renders on a real GPU.
    HardwareVerified,
    /// SurfaceFlinger renders on the CPU (SwiftShader, llvmpipe, softpipe).
    Software,
    /// Android is not running or the output could not be read.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayStatus {
    /// No display-path error logged since the container started.
    Ok,
    /// The host compositor refused Android's buffers or the hwcomposer crashed.
    Failed,
    /// No diagnostics available.
    Unknown,
}

/// Whether Android's frames actually reach the Linux desktop. Rendering on the GPU is not
/// enough: on a hybrid machine the compositor may be unable to import the buffers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplayHealth {
    pub status: DisplayStatus,
    pub detail: String,
    /// Log lines backing the verdict (at most a handful).
    pub evidence: Vec<String>,
}

impl DisplayHealth {
    pub fn unknown() -> Self {
        Self { status: DisplayStatus::Unknown, detail: tr!("diagnostica non disponibile", "diagnostics not available"), evidence: Vec::new() }
    }
}

/// Separates `dumpsys SurfaceFlinger` from the filtered logcat in the diagnostics output.
pub const DISPLAY_LOG_MARKER: &str = "\n===DROIDNATIVE-DISPLAY-LOG===\n";

pub fn classify_display(log: &str) -> DisplayHealth {
    let import = log.lines().filter(|l| l.contains("failed to import supplied dmabufs")).collect::<Vec<_>>();
    let disconnected = log.lines().filter(|l| l.contains("Wayland client was disconnected")).collect::<Vec<_>>();
    let crashes = log.lines().filter(|l| l.contains("Cmdline:") && l.contains("composer")).collect::<Vec<_>>();
    if import.is_empty() && disconnected.is_empty() && crashes.is_empty() {
        return DisplayHealth {
            status: DisplayStatus::Ok,
            detail: tr!("nessun errore di visualizzazione registrato dall'avvio del container", "no display error logged since the container started"),
            evidence: Vec::new(),
        };
    }
    let times = |n: usize| if n == 1 { tr!("1 volta", "once") } else { tr!("{n} volte", "{n} times") };
    let mut detail = Vec::new();
    if !import.is_empty() {
        let t = times(import.len());
        detail.push(tr!("il compositor del desktop ha rifiutato i buffer di Android {t}", "the desktop compositor refused Android's buffers {t}"));
    }
    if !crashes.is_empty() {
        let t = times(crashes.len());
        detail.push(tr!("l'hwcomposer di Android è andato in crash {t}", "Android's hwcomposer crashed {t}"));
    } else if !disconnected.is_empty() {
        detail.push(tr!("l'hwcomposer è stato disconnesso dal compositor", "the hwcomposer was disconnected by the compositor"));
    }
    let evidence = import.iter().chain(disconnected.iter()).chain(crashes.iter()).take(5).map(|l| l.trim().to_string()).collect();
    DisplayHealth { status: DisplayStatus::Failed, detail: detail.join("; "), evidence }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccelVerdict {
    pub status: AccelStatus,
    pub vendor: Option<GpuVendor>,
    /// Graphics path recognised from the renderer string.
    pub path: Option<String>,
    /// The raw `GLES:` line, shown to the user as evidence.
    pub gles_line: Option<String>,
    pub detail: String,
    pub display: DisplayHealth,
}

impl AccelVerdict {
    pub fn unknown(detail: impl Into<String>) -> Self {
        Self { status: AccelStatus::Unknown, vendor: None, path: None, gles_line: None, detail: detail.into(), display: DisplayHealth::unknown() }
    }
}

/// Classify the output of the `graphics-diagnostics` privileged operation.
pub fn classify_graphics(output: &str) -> AccelVerdict {
    let (dumpsys, log) = output.split_once(DISPLAY_LOG_MARKER.trim()).unwrap_or((output, ""));
    let mut verdict = classify_gles(dumpsys);
    if output.contains(DISPLAY_LOG_MARKER.trim()) {
        verdict.display = classify_display(log);
    }
    verdict
}

const SOFTWARE_MARKERS: &[&str] = &["swiftshader", "llvmpipe", "softpipe", "lavapipe", "software rasterizer"];

/// Classify the output of `dumpsys SurfaceFlinger`.
pub fn classify_gles(dumpsys: &str) -> AccelVerdict {
    let Some(line) = dumpsys.lines().map(str::trim).find(|l| l.starts_with("GLES:")) else {
        return AccelVerdict::unknown(tr!("Nessuna riga GLES nell'output di SurfaceFlinger", "No GLES line in SurfaceFlinger's output"));
    };
    let lower = line.to_ascii_lowercase();
    let verdict = |status, vendor, path: &str, detail: &str| AccelVerdict {
        status,
        vendor,
        path: Some(path.to_string()),
        gles_line: Some(line.to_string()),
        detail: detail.to_string(),
        display: DisplayHealth::unknown(),
    };

    if SOFTWARE_MARKERS.iter().any(|m| lower.contains(m)) {
        // ANGLE on SwiftShader-Vulkan is still CPU rendering.
        return verdict(AccelStatus::Software, None, "software", &tr!("Rendering software: la GPU non viene utilizzata", "Software rendering: the GPU is not used"));
    }
    if lower.contains("venus") && lower.contains("nvidia") {
        return verdict(AccelStatus::HardwareVerified, Some(GpuVendor::Nvidia), "angle-venus", &tr!("GPU NVIDIA tramite Venus (waydroid-nvidia)", "NVIDIA GPU through Venus (waydroid-nvidia)"));
    }
    if lower.contains("venus") {
        return verdict(AccelStatus::HardwareVerified, None, "angle-venus", &tr!("GPU host tramite Venus", "Host GPU through Venus"));
    }
    if lower.contains("nvidia") || lower.contains("nvk") {
        return verdict(AccelStatus::HardwareVerified, Some(GpuVendor::Nvidia), "nvidia-native", &tr!("GPU NVIDIA con driver nel guest", "NVIDIA GPU with the driver inside Android"));
    }
    if lower.contains("radeonsi") || lower.contains("amd") || lower.contains("radeon") || lower.contains("radv") {
        return verdict(AccelStatus::HardwareVerified, Some(GpuVendor::Amd), "mesa", &tr!("GPU AMD tramite Mesa", "AMD GPU through Mesa"));
    }
    if lower.contains("intel") || lower.contains("iris") || lower.contains("crocus") || lower.contains("anv") {
        return verdict(AccelStatus::HardwareVerified, Some(GpuVendor::Intel), "mesa", &tr!("GPU Intel tramite Mesa", "Intel GPU through Mesa"));
    }
    if lower.contains("virgl") || lower.contains("virtio") {
        return verdict(AccelStatus::HardwareVerified, Some(GpuVendor::VirtIo), "virgl", &tr!("GPU virtuale (virgl)", "Virtual GPU (virgl)"));
    }
    if lower.contains("mesa") || lower.contains("freedreno") || lower.contains("panfrost") || lower.contains("mali") {
        return verdict(AccelStatus::HardwareVerified, None, "mesa", &tr!("GPU tramite Mesa", "GPU through Mesa"));
    }
    AccelVerdict {
        status: AccelStatus::Unknown,
        vendor: None,
        path: None,
        gles_line: Some(line.to_string()),
        detail: tr!("Renderer non riconosciuto: verificare manualmente", "Renderer not recognized: check it manually"),
        display: DisplayHealth::unknown(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swiftshader_is_software() {
        let v = classify_gles("Display 0\nGLES: Google Inc., Google SwiftShader 4.1.0.7, OpenGL ES 3.0 SwiftShader 4.1.0.7\n");
        assert_eq!(v.status, AccelStatus::Software);
    }

    #[test]
    fn angle_on_swiftshader_vulkan_is_software() {
        let v = classify_gles("GLES: Google Inc. (Google), ANGLE (Google, Vulkan 1.3.0 (SwiftShader Device (Subzero) (0x0000C0DE)), SwiftShader driver-5.0.0), OpenGL ES 3.2.0\n");
        assert_eq!(v.status, AccelStatus::Software);
    }

    #[test]
    fn waydroid_nvidia_venus() {
        let v = classify_gles("GLES: Google Inc. (NVIDIA), ANGLE (NVIDIA, Vulkan 1.3.296 (Virtio-GPU Venus (NVIDIA GeForce RTX 4070 Ti SUPER) (0x00002705)), venus driver-26.0.0), OpenGL ES 3.2.0\n");
        assert_eq!(v.status, AccelStatus::HardwareVerified);
        assert_eq!(v.vendor, Some(GpuVendor::Nvidia));
        assert_eq!(v.path.as_deref(), Some("angle-venus"));
    }

    #[test]
    fn mesa_radeonsi() {
        let v = classify_gles("GLES: AMD, AMD Radeon Graphics (radeonsi, raphael_mendocino, LLVM 19.1.7, DRM 3.61, 7.0.0-38-generic), OpenGL ES 3.2 Mesa 25.0.7\n");
        assert_eq!(v.status, AccelStatus::HardwareVerified);
        assert_eq!(v.vendor, Some(GpuVendor::Amd));
    }

    #[test]
    fn mesa_intel() {
        let v = classify_gles("GLES: Intel, Mesa Intel(R) Graphics (RPL-S), OpenGL ES 3.2 Mesa 25.0.7\n");
        assert_eq!(v.vendor, Some(GpuVendor::Intel));
    }

    /// Captured on the development machine: Mesa on the AMD iGPU, GNOME/Mutter on the RTX.
    const HYBRID_FAILURE: &str = "10-19 08:48:23.504 E/wayland-hwc(   92): [destroyed object]: error 7: failed to import supplied dmabufs: Could not bind the given EGLImage to a CoglTexture2D\n10-19 08:48:23.504 E/hwcomposer(   92): *** void *hwc_wayland_thread(void *): Wayland client was disconnected: protocol error: interface ?, object id 0, code 7\n10-19 08:48:23.528 F/DEBUG   (   92): Cmdline: /vendor/bin/hw/android.hardware.graphics.composer@2.1-service --desktop_file_hint=Waydroid.desktop\n";

    #[test]
    fn hybrid_display_failure_is_detected() {
        let out = format!("GLES: AMD, AMD Ryzen 7 7700X 8-Core Processor (radeonsi, raphael_mendocino, ACO, DRM 3.64, 7.0.0-38-generic), OpenGL ES 3.2 Mesa 26.0.8\n{DISPLAY_LOG_MARKER}{HYBRID_FAILURE}");
        let v = classify_graphics(&out);
        assert_eq!(v.status, AccelStatus::HardwareVerified);
        assert_eq!(v.display.status, DisplayStatus::Failed);
        assert!(v.display.detail.contains("rifiutato i buffer"));
        assert!(v.display.detail.contains("crash"));
        assert_eq!(v.display.evidence.len(), 3);
    }

    #[test]
    fn clean_log_is_ok_and_missing_section_unknown() {
        let v = classify_graphics(&format!("GLES: Google Inc., Google SwiftShader 4.1\n{DISPLAY_LOG_MARKER}--------- beginning of main\n"));
        assert_eq!(v.display.status, DisplayStatus::Ok);
        assert_eq!(classify_graphics("GLES: Mesa").display.status, DisplayStatus::Unknown);
    }

    #[test]
    fn missing_line_is_unknown() {
        assert_eq!(classify_gles("error: SurfaceFlinger not running").status, AccelStatus::Unknown);
    }
}
