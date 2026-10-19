//! The four selectable GPU backends, what each requires, and whether this host meets it.

use serde::{Deserialize, Serialize};

use super::{GpuDevice, GpuVendor, NvidiaModuleFlavor};
use crate::host::{HostReport, SessionKind};
use crate::nvidia_stack::NvidiaMode;
use crate::report::{Check, CheckStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuBackendKind {
    MesaNative,
    NvidiaAccelerated,
    NvidiaExperimental,
    SoftwareFallback,
}

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
    pub name: &'static str,
    pub summary: &'static str,
    pub stability: Stability,
    pub requirements: &'static [&'static str],
    pub compatible_drivers: &'static [&'static str],
    pub upstream: Option<&'static str>,
    /// What the SurfaceFlinger `GLES:` line looks like when this backend works.
    pub expected_renderer: &'static str,
}

pub const DESCRIPTORS: &[BackendDescriptor] = &[
    BackendDescriptor {
        kind: GpuBackendKind::MesaNative,
        name: "Mesa Native",
        summary: "Android usa direttamente il nodo render della GPU tramite Mesa (gralloc gbm + EGL mesa).",
        stability: Stability::Stable,
        requirements: &[
            "GPU AMD o Intel con driver kernel amdgpu/radeon/i915/xe",
            "Nodo /dev/dri/renderD* accessibile",
            "Compositor sulla stessa GPU, oppure import dmabuf cross-GPU funzionante",
            "Sessione Wayland",
        ],
        compatible_drivers: &["amdgpu (RadeonSI, RADV)", "radeon", "i915 (Iris, ANV)", "xe (Intel Arc/Xe2)"],
        upstream: Some("https://github.com/waydroid/waydroid"),
        expected_renderer: "… (radeonsi …) / Mesa Intel …, OpenGL ES 3.2 Mesa …",
    },
    BackendDescriptor {
        kind: GpuBackendKind::NvidiaAccelerated,
        name: "NVIDIA Accelerated",
        summary: "Vulkan di Android inoltrato via Mesa Venus a un render server host sul driver NVIDIA; GL ES tramite ANGLE.",
        stability: Stability::Stable,
        requirements: &[
            "GPU NVIDIA Turing (RTX 20 / GTX 16) o successiva",
            "Modulo kernel NVIDIA open (DMA-BUF)",
            "Driver ≥ 595.71 con nvidia-drm.modeset=1",
            "Compositor Wayland in esecuzione sulla GPU NVIDIA",
            "Accesso a /dev/udmabuf",
            "waydroid-nvidia (ramo main) installato",
        ],
        compatible_drivers: &["nvidia-open ≥ 595.71 (consigliato 610+)"],
        upstream: Some("https://github.com/Shiro836/waydroid-nvidia"),
        expected_renderer: "ANGLE (NVIDIA, Vulkan … (Virtio-GPU Venus (NVIDIA GeForce …)))",
    },
    BackendDescriptor {
        kind: GpuBackendKind::NvidiaExperimental,
        name: "NVIDIA Experimental",
        summary: "Driver open-source (NVK + backend NVIDIA) eseguito direttamente dentro Android, senza proxy; decodifica video HW.",
        stability: Stability::Experimental,
        requirements: &[
            "GPU NVIDIA Maxwell 2 o successiva (Waydroid confermato su RTX 50)",
            "Driver 550–615 (modulo open o chiuso; 550.107 rifiutato)",
            "nvidia-drm.modeset=1",
            "Compositor Wayland sulla GPU NVIDIA",
            "Immagine Android 13 ufficiale (LineageOS 20)",
            "waydroid-nvidia ramo public-test installato",
        ],
        compatible_drivers: &["nvidia 615 open (✅ upstream)", "nvidia 580–610 open (🧪)", "nvidia 550–610 closed (🧪)"],
        upstream: Some("https://github.com/Shiro836/waydroid-nvidia/tree/public-test"),
        expected_renderer: "ANGLE (NVIDIA, Vulkan … (NVIDIA GeForce …))",
    },
    BackendDescriptor {
        kind: GpuBackendKind::SoftwareFallback,
        name: "Software Fallback",
        summary: "Rendering CPU con SwiftShader. Sempre disponibile, mai considerato accelerato.",
        stability: Stability::Fallback,
        requirements: &["Nessun requisito GPU"],
        compatible_drivers: &["qualsiasi"],
        upstream: None,
        expected_renderer: "Google SwiftShader",
    },
];

