//! The four selectable GPU backends, what each requires, and whether this host meets it.

use serde::{Deserialize, Serialize};

use super::{GpuDevice, GpuVendor, NvidiaModuleFlavor};
use crate::host::{HostReport, SessionKind};
use crate::nvidia_stack::NvidiaMode;
use crate::report::{Check, CheckStatus};
use crate::tr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuBackendKind {
    MesaNative,
    NvidiaAccelerated,
    NvidiaExperimental,
    SoftwareFallback,
}

pub const ALL_KINDS: [GpuBackendKind; 4] =
    [GpuBackendKind::MesaNative, GpuBackendKind::NvidiaAccelerated, GpuBackendKind::NvidiaExperimental, GpuBackendKind::SoftwareFallback];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stability {
    Stable,
    Experimental,
    Fallback,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackendDescriptor {
    pub kind: GpuBackendKind,
    pub name: String,
    pub summary: String,
    pub stability: Stability,
    pub requirements: Vec<String>,
    pub compatible_drivers: Vec<String>,
    pub upstream: Option<&'static str>,
    /// What the SurfaceFlinger `GLES:` line looks like when this backend works.
    pub expected_renderer: &'static str,
}

/// Descriptor in the current language.
pub fn descriptor(kind: GpuBackendKind) -> BackendDescriptor {
    let v = |items: &[String]| items.to_vec();
    match kind {
        GpuBackendKind::MesaNative => BackendDescriptor {
            kind,
            name: "Mesa Native".into(),
            summary: tr!(
                "Android usa direttamente il nodo render della GPU tramite Mesa (gralloc gbm + EGL mesa).",
                "Android renders directly on the GPU render node through Mesa (gbm gralloc + Mesa EGL)."
            ),
            stability: Stability::Stable,
            requirements: v(&[
                tr!("GPU AMD o Intel con driver kernel amdgpu/radeon/i915/xe", "AMD or Intel GPU with the amdgpu/radeon/i915/xe kernel driver"),
                tr!("Nodo /dev/dri/renderD* accessibile", "Accessible /dev/dri/renderD* node"),
                tr!("Compositor sulla stessa GPU, oppure import dmabuf cross-GPU funzionante", "Compositor on the same GPU, or working cross-GPU dmabuf import"),
                tr!("Sessione Wayland", "Wayland session"),
            ]),
            compatible_drivers: v(&["amdgpu (RadeonSI, RADV)".into(), "radeon".into(), "i915 (Iris, ANV)".into(), "xe (Intel Arc/Xe2)".into()]),
            upstream: Some("https://github.com/waydroid/waydroid"),
            expected_renderer: "… (radeonsi …) / Mesa Intel …, OpenGL ES 3.2 Mesa …",
        },
        GpuBackendKind::NvidiaAccelerated => BackendDescriptor {
            kind,
            name: "NVIDIA Accelerated".into(),
            summary: tr!(
                "Vulkan di Android inoltrato via Mesa Venus a un render server host sul driver NVIDIA; GL ES tramite ANGLE.",
                "Android's Vulkan is forwarded through Mesa Venus to a host render server on the NVIDIA driver; GL ES via ANGLE."
            ),
            stability: Stability::Stable,
            requirements: v(&[
                tr!("GPU NVIDIA Turing (RTX 20 / GTX 16) o successiva", "NVIDIA Turing (RTX 20 / GTX 16) or newer GPU"),
                tr!("Modulo kernel NVIDIA open (DMA-BUF)", "Open NVIDIA kernel module (DMA-BUF)"),
                tr!("Driver ≥ 595.71 con nvidia-drm.modeset=1", "Driver ≥ 595.71 with nvidia-drm.modeset=1"),
                tr!("Compositor Wayland in esecuzione sulla GPU NVIDIA", "Wayland compositor running on the NVIDIA GPU"),
                tr!("Accesso a /dev/udmabuf", "Access to /dev/udmabuf"),
                tr!("waydroid-nvidia installato in modalità venus", "waydroid-nvidia installed in venus mode"),
            ]),
            compatible_drivers: v(&[tr!("nvidia-open ≥ 595.71 (consigliato 610+)", "nvidia-open ≥ 595.71 (610+ recommended)")]),
            upstream: Some("https://github.com/Shiro836/waydroid-nvidia"),
            expected_renderer: "ANGLE (NVIDIA, Vulkan … (Virtio-GPU Venus (NVIDIA GeForce …)))",
        },
        GpuBackendKind::NvidiaExperimental => BackendDescriptor {
            kind,
            name: "NVIDIA Experimental".into(),
            summary: tr!(
                "Driver open-source (NVK + backend NVIDIA) eseguito direttamente dentro Android, senza proxy; decodifica video HW.",
                "Open-source driver (NVK + NVIDIA backend) running directly inside Android, no proxy; hardware video decode."
            ),
            stability: Stability::Experimental,
            requirements: v(&[
                tr!("GPU NVIDIA Maxwell 2 o successiva (Waydroid confermato su RTX 50)", "NVIDIA Maxwell 2 or newer GPU (Waydroid confirmed on RTX 50)"),
                tr!("Driver 550–615 (modulo open o chiuso; 550.107 rifiutato)", "Driver 550–615 (open or closed module; 550.107 refused)"),
                "nvidia-drm.modeset=1".into(),
                tr!("Compositor Wayland sulla GPU NVIDIA", "Wayland compositor on the NVIDIA GPU"),
                tr!("Immagine Android 13 ufficiale (LineageOS 20)", "Official Android 13 image (LineageOS 20)"),
                tr!("waydroid-nvidia installato in modalità nvidia-prolapse", "waydroid-nvidia installed in nvidia-prolapse mode"),
            ]),
            compatible_drivers: v(&["nvidia 615 open (✅ upstream)".into(), "nvidia 580–610 open (🧪)".into(), "nvidia 550–610 closed (🧪)".into()]),
            upstream: Some("https://github.com/Shiro836/waydroid-nvidia/tree/public-test"),
            expected_renderer: "ANGLE (NVIDIA, Vulkan … (NVIDIA GeForce … (NVK …)))",
        },
        GpuBackendKind::SoftwareFallback => BackendDescriptor {
            kind,
            name: "Software Fallback".into(),
            summary: tr!(
                "Rendering CPU con SwiftShader. Sempre disponibile, mai considerato accelerato.",
                "CPU rendering with SwiftShader. Always available, never reported as accelerated."
            ),
            stability: Stability::Fallback,
            requirements: v(&[tr!("Nessun requisito GPU", "No GPU requirement")]),
            compatible_drivers: v(&[tr!("qualsiasi", "any")]),
            upstream: None,
            expected_renderer: "Google SwiftShader",
        },
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Readiness {
    /// Hardware and software in place; acceleration still needs runtime verification.
    Ready,
    /// Hardware compatible, components to install or configure.
    NeedsSetup,
    /// A hardware or driver requirement is not met.
    Unsupported,
}

#[derive(Debug, Clone, Serialize)]
pub struct BackendAssessment {
    pub descriptor: BackendDescriptor,
    pub readiness: Readiness,
    /// Device the backend would render on.
    pub target_device: Option<String>,
    pub target_render_node: Option<String>,
    pub checks: Vec<Check>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GpuPlan {
    pub assessments: Vec<BackendAssessment>,
    pub recommended: GpuBackendKind,
    pub reason: String,
}

/// NVIDIA architecture from the PCI device id (approximate ranges, name used as tie-breaker).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NvidiaGeneration {
    PreMaxwell,
    Maxwell,
    Pascal,
    Volta,
    Turing,
    Ampere,
    Ada,
    Blackwell,
    Unknown,
}

pub fn nvidia_generation(dev: &GpuDevice) -> NvidiaGeneration {
    let name = dev.name.clone().unwrap_or_default().to_ascii_uppercase();
    for (marker, generation) in [
        ("RTX 50", NvidiaGeneration::Blackwell),
        ("RTX 40", NvidiaGeneration::Ada),
        ("RTX 30", NvidiaGeneration::Ampere),
        ("RTX 20", NvidiaGeneration::Turing),
        ("GTX 16", NvidiaGeneration::Turing),
        ("GTX 10", NvidiaGeneration::Pascal),
        ("TITAN V", NvidiaGeneration::Volta),
        ("GTX 9", NvidiaGeneration::Maxwell),
    ] {
        if name.contains(marker) {
            return generation;
        }
    }
    match dev.device_id_u16() {
        Some(0x2b00..=0x2fff) => NvidiaGeneration::Blackwell,
        Some(0x2680..=0x28ff) => NvidiaGeneration::Ada,
        Some(0x2200..=0x25ff) => NvidiaGeneration::Ampere,
        Some(0x1e00..=0x1fff) | Some(0x2180..=0x21ff) => NvidiaGeneration::Turing,
        Some(0x1d81) => NvidiaGeneration::Volta,
        Some(0x1500..=0x1dff) => NvidiaGeneration::Pascal,
        Some(0x1340..=0x14ff) => NvidiaGeneration::Maxwell,
        Some(0x0000..=0x133f) => NvidiaGeneration::PreMaxwell,
        _ => NvidiaGeneration::Unknown,
    }
}

fn readiness(checks: &[Check]) -> Readiness {
    if checks.iter().any(|c| c.status == CheckStatus::Fail) {
        Readiness::Unsupported
    } else if checks.iter().any(|c| c.status == CheckStatus::Missing) {
        Readiness::NeedsSetup
    } else {
        Readiness::Ready
    }
}

fn common_runtime_checks(r: &HostReport, checks: &mut Vec<Check>) {
    let binder = tr!("Binder nel kernel", "Kernel binder");
    if r.kernel.binder.usable() {
        checks.push(Check::pass("binder", &binder, tr!("disponibile", "available")));
    } else {
        checks.push(
            Check::fail("binder", &binder, tr!("CONFIG_ANDROID_BINDERFS assente", "CONFIG_ANDROID_BINDERFS missing"))
                .remedy(tr!("Usa un kernel con binder (es. linux-generic Ubuntu o linux-image Debian ≥ 6.x)", "Use a kernel with binder (e.g. Ubuntu linux-generic or Debian linux-image ≥ 6.x)")),
        );
    }
    if r.waydroid.installed {
        checks.push(Check::pass("waydroid", "Waydroid", r.waydroid.version.clone().unwrap_or_else(|| tr!("installato", "installed"))));
    } else {
        checks.push(
            Check::missing("waydroid", "Waydroid", tr!("non installato", "not installed"))
                .remedy(tr!("Installazione guidata da DroidNative (repository ufficiale repo.waydro.id)", "Guided setup in DroidNative (official repo.waydro.id repository)")),
        );
    }
}

fn session_check(r: &HostReport, required: bool) -> Check {
    let label = tr!("Sessione Wayland", "Wayland session");
    match r.session.kind {
        SessionKind::Wayland => Check::pass("wayland", &label, r.session.compositor.clone().unwrap_or_else(|| tr!("attiva", "active"))),
        SessionKind::X11 if !required => Check::warn(
            "wayland",
            &label,
            tr!("sessione X11: serve un compositor Wayland annidato (weston/cage)", "X11 session: a nested Wayland compositor (weston/cage) is needed"),
        ),
        _ => Check::fail("wayland", &label, tr!("richiesta una sessione Wayland", "a Wayland session is required"))
            .remedy(tr!("Accedi con una sessione Wayland (es. «GNOME» invece di «GNOME su Xorg»)", "Log in with a Wayland session (e.g. “GNOME” instead of “GNOME on Xorg”)")),
    }
}

fn nvidia_display_check(r: &HostReport) -> Check {
    let label = tr!("Compositor sulla GPU NVIDIA", "Compositor on the NVIDIA GPU");
    match r.gpu.display_gpu() {
        Some(d) if d.vendor == GpuVendor::Nvidia => {
            let (name, outs) = (d.display_name(), d.connected_outputs.join(", "));
            Check::pass("compositor_gpu", &label, tr!("schermo su {name} ({outs})", "display on {name} ({outs})"))
        }
        Some(d) => {
            let name = d.display_name();
            Check::fail(
                "compositor_gpu",
                &label,
                tr!(
                    "lo schermo è collegato a {name}: il compositor non può mostrare i buffer NVIDIA block-linear",
                    "the display is connected to {name}: the compositor cannot show NVIDIA block-linear buffers"
                ),
            )
            .remedy(tr!(
                "Collega il monitor alla scheda NVIDIA, oppure esegui Waydroid in gamescope sulla GPU NVIDIA (non funziona su portatili ibridi Intel+NVIDIA)",
                "Connect the monitor to the NVIDIA card, or run Waydroid in gamescope on the NVIDIA GPU (does not work on hybrid Intel+NVIDIA laptops)"
            ))
        }
        None => Check::unknown("compositor_gpu", &label, tr!("nessuna uscita video collegata rilevata", "no connected video output detected")),
    }
}

fn assess_mesa(r: &HostReport) -> BackendAssessment {
    let mut checks = Vec::new();
    let display = r.gpu.display_gpu();
    // Prefer the Mesa GPU that drives the display; otherwise the first Mesa-capable one.
    let target = r.gpu.mesa_devices().find(|d| d.drives_display()).or_else(|| r.gpu.mesa_devices().next());
    let mesa_gpu = tr!("GPU compatibile Mesa", "Mesa-compatible GPU");
    let same_gpu = tr!("Compositor sulla stessa GPU", "Compositor on the same GPU");

    match target {
        Some(dev) => {
            checks.push(Check::pass(
                "mesa_gpu",
                &mesa_gpu,
                format!("{} ({}) → {}", dev.display_name(), dev.kernel_driver.as_deref().unwrap_or("?"), dev.render_node.as_deref().unwrap_or("?")),
            ));
            let access = tr!("Accesso al nodo render", "Render node access");
            if dev.render_node_accessible {
                checks.push(Check::pass("render_access", &access, tr!("consentito all'utente", "allowed for the user")));
            } else {
                checks.push(Check::warn("render_access", &access, tr!("l'utente non può aprire il nodo (gruppo render/ACL)", "the user cannot open the node (render group/ACL)")));
            }
            match display {
                Some(d) if d.pci_slot == dev.pci_slot => {
                    checks.push(Check::pass("same_gpu", &same_gpu, tr!("rendering e composizione sulla stessa GPU", "rendering and compositing on the same GPU")))
                }
                Some(d) if d.vendor == GpuVendor::Nvidia => {
                    let (disp, rend) = (d.display_name(), dev.display_name());
                    checks.push(
                        Check::fail(
                            "same_gpu",
                            &same_gpu,
                            tr!(
                                "lo schermo è su {disp}: il compositor non riesce a importare i buffer di {rend} (il driver NVIDIA importa dmabuf LINEAR di altri vendor solo come GL_TEXTURE_EXTERNAL_OES). Verificato su GNOME/Mutter: «failed to import supplied dmabufs: Could not bind the given EGLImage to a CoglTexture2D», poi crash dell'hwcomposer appena Android apre una finestra.",
                                "the display is on {disp}: the compositor cannot import {rend}'s buffers (the NVIDIA driver imports other vendors' LINEAR dmabufs only as GL_TEXTURE_EXTERNAL_OES). Seen on GNOME/Mutter: “failed to import supplied dmabufs: Could not bind the given EGLImage to a CoglTexture2D”, then the hwcomposer crashes as soon as Android opens a window."
                            ),
                        )
                        .remedy(tr!(
                            "Collega il monitor alla GPU AMD/Intel, oppure usa un backend NVIDIA o il fallback software",
                            "Connect the monitor to the AMD/Intel GPU, or use an NVIDIA backend or the software fallback"
                        )),
                    )
                }
                Some(d) => {
                    let (disp, rend) = (d.display_name(), dev.display_name());
                    checks.push(Check::info("same_gpu", &same_gpu, tr!("PRIME tra {disp} e {rend}: generalmente supportato da Mesa", "PRIME between {disp} and {rend}: generally supported by Mesa")))
                }
                None => checks.push(Check::unknown("same_gpu", &same_gpu, tr!("GPU del display non determinata", "display GPU not determined"))),
            }
        }
        None => checks.push(
            Check::fail("mesa_gpu", &mesa_gpu, tr!("nessuna GPU AMD/Intel con nodo render", "no AMD/Intel GPU with a render node"))
                .remedy(tr!("Usa un backend NVIDIA o il fallback software", "Use an NVIDIA backend or the software fallback")),
        ),
    }
    checks.push(session_check(r, false));
    common_runtime_checks(r, &mut checks);
    if let Some(cfg) = r.waydroid.config.as_ref() {
        let forced_sw = cfg.properties.iter().any(|(k, v)| k == "ro.hardware.egl" && v == "swiftshader");
        if forced_sw {
            checks.push(
                Check::missing("cfg_override", tr!("Configurazione Waydroid", "Waydroid configuration"), tr!("ro.hardware.egl=swiftshader forza il rendering software", "ro.hardware.egl=swiftshader forces software rendering"))
                    .remedy(tr!("Seleziona Mesa Native in DroidNative per rimuovere l'override", "Select Mesa Native in DroidNative to remove the override")),
            );
        }
        let node = tr!("Nodo GPU configurato", "Configured GPU node");
        // With waydroid-nvidia, drm_device points at the NVIDIA node by design: not a Mesa setting.
        let drm = if r.waydroid.nvidia_setup.is_some() { None } else { cfg.drm_device.as_ref() };
        match (drm, target.and_then(|t| t.render_node.as_ref())) {
            (Some(cur), Some(want)) if cur == want => checks.push(Check::pass("drm_device", &node, cur)),
            (Some(cur), Some(want)) => checks.push(Check::missing("drm_device", &node, tr!("{cur} invece di {want}", "{cur} instead of {want}"))),
            (None, Some(want)) if r.waydroid.nvidia_setup.is_none() => checks.push(Check::info(
                "drm_device",
                &node,
                tr!("automatico (Waydroid sceglierà il primo nodo non NVIDIA: {want})", "automatic (Waydroid will pick the first non-NVIDIA node: {want})"),
            )),
            _ => {}
        }
    }
    BackendAssessment {
        descriptor: descriptor(GpuBackendKind::MesaNative),
        readiness: readiness(&checks),
        target_device: target.map(|d| d.display_name()),
        target_render_node: target.and_then(|d| d.render_node.clone()),
        checks,
    }
}

fn nvidia_driver_checks(r: &HostReport, checks: &mut Vec<Check>, experimental: bool) {
    let drv_label = tr!("Driver NVIDIA", "NVIDIA driver");
    let Some(drv) = r.gpu.nvidia.as_ref() else {
        checks.push(
            Check::fail("nvidia_driver", &drv_label, tr!("driver proprietario NVIDIA non caricato (nouveau?)", "proprietary NVIDIA driver not loaded (nouveau?)"))
                .remedy(tr!("Installa nvidia-driver-<versione>-open", "Install nvidia-driver-<version>-open")),
        );
        return;
    };
    let version = drv.version.clone().unwrap_or_else(|| "?".into());
    let tuple = drv.version_tuple();
    let flavor = drv.module_flavor;

    if experimental {
        let module = if flavor == NvidiaModuleFlavor::Open { tr!("modulo open", "open module") } else { tr!("modulo chiuso", "closed module") };
        let detail = format!("{version} ({module})");
        match (tuple, flavor) {
            (Some((major, _)), NvidiaModuleFlavor::Open) if major >= 615 => checks.push(Check::pass("nvidia_driver", &drv_label, format!("{detail}: ✅ upstream"))),
            (Some((550, 107)), _) => checks.push(Check::fail("nvidia_driver", &drv_label, tr!("{detail}: versione rifiutata da upstream", "{detail}: version refused upstream"))),
            // Upstream table: open 580–610 and closed 550–610 are 🧪; newer closed builds are untested but accepted.
            (Some((major, _)), _) if major >= 550 => checks.push(Check::warn(
                "nvidia_driver",
                &drv_label,
                tr!("{detail}: 🧪 test driver superati, non provato in Waydroid", "{detail}: 🧪 driver tests pass, not tried in Waydroid"),
            )),
            _ => checks.push(Check::fail("nvidia_driver", &drv_label, tr!("{detail}: non supportato (serve ≥ 550)", "{detail}: not supported (≥ 550 needed)"))),
        }
    } else {
        let open = tr!("Modulo kernel open", "Open kernel module");
        if flavor == NvidiaModuleFlavor::Open {
            checks.push(Check::pass("nvidia_module", &open, tr!("open-gpu-kernel-modules (DMA-BUF disponibile)", "open-gpu-kernel-modules (DMA-BUF available)")));
        } else {
            checks.push(
                Check::fail("nvidia_module", &open, tr!("modulo chiuso: nessun export DMA-BUF", "closed module: no DMA-BUF export"))
                    .remedy(tr!("Installa la variante -open del driver (es. nvidia-driver-615-open)", "Install the -open variant of the driver (e.g. nvidia-driver-615-open)")),
            );
        }
        match tuple {
            Some(t) if t >= (595, 71) => checks.push(Check::pass("nvidia_driver", "Driver ≥ 595.71", &version)),
            Some(_) => checks.push(
                Check::fail("nvidia_driver", "Driver ≥ 595.71", tr!("{version} troppo vecchio", "{version} is too old"))
                    .remedy(tr!("Aggiorna al driver 610 o successivo", "Update to driver 610 or newer")),
            ),
            None => checks.push(Check::unknown("nvidia_driver", "Driver ≥ 595.71", tr!("versione non leggibile", "version not readable"))),
        }
    }
    match drv.modeset {
        Some(true) => checks.push(Check::pass("modeset", "nvidia-drm.modeset=1", tr!("attivo", "on"))),
        Some(false) => checks.push(
            Check::fail("modeset", "nvidia-drm.modeset=1", tr!("disattivato: niente DMA-BUF né sync_fd", "off: no DMA-BUF and no sync_fd"))
                .remedy(tr!("Aggiungi nvidia-drm.modeset=1 ai parametri del kernel e riavvia", "Add nvidia-drm.modeset=1 to the kernel parameters and reboot")),
        ),
        None => checks.push(Check::unknown("modeset", "nvidia-drm.modeset=1", tr!("parametro non leggibile", "parameter not readable"))),
    }
}

fn assess_nvidia(r: &HostReport, experimental: bool) -> BackendAssessment {
    let kind = if experimental { GpuBackendKind::NvidiaExperimental } else { GpuBackendKind::NvidiaAccelerated };
    let mut checks = Vec::new();
    let dev = r.gpu.nvidia_device();
    let gpu_label = tr!("GPU NVIDIA supportata", "Supported NVIDIA GPU");
    match dev {
        Some(d) => {
            let generation = nvidia_generation(d);
            let name = d.display_name();
            let gen_label = tr!("{name} — architettura {generation:?}", "{name} — {generation:?} architecture");
            let min = if experimental { NvidiaGeneration::Maxwell } else { NvidiaGeneration::Turing };
            if generation == NvidiaGeneration::Unknown {
                checks.push(Check::unknown("nvidia_gpu", &gpu_label, &gen_label));
            } else if generation < min {
                checks.push(Check::fail("nvidia_gpu", &gpu_label, tr!("{gen_label}: troppo vecchia", "{gen_label}: too old")));
            } else if experimental && generation != NvidiaGeneration::Blackwell {
                checks.push(Check::warn(
                    "nvidia_gpu",
                    &gpu_label,
                    tr!("{gen_label}: 🧪 in upstream (Waydroid confermato solo su RTX 50)", "{gen_label}: 🧪 upstream (Waydroid confirmed on RTX 50 only)"),
                ));
            } else {
                checks.push(Check::pass("nvidia_gpu", &gpu_label, &gen_label));
            }
            if !experimental && generation == NvidiaGeneration::Ada {
                checks.push(Check::warn(
                    "ada_bug",
                    tr!("Bug noto del driver su Ada", "Known driver bug on Ada"),
                    tr!(
                        "redroid-nvidia ha riprodotto su RTX 4060 un SIGABRT di SurfaceFlinger (vn_ring_submit_locked) causato da un'asserzione nel driver kernel NVIDIA, aperto upstream",
                        "redroid-nvidia reproduced a SurfaceFlinger SIGABRT (vn_ring_submit_locked) on an RTX 4060, caused by an NVIDIA kernel driver assertion; open upstream"
                    ),
                ));
            }
            nvidia_driver_checks(r, &mut checks, experimental);
        }
        None => checks.push(Check::fail("nvidia_gpu", &gpu_label, tr!("nessuna GPU NVIDIA rilevata", "no NVIDIA GPU detected"))),
    }
    if dev.is_some() {
        checks.push(nvidia_display_check(r));
        checks.push(session_check(r, true));
        let tested = tr!("Compositor testato upstream", "Compositor tested upstream");
        match r.session.compositor.as_deref() {
            Some("KWin") => checks.push(Check::pass("compositor_tested", &tested, "KWin / Plasma 6")),
            Some(other) => checks.push(Check::warn("compositor_tested", &tested, tr!("{other}: upstream testa solo KWin / Plasma 6", "{other}: upstream tests KWin / Plasma 6 only"))),
            None => checks.push(Check::unknown("compositor_tested", &tested, tr!("compositor non riconosciuto", "compositor not recognized"))),
        }
        if !experimental {
            let u = &r.gpu.udmabuf;
            let label = tr!("Accesso a /dev/udmabuf", "Access to /dev/udmabuf");
            match (u.exists, u.accessible) {
                (true, true) => checks.push(Check::pass("udmabuf", &label, tr!("consentito", "allowed"))),
                (true, false) => checks.push(
                    Check::warn("udmabuf", &label, tr!("presente ma non accessibile: cursore invisibile e screenshot neri", "present but not accessible: invisible cursor and black screenshots"))
                        .remedy(tr!("La regola udev uaccess di waydroid-nvidia lo concede al prossimo login", "waydroid-nvidia's uaccess udev rule grants it at the next login")),
                ),
                (false, _) => checks.push(Check::warn("udmabuf", &label, tr!("modulo udmabuf non caricato", "udmabuf module not loaded")).remedy("sudo modprobe udmabuf")),
            }
        }
        if experimental {
            checks.push(Check::info(
                "android_version",
                tr!("Versione Android", "Android version"),
                tr!("solo immagine ufficiale Android 13 (LineageOS 20)", "official Android 13 image only (LineageOS 20)"),
            ));
        }
    }
    common_runtime_checks(r, &mut checks);
    if dev.is_some() {
        let want = if experimental { NvidiaMode::Prolapse } else { NvidiaMode::Venus };
        let tag = r.waydroid.nvidia_stack.as_ref().map(|s| s.tag.clone()).unwrap_or_else(|| tr!("versione non registrata", "version not recorded"));
        let want_arg = want.as_arg();
        match (&r.waydroid.nvidia_setup, r.waydroid.nvidia_mode) {
            (Some(_), Some(m)) if m == want => {
                let mode = m.as_arg();
                checks.push(Check::pass("waydroid_nvidia", "waydroid-nvidia", tr!("{tag}, modalità {mode} attiva", "{tag}, {mode} mode active")))
            }
            (Some(_), current) => {
                let mode = current.map(|m| m.as_arg().to_string()).unwrap_or_else(|| tr!("nessuna", "none"));
                checks.push(
                    Check::missing("waydroid_nvidia", "waydroid-nvidia", tr!("{tag} installato, modalità attiva: {mode}", "{tag} installed, active mode: {mode}"))
                        .remedy(tr!("Seleziona questo backend: DroidNative esegue waydroid-nvidia-setup --mode {want_arg}", "Select this backend: DroidNative runs waydroid-nvidia-setup --mode {want_arg}")),
                )
            }
            (None, _) => {
                let rel = crate::nvidia_stack::latest_release().tag;
                checks.push(
                    Check::missing("waydroid_nvidia", "waydroid-nvidia", tr!("non installato", "not installed")).remedy(tr!(
                        "Installazione guidata del rilascio {rel} (hash e commit fissati in DroidNative, attestazioni di build verificate)",
                        "Guided installation of release {rel} (hashes and commit pinned in DroidNative, build attestations checked)"
                    )),
                )
            }
        }
        if r.waydroid.distro_package.as_deref().is_some_and(|s| s.contains("ok installed")) && r.waydroid.nvidia_setup.is_none() {
            checks.push(Check::warn(
                "runtime_replacement",
                tr!("Sostituzione del runtime", "Runtime replacement"),
                tr!(
                    "waydroid-nvidia installa una Waydroid patchata e non sovrascrive i file di un pacchetto: il pacchetto waydroid attuale va rimosso prima (immagini e dati restano in /var/lib/waydroid e ~/.local/share/waydroid)",
                    "waydroid-nvidia installs a patched Waydroid and won't overwrite packaged files: the current waydroid package is removed first (images and data stay in /var/lib/waydroid and ~/.local/share/waydroid)"
                ),
            ));
        }
        if !experimental && r.waydroid.nvidia_mode == Some(NvidiaMode::Venus) && !r.waydroid.venus_service.is_active() {
            checks.push(Check::info(
                "venus_service",
                tr!("Render server Venus (wd-venus)", "Venus render server (wd-venus)"),
                tr!("non attivo: DroidNative lo avvia insieme alla sessione Android", "not running: DroidNative starts it with the Android session"),
            ));
        }
    }
    BackendAssessment {
        descriptor: descriptor(kind),
        readiness: readiness(&checks),
        target_device: dev.map(|d| d.display_name()),
        target_render_node: dev.and_then(|d| d.render_node.clone()),
        checks,
    }
}

fn assess_software(r: &HostReport) -> BackendAssessment {
    let mut checks = vec![Check::warn(
        "performance",
        tr!("Prestazioni", "Performance"),
        tr!("rendering su CPU: giochi e app 3D lenti, nessuna accelerazione", "CPU rendering: games and 3D apps are slow, no acceleration"),
    )];
    checks.push(session_check(r, false));
    common_runtime_checks(r, &mut checks);
    BackendAssessment {
        descriptor: descriptor(GpuBackendKind::SoftwareFallback),
        readiness: readiness(&checks),
        target_device: Some("CPU (SwiftShader)".into()),
        target_render_node: None,
        checks,
    }
}

pub fn assess_backends(r: &HostReport) -> GpuPlan {
    let assessments = vec![assess_mesa(r), assess_nvidia(r, false), assess_nvidia(r, true), assess_software(r)];
    let get = |k: GpuBackendKind| assessments.iter().find(|a| a.descriptor.kind == k).unwrap();
    let ok = |k: GpuBackendKind| get(k).readiness != Readiness::Unsupported;
    let warn_count = |k: GpuBackendKind| get(k).checks.iter().filter(|c| c.status == CheckStatus::Warn).count();
    let display_vendor = r.gpu.display_gpu().map(|d| d.vendor);

    let (recommended, reason) = if display_vendor == Some(GpuVendor::Nvidia) && ok(GpuBackendKind::NvidiaAccelerated) {
        (
            GpuBackendKind::NvidiaAccelerated,
            tr!(
                "Il display è sulla GPU NVIDIA e i requisiti di Venus sono soddisfatti: è il percorso hardware più collaudato per questa configurazione.",
                "The display is on the NVIDIA GPU and Venus' requirements are met: it's the most tested hardware path for this setup."
            ),
        )
    } else if ok(GpuBackendKind::MesaNative) && (display_vendor != Some(GpuVendor::Nvidia) || warn_count(GpuBackendKind::MesaNative) == 0) {
        (GpuBackendKind::MesaNative, tr!("GPU AMD/Intel che pilota il display: Mesa è il percorso nativo senza proxy.", "AMD/Intel GPU driving the display: Mesa is the native path, no proxy."))
    } else if ok(GpuBackendKind::NvidiaExperimental) {
        (GpuBackendKind::NvidiaExperimental, tr!("Unico percorso hardware NVIDIA compatibile, ma sperimentale.", "The only compatible NVIDIA hardware path, but experimental."))
    } else if ok(GpuBackendKind::MesaNative) {
        (GpuBackendKind::MesaNative, tr!("Mesa sull'iGPU con import cross-GPU da verificare.", "Mesa on the iGPU, cross-GPU import to be verified."))
    } else {
        (GpuBackendKind::SoftwareFallback, tr!("Nessun percorso hardware compatibile: solo rendering software.", "No compatible hardware path: software rendering only."))
    };
    GpuPlan { assessments, recommended, reason }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::{CmdOutput, FakeRunner};
    use crate::fs::HostFs;
    use crate::host::{probe_host, Env, Probe};

    fn hybrid_report(open_module: bool, modeset: &str, desktop: &str) -> HostReport {
        let dir = tempfile::tempdir().unwrap();
        crate::gpu::fixtures::hybrid_nvidia_display(dir.path());
        let w = |p: &str, c: &str| {
            let p = dir.path().join(p.trim_start_matches('/'));
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, c).unwrap();
        };
        let module = if open_module { "Open Kernel Module for x86_64" } else { "x86_64 Kernel Module" };
        w("/proc/driver/nvidia/version", &format!("NVRM version: NVIDIA UNIX {module}  615.71.09  Release Build\n"));
        w("/sys/module/nvidia_drm/parameters/modeset", modeset);
        w("/proc/sys/kernel/osrelease", "7.0.0-38-generic\n");
        w("/boot/config-7.0.0-38-generic", "CONFIG_ANDROID_BINDER_IPC=m\nCONFIG_ANDROID_BINDERFS=m\n");
        w("/lib/modules/7.0.0-38-generic/modules.dep", "kernel/drivers/android/binder_linux.ko.zst:\n");
        w("/dev/udmabuf", "");
        w("/run/user/1000/wayland-0", "");
        let runner = FakeRunner::new()
            .with_program("lspci")
            .respond("lspci -mm -nn -s 0000:01:00.0", CmdOutput::ok("01:00.0 \"VGA\" \"NVIDIA Corporation [10de]\" \"AD103 [GeForce RTX 4070 Ti SUPER] [2705]\"\n"))
            .respond("lspci -mm -nn -s 0000:09:00.0", CmdOutput::ok("09:00.0 \"VGA\" \"Advanced Micro Devices, Inc. [AMD/ATI] [1002]\" \"Raphael [164e]\"\n"));
        let env = Env::from_pairs(&[
            ("XDG_SESSION_TYPE", "wayland"),
            ("WAYLAND_DISPLAY", "wayland-0"),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
            ("XDG_CURRENT_DESKTOP", desktop),
            ("HOME", "/home/u"),
        ]);
        let fs = HostFs::rooted(dir.path());
        probe_host(&Probe { fs: &fs, cmd: &runner, env: &env })
    }

    fn find(plan: &GpuPlan, k: GpuBackendKind) -> &BackendAssessment {
        plan.assessments.iter().find(|a| a.descriptor.kind == k).unwrap()
    }

    #[test]
    fn development_machine_layout() {
        let r = hybrid_report(true, "Y", "ubuntu:GNOME");
        let plan = assess_backends(&r);
        let nv = find(&plan, GpuBackendKind::NvidiaAccelerated);
        assert_eq!(nv.readiness, Readiness::NeedsSetup, "{:#?}", nv.checks);
        assert!(nv.checks.iter().any(|c| c.id == "ada_bug" && c.status == CheckStatus::Warn));
        assert!(nv.checks.iter().any(|c| c.id == "compositor_tested" && c.status == CheckStatus::Warn));

        let mesa = find(&plan, GpuBackendKind::MesaNative);
        assert_eq!(mesa.target_render_node.as_deref(), Some("/dev/dri/renderD128"));
        assert!(mesa.checks.iter().any(|c| c.id == "same_gpu" && c.status == CheckStatus::Fail));
        assert_eq!(mesa.readiness, Readiness::Unsupported);

        assert_eq!(plan.recommended, GpuBackendKind::NvidiaAccelerated);
        assert_eq!(find(&plan, GpuBackendKind::SoftwareFallback).readiness, Readiness::NeedsSetup);
    }

    #[test]
    fn closed_module_blocks_venus_but_not_experimental() {
        let r = hybrid_report(false, "Y", "KDE");
        let plan = assess_backends(&r);
        assert_eq!(find(&plan, GpuBackendKind::NvidiaAccelerated).readiness, Readiness::Unsupported);
        assert_eq!(find(&plan, GpuBackendKind::NvidiaExperimental).readiness, Readiness::NeedsSetup);
        assert_eq!(plan.recommended, GpuBackendKind::NvidiaExperimental);
    }

    #[test]
    fn modeset_off_blocks_all_nvidia_paths() {
        let r = hybrid_report(true, "N", "KDE");
        let plan = assess_backends(&r);
        assert_eq!(find(&plan, GpuBackendKind::NvidiaAccelerated).readiness, Readiness::Unsupported);
        assert_eq!(find(&plan, GpuBackendKind::NvidiaExperimental).readiness, Readiness::Unsupported);
        // The monitor is on the NVIDIA card, so the AMD iGPU's buffers cannot be displayed either:
        // only software rendering remains, and it must not be presented as acceleration.
        assert_eq!(find(&plan, GpuBackendKind::MesaNative).readiness, Readiness::Unsupported);
        assert_eq!(plan.recommended, GpuBackendKind::SoftwareFallback);
    }

    #[test]
    fn generation_from_ids() {
        let mk = |id: &str, name: Option<&str>| GpuDevice {
            pci_slot: None,
            vendor: GpuVendor::Nvidia,
            vendor_id: "0x10de".into(),
            device_id: id.into(),
            name: name.map(str::to_string),
            kernel_driver: Some("nvidia".into()),
            render_node: None,
            card_node: None,
            render_node_accessible: false,
            connected_outputs: vec![],
            boot_vga: false,
            vram_bytes: None,
        };
        assert_eq!(nvidia_generation(&mk("0x2705", None)), NvidiaGeneration::Ada);
        assert_eq!(nvidia_generation(&mk("0x2204", None)), NvidiaGeneration::Ampere);
        assert_eq!(nvidia_generation(&mk("0x1e84", None)), NvidiaGeneration::Turing);
        assert_eq!(nvidia_generation(&mk("0x1b80", None)), NvidiaGeneration::Pascal);
        assert_eq!(nvidia_generation(&mk("0x0000", Some("NVIDIA GeForce RTX 5090"))), NvidiaGeneration::Blackwell);
    }
}