pub fn descriptor(kind: GpuBackendKind) -> &'static BackendDescriptor {
    DESCRIPTORS.iter().find(|d| d.kind == kind).expect("every kind has a descriptor")
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
    if r.kernel.binder.usable() {
        checks.push(Check::pass("binder", "Binder nel kernel", "disponibile"));
    } else {
        checks.push(Check::fail("binder", "Binder nel kernel", "CONFIG_ANDROID_BINDERFS assente").remedy("Usa un kernel con binder (es. linux-generic Ubuntu o linux-image Debian ≥ 6.x)"));
    }
    if r.waydroid.installed {
        checks.push(Check::pass("waydroid", "Waydroid", r.waydroid.version.as_deref().unwrap_or("installato")));
    } else {
        checks.push(Check::missing("waydroid", "Waydroid", "non installato").remedy("Installazione guidata da DroidNative (repository ufficiale repo.waydro.id)"));
    }
}

fn session_check(r: &HostReport, required: bool) -> Check {
    match r.session.kind {
        SessionKind::Wayland => Check::pass("wayland", "Sessione Wayland", r.session.compositor.as_deref().unwrap_or("attiva")),
        SessionKind::X11 if !required => Check::warn("wayland", "Sessione Wayland", "sessione X11: serve un compositor Wayland annidato (weston/cage)"),
        _ => Check::fail("wayland", "Sessione Wayland", "richiesta una sessione Wayland").remedy("Accedi con una sessione Wayland (es. «GNOME» invece di «GNOME su Xorg»)"),
    }
}

fn nvidia_display_check(r: &HostReport) -> Check {
    match r.gpu.display_gpu() {
        Some(d) if d.vendor == GpuVendor::Nvidia => {
            Check::pass("compositor_gpu", "Compositor sulla GPU NVIDIA", &format!("schermo su {} ({})", d.display_name(), d.connected_outputs.join(", ")))
        }
        Some(d) => Check::fail(
            "compositor_gpu",
            "Compositor sulla GPU NVIDIA",
            &format!("lo schermo è collegato a {}: il compositor non può mostrare i buffer NVIDIA block-linear", d.display_name()),
        )
        .remedy("Collega il monitor alla scheda NVIDIA, oppure esegui Waydroid in gamescope sulla GPU NVIDIA (non funziona su portatili ibridi Intel+NVIDIA)"),
        None => Check::unknown("compositor_gpu", "Compositor sulla GPU NVIDIA", "nessuna uscita video collegata rilevata"),
    }
}

fn assess_mesa(r: &HostReport) -> BackendAssessment {
    let mut checks = Vec::new();
    let display = r.gpu.display_gpu();
    // Prefer the Mesa GPU that drives the display; otherwise the first Mesa-capable one.
    let target = r.gpu.mesa_devices().find(|d| d.drives_display()).or_else(|| r.gpu.mesa_devices().next());

    match target {
        Some(dev) => {
            checks.push(Check::pass(
                "mesa_gpu",
                "GPU compatibile Mesa",
                &format!("{} ({}) → {}", dev.display_name(), dev.kernel_driver.as_deref().unwrap_or("?"), dev.render_node.as_deref().unwrap_or("?")),
            ));
            if dev.render_node_accessible {
                checks.push(Check::pass("render_access", "Accesso al nodo render", "consentito all'utente"));
            } else {
                checks.push(Check::warn("render_access", "Accesso al nodo render", "l'utente non può aprire il nodo (gruppo render/ACL)"));
            }
            match display {
                Some(d) if d.pci_slot == dev.pci_slot => {
                    checks.push(Check::pass("same_gpu", "Compositor sulla stessa GPU", "rendering e composizione sulla stessa GPU"))
                }
                Some(d) if d.vendor == GpuVendor::Nvidia => checks.push(
                    Check::fail(
                        "same_gpu",
                        "Compositor sulla stessa GPU",
                        &format!(
                            "lo schermo è su {}: il compositor non riesce a importare i buffer di {} (il driver NVIDIA importa dmabuf LINEAR di altri vendor solo come GL_TEXTURE_EXTERNAL_OES). \
                             Verificato su GNOME/Mutter: «failed to import supplied dmabufs: Could not bind the given EGLImage to a CoglTexture2D», poi crash dell'hwcomposer appena Android apre una finestra.",
                            d.display_name(),
                            dev.display_name()
                        ),
                    )
                    .remedy("Collega il monitor alla GPU AMD/Intel, oppure usa un backend NVIDIA o il fallback software"),
                ),
                Some(d) => checks.push(Check::info(
                    "same_gpu",
                    "Compositor sulla stessa GPU",
                    &format!("PRIME tra {} e {}: generalmente supportato da Mesa", d.display_name(), dev.display_name()),
                )),
                None => checks.push(Check::unknown("same_gpu", "Compositor sulla stessa GPU", "GPU del display non determinata")),
            }
        }
        None => checks.push(
            Check::fail("mesa_gpu", "GPU compatibile Mesa", "nessuna GPU AMD/Intel con nodo render").remedy("Usa un backend NVIDIA o il fallback software"),
        ),
    }
    checks.push(session_check(r, false));
    common_runtime_checks(r, &mut checks);
    if let Some(cfg) = r.waydroid.config.as_ref() {
        let forced_sw = cfg.properties.iter().any(|(k, v)| k == "ro.hardware.egl" && v == "swiftshader");
        if forced_sw {
            checks.push(Check::missing("cfg_override", "Configurazione Waydroid", "ro.hardware.egl=swiftshader forza il rendering software").remedy("Seleziona Mesa Native in DroidNative per rimuovere l'override"));
        }
        match (&cfg.drm_device, target.and_then(|t| t.render_node.as_ref())) {
            (Some(cur), Some(want)) if cur == want => checks.push(Check::pass("drm_device", "Nodo GPU configurato", cur)),
            (Some(cur), Some(want)) => checks.push(Check::missing("drm_device", "Nodo GPU configurato", &format!("{cur} invece di {want}"))),
            (None, Some(want)) => checks.push(Check::info("drm_device", "Nodo GPU configurato", &format!("automatico (Waydroid sceglierà il primo nodo non NVIDIA: {want})"))),
            _ => {}
        }
    }
    BackendAssessment {
        descriptor: descriptor(GpuBackendKind::MesaNative).clone(),
        readiness: readiness(&checks),
        target_device: target.map(|d| d.display_name()),
        target_render_node: target.and_then(|d| d.render_node.clone()),
        checks,
    }
}

fn nvidia_driver_checks(r: &HostReport, checks: &mut Vec<Check>, experimental: bool) {
    let Some(drv) = r.gpu.nvidia.as_ref() else {
        checks.push(Check::fail("nvidia_driver", "Driver NVIDIA", "driver proprietario NVIDIA non caricato (nouveau?)").remedy("Installa nvidia-driver-<versione>-open"));
        return;
    };
    let version = drv.version.clone().unwrap_or_else(|| "?".into());
    let tuple = drv.version_tuple();
    let flavor = drv.module_flavor;

    if experimental {
        let detail = format!("{version} ({})", if flavor == NvidiaModuleFlavor::Open { "modulo open" } else { "modulo chiuso" });
        match (tuple, flavor) {
            (Some((major, _)), NvidiaModuleFlavor::Open) if major >= 615 => checks.push(Check::pass("nvidia_driver", "Driver NVIDIA", &format!("{detail}: ✅ in upstream"))),
            (Some((550, 107)), _) => checks.push(Check::fail("nvidia_driver", "Driver NVIDIA", &format!("{detail}: versione rifiutata da upstream"))),
            // Upstream table: open 580–610 and closed 550–610 are 🧪; newer closed builds are untested but accepted.
            (Some((major, _)), _) if major >= 550 => {
                checks.push(Check::warn("nvidia_driver", "Driver NVIDIA", &format!("{detail}: 🧪 test driver superati, non provato in Waydroid")))
            }
            _ => checks.push(Check::fail("nvidia_driver", "Driver NVIDIA", &format!("{detail}: non supportato (serve ≥ 550)"))),
        }
    } else {
        if flavor == NvidiaModuleFlavor::Open {
            checks.push(Check::pass("nvidia_module", "Modulo kernel open", "open-gpu-kernel-modules (DMA-BUF disponibile)"));
        } else {
            checks.push(Check::fail("nvidia_module", "Modulo kernel open", "modulo chiuso: nessun export DMA-BUF").remedy("Installa la variante -open del driver (es. nvidia-driver-615-open)"));
        }
        match tuple {
            Some(t) if t >= (595, 71) => checks.push(Check::pass("nvidia_driver", "Driver ≥ 595.71", &version)),
            Some(_) => checks.push(Check::fail("nvidia_driver", "Driver ≥ 595.71", &format!("{version} troppo vecchio")).remedy("Aggiorna al driver 610 o successivo")),
            None => checks.push(Check::unknown("nvidia_driver", "Driver ≥ 595.71", "versione non leggibile")),
        }
    }
    match drv.modeset {
        Some(true) => checks.push(Check::pass("modeset", "nvidia-drm.modeset=1", "attivo")),
        Some(false) => checks.push(Check::fail("modeset", "nvidia-drm.modeset=1", "disattivato: niente DMA-BUF né sync_fd").remedy("Aggiungi nvidia-drm.modeset=1 ai parametri del kernel e riavvia")),
        None => checks.push(Check::unknown("modeset", "nvidia-drm.modeset=1", "parametro non leggibile")),
    }
}

fn assess_nvidia(r: &HostReport, experimental: bool) -> BackendAssessment {
    let kind = if experimental { GpuBackendKind::NvidiaExperimental } else { GpuBackendKind::NvidiaAccelerated };
    let mut checks = Vec::new();
    let dev = r.gpu.nvidia_device();
    match dev {
        Some(d) => {
            let generation = nvidia_generation(d);
            let gen_label = format!("{} — architettura {:?}", d.display_name(), generation);
            let min = if experimental { NvidiaGeneration::Maxwell } else { NvidiaGeneration::Turing };
            if generation == NvidiaGeneration::Unknown {
                checks.push(Check::unknown("nvidia_gpu", "GPU NVIDIA supportata", &gen_label));
            } else if generation < min {
                checks.push(Check::fail("nvidia_gpu", "GPU NVIDIA supportata", &format!("{gen_label}: troppo vecchia")));
            } else if experimental && generation != NvidiaGeneration::Blackwell {
                checks.push(Check::warn("nvidia_gpu", "GPU NVIDIA supportata", &format!("{gen_label}: 🧪 in upstream (Waydroid confermato solo su RTX 50)")));
            } else {
                checks.push(Check::pass("nvidia_gpu", "GPU NVIDIA supportata", &gen_label));
            }
            if !experimental && generation == NvidiaGeneration::Ada {
                checks.push(Check::warn(
                    "ada_bug",
                    "Bug noto del driver su Ada",
                    "redroid-nvidia ha riprodotto su RTX 4060 un SIGABRT di SurfaceFlinger (vn_ring_submit_locked) causato da un'asserzione nel driver kernel NVIDIA, aperto upstream",
                ));
            }
            nvidia_driver_checks(r, &mut checks, experimental);
        }
        None => checks.push(Check::fail("nvidia_gpu", "GPU NVIDIA supportata", "nessuna GPU NVIDIA rilevata")),
    }
    if dev.is_some() {
        checks.push(nvidia_display_check(r));
        checks.push(session_check(r, true));
        match r.session.compositor.as_deref() {
            Some("KWin") => checks.push(Check::pass("compositor_tested", "Compositor testato upstream", "KWin / Plasma 6")),
            Some(other) => checks.push(Check::warn("compositor_tested", "Compositor testato upstream", &format!("{other}: upstream testa solo KWin / Plasma 6"))),
            None => checks.push(Check::unknown("compositor_tested", "Compositor testato upstream", "compositor non riconosciuto")),
        }
        if !experimental {
            let u = &r.gpu.udmabuf;
            match (u.exists, u.accessible) {
                (true, true) => checks.push(Check::pass("udmabuf", "Accesso a /dev/udmabuf", "consentito")),
                (true, false) => checks.push(Check::warn("udmabuf", "Accesso a /dev/udmabuf", "presente ma non accessibile: cursore invisibile e screenshot neri").remedy("La regola udev uaccess di waydroid-nvidia lo concede al prossimo login")),
                (false, _) => checks.push(Check::warn("udmabuf", "Accesso a /dev/udmabuf", "modulo udmabuf non caricato").remedy("sudo modprobe udmabuf")),
            }
        }
        if experimental {
            checks.push(Check::info("android_version", "Versione Android", "solo immagine ufficiale Android 13 (LineageOS 20)"));
        }
    }
    common_runtime_checks(r, &mut checks);
    if dev.is_some() {
        let want = if experimental { NvidiaMode::Prolapse } else { NvidiaMode::Venus };
        let tag = r.waydroid.nvidia_stack.as_ref().map(|s| s.tag.clone()).unwrap_or_else(|| "versione non registrata".into());
        match (&r.waydroid.nvidia_setup, r.waydroid.nvidia_mode) {
            (Some(_), Some(m)) if m == want => {
                checks.push(Check::pass("waydroid_nvidia", "waydroid-nvidia", &format!("{tag}, modalità {} attiva", m.as_arg())))
            }
            (Some(_), current) => checks.push(
                Check::missing(
                    "waydroid_nvidia",
                    "waydroid-nvidia",
                    &format!("{tag} installato, modalità attiva: {}", current.map(|m| m.as_arg()).unwrap_or("nessuna")),
                )
                .remedy(&format!("Seleziona questo backend: DroidNative esegue waydroid-nvidia-setup --mode {}", want.as_arg())),
            ),
            (None, _) => checks.push(
                Check::missing("waydroid_nvidia", "waydroid-nvidia", "non installato").remedy(&format!(
                    "Installazione guidata del rilascio {} (hash e commit fissati in DroidNative, attestazioni di build verificate)",
                    crate::nvidia_stack::latest_release().tag
                )),
            ),
        }
        if r.waydroid.distro_package.as_deref().is_some_and(|s| s.contains("ok installed")) && r.waydroid.nvidia_setup.is_none() {
            checks.push(Check::warn(
                "runtime_replacement",
                "Sostituzione del runtime",
                "waydroid-nvidia installa una Waydroid patchata e non sovrascrive i file di un pacchetto: il pacchetto waydroid attuale va rimosso prima (immagini e dati restano in /var/lib/waydroid e ~/.local/share/waydroid)",
            ));
        }
        if !experimental && r.waydroid.nvidia_mode == Some(NvidiaMode::Venus) && !r.waydroid.venus_service.is_active() {
            checks.push(Check::info("venus_service", "Render server Venus (wd-venus)", "non attivo: DroidNative lo avvia insieme alla sessione Android"));
        }
    }
    BackendAssessment {
        descriptor: descriptor(kind).clone(),
        readiness: readiness(&checks),
        target_device: dev.map(|d| d.display_name()),
        target_render_node: dev.and_then(|d| d.render_node.clone()),
        checks,
    }
}

fn assess_software(r: &HostReport) -> BackendAssessment {
    let mut checks = vec![Check::warn("performance", "Prestazioni", "rendering su CPU: giochi e app 3D lenti, nessuna accelerazione")];
    checks.push(session_check(r, false));
    common_runtime_checks(r, &mut checks);
    BackendAssessment {
        descriptor: descriptor(GpuBackendKind::SoftwareFallback).clone(),
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
        (GpuBackendKind::NvidiaAccelerated, "Il display è sulla GPU NVIDIA e i requisiti di Venus sono soddisfatti: è il percorso hardware più collaudato per questa configurazione.".to_string())
    } else if ok(GpuBackendKind::MesaNative) && (display_vendor != Some(GpuVendor::Nvidia) || warn_count(GpuBackendKind::MesaNative) == 0) {
        (GpuBackendKind::MesaNative, "GPU AMD/Intel che pilota il display: Mesa è il percorso nativo senza proxy.".to_string())
    } else if ok(GpuBackendKind::NvidiaExperimental) {
        (GpuBackendKind::NvidiaExperimental, "Unico percorso hardware NVIDIA compatibile, ma sperimentale.".to_string())
    } else if ok(GpuBackendKind::MesaNative) {
        (GpuBackendKind::MesaNative, "Mesa sull'iGPU con import cross-GPU da verificare.".to_string())
    } else {
        (GpuBackendKind::SoftwareFallback, "Nessun percorso hardware compatibile: solo rendering software.".to_string())
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
