//! Guided setup: the ordered steps that take this host from its current state to a running,
//! GPU-accelerated Android. Each step is either done, runnable (a privileged op), or blocked.

use serde::Serialize;

use crate::gpu::{GpuBackendKind, GpuPlan};
use crate::nvidia_stack::NvidiaMode;
use crate::host::HostReport;
use crate::privileged::{validate_suite, GpuSelection, PrivilegedOp, SystemType, DIAGNOSTIC_PACKAGES};
use crate::tr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StepState {
    Done,
    Pending,
    Blocked,
    Optional,
}

#[derive(Debug, Clone, Serialize)]
pub struct SetupStep {
    pub id: &'static str,
    pub title: String,
    pub detail: String,
    pub state: StepState,
    /// Present when DroidNative can perform the step itself (after polkit authentication).
    pub op: Option<PrivilegedOp>,
    /// Human readable description of `op`, shown before asking for confirmation.
    pub action: Option<String>,
}

impl SetupStep {
    fn with_action(mut self) -> Self {
        self.action = self.op.as_ref().map(PrivilegedOp::description);
        self
    }
}

pub fn plan(host: &HostReport, gpu: &GpuPlan) -> Vec<SetupStep> {
    let w = &host.waydroid;
    let mut steps = Vec::new();

    let suite = host.os.upstream_codename.clone().filter(|s| validate_suite(s).is_ok());
    let install_title = tr!("Installare Waydroid", "Install Waydroid");
    let version = w.version.as_deref().unwrap_or("?");
    steps.push(match (w.installed, &suite) {
        (true, _) => SetupStep {
            id: "install_waydroid",
            title: install_title.clone(),
            detail: if w.nvidia_setup.is_some() {
                tr!("Waydroid patchata da waydroid-nvidia: {version}", "Waydroid patched by waydroid-nvidia: {version}")
            } else {
                tr!("installato: {version}", "installed: {version}")
            },
            state: StepState::Done,
            op: None,
            action: None,
        },
        (false, Some(s)) => SetupStep {
            id: "install_waydroid",
            title: install_title.clone(),
            detail: tr!(
                "repository ufficiale repo.waydro.id, suite {s}; la chiave viene verificata prima dell'uso",
                "official repo.waydro.id repository, suite {s}; the signing key is checked before use"
            ),
            state: StepState::Pending,
            op: Some(PrivilegedOp::InstallWaydroid { suite: s.clone() }),
            action: None,
        },
        (false, None) => SetupStep {
            id: "install_waydroid",
            title: install_title.clone(),
            detail: {
                let release = host.os.upstream_codename.clone().unwrap_or_else(|| tr!("sconosciuta", "unknown"));
                tr!("la release {release:?} non è pubblicata su repo.waydro.id", "release {release:?} is not published on repo.waydro.id")
            },
            state: StepState::Blocked,
            op: None,
            action: None,
        },
    });

    steps.push(SetupStep {
        id: "init_images",
        title: tr!("Scaricare le immagini Android", "Download the Android images"),
        detail: if w.initialized {
            let n = w.images.len();
            tr!("{n} immagini presenti", "{n} images present")
        } else {
            tr!(
                "LineageOS 20 (Android 13) dal canale OTA ufficiale, verificate con SHA-256 da Waydroid",
                "LineageOS 20 (Android 13) from the official OTA channel, SHA-256 checked by Waydroid"
            )
        },
        state: if w.initialized {
            StepState::Done
        } else if w.installed {
            StepState::Pending
        } else {
            StepState::Blocked
        },
        op: (!w.initialized && w.installed).then_some(PrivilegedOp::InitWaydroid { system_type: SystemType::Vanilla }),
        action: None,
    });

    let service_active = w.container_service.is_active();
    steps.push(SetupStep {
        id: "container_service",
        title: tr!("Avviare il servizio del container", "Start the container service"),
        detail: format!("waydroid-container.service: {}", w.container_service.active.clone().unwrap_or_else(|| tr!("non presente", "not present"))),
        state: if service_active {
            StepState::Done
        } else if w.initialized {
            StepState::Pending
        } else {
            StepState::Blocked
        },
        op: (!service_active && w.initialized).then_some(PrivilegedOp::EnableContainerService),
        action: None,
    });

    let blocked_by_fw = host.firewall.may_block_container();
    steps.push(SetupStep {
        id: "container_network",
        title: tr!("Consentire la rete del container", "Allow the container network"),
        detail: match (&w.dhcp_lease, blocked_by_fw) {
            (Some(ip), _) => tr!("Android ha l'indirizzo {ip}", "Android has address {ip}"),
            (None, true) => tr!(
                "il firewall UFW scarta DHCP e DNS di Android e il suo traffico inoltrato: verranno aggiunte 3 regole limitate all'interfaccia waydroid0",
                "the UFW firewall drops Android's DHCP, DNS and forwarded traffic: 3 rules limited to the waydroid0 interface will be added"
            ),
            (None, false) => tr!("nessun firewall che blocchi il bridge waydroid0", "no firewall blocking the waydroid0 bridge"),
        },
        state: match (&w.dhcp_lease, blocked_by_fw, w.installed) {
            (Some(_), _, _) | (None, false, _) => StepState::Done,
            (None, true, true) => StepState::Pending,
            (None, true, false) => StepState::Blocked,
        },
        op: (w.dhcp_lease.is_none() && blocked_by_fw && w.installed).then_some(PrivilegedOp::AllowContainerNetwork),
        action: None,
    });

    let rec = gpu.assessments.iter().find(|a| a.descriptor.kind == gpu.recommended);
    let gpu_step = match (gpu.recommended, rec) {
        (GpuBackendKind::MesaNative, Some(a)) => {
            let node = a.target_render_node.clone();
            let configured = w.config.as_ref().and_then(|c| c.drm_device.clone());
            let done = node.is_some() && configured == node;
            SetupStep {
                id: "gpu_backend",
                title: tr!("Configurare il backend GPU Mesa Native", "Set up the Mesa Native GPU backend"),
                detail: format!("{} → {}", a.target_device.as_deref().unwrap_or("?"), node.as_deref().unwrap_or("?")),
                state: if done {
                    StepState::Done
                } else if w.initialized {
                    StepState::Pending
                } else {
                    StepState::Blocked
                },
                op: (!done && w.initialized).then(|| node.map(|n| PrivilegedOp::SelectGpu { selection: GpuSelection::RenderNode(n) })).flatten(),
                action: None,
            }
        }
        (kind @ (GpuBackendKind::NvidiaAccelerated | GpuBackendKind::NvidiaExperimental), Some(a)) => {
            let want = if kind == GpuBackendKind::NvidiaAccelerated { NvidiaMode::Venus } else { NvidiaMode::Prolapse };
            let release = crate::nvidia_stack::latest_release();
            let (state, detail, op) = match (w.nvidia_setup.is_some(), w.nvidia_mode) {
                (true, Some(m)) if m == want => {
                    let mode = m.as_arg();
                    (
                        StepState::Done,
                        tr!(
                            "waydroid-nvidia attivo in modalità {mode}; dopo ogni aggiornamento del driver NVIDIA ripeti questo passo",
                            "waydroid-nvidia active in {mode} mode; repeat this step after every NVIDIA driver update"
                        ),
                        None,
                    )
                }
                (true, _) => {
                    let mode = want.as_arg();
                    (
                        StepState::Pending,
                        tr!("waydroid-nvidia installato: attivare la modalità {mode}", "waydroid-nvidia installed: switch to {mode} mode"),
                        Some(PrivilegedOp::NvidiaSetMode { mode: want }),
                    )
                }
                (false, _) => {
                    let (tag, mode) = (release.tag, want.as_arg());
                    (
                        StepState::Pending,
                        tr!(
                            "installa waydroid-nvidia {tag} (modalità {mode}) sostituendo il pacchetto waydroid della distribuzione; immagini e dati restano",
                            "install waydroid-nvidia {tag} ({mode} mode) replacing the distribution's waydroid package; images and data are kept"
                        ),
                        Some(PrivilegedOp::InstallNvidiaStack { tag: release.tag.to_string(), mode: want }),
                    )
                }
            };
            let name = &a.descriptor.name;
            SetupStep { id: "gpu_backend", title: tr!("Configurare il backend GPU {name}", "Set up the {name} GPU backend"), detail, state, op, action: None }
        }
        _ => SetupStep {
            id: "gpu_backend",
            title: tr!("Backend GPU", "GPU backend"),
            detail: tr!("nessun backend hardware compatibile: verrà usato il rendering software", "no compatible hardware backend: software rendering will be used"),
            state: StepState::Done,
            op: None,
            action: None,
        },
    };
    steps.push(gpu_step);

    let gapps = w.system_image == Some(crate::extras::SystemImage::Gapps);
    steps.push(SetupStep {
        id: "google_services",
        title: tr!("Servizi Google (Play Store)", "Google services (Play Store)"),
        detail: if gapps {
            tr!(
                "immagine GAPPS attiva: registra il dispositivo con il tuo account Google (ID nella sezione «Registra il dispositivo con Google»)",
                "GAPPS image active: register the device with your Google account (ID in the “Register the device with Google” section)"
            )
        } else {
            tr!(
                "Play Store e Google Play Services con l'immagine ufficiale GAPPS (~1 GB); app e dati restano",
                "Play Store and Google Play Services with the official GAPPS image (~1 GB); apps and data are kept"
            )
        },
        state: if gapps {
            StepState::Done
        } else if w.initialized {
            StepState::Optional
        } else {
            StepState::Blocked
        },
        op: (!gapps && w.initialized).then_some(PrivilegedOp::SwitchSystemImage { system_type: SystemType::Gapps }),
        action: None,
    });

    steps.push(SetupStep {
        id: "arm_translation",
        title: tr!("Traduzione ARM (libndk)", "ARM translation (libndk)"),
        detail: if w.arm_translation {
            tr!("attiva: le app con solo codice ARM vengono tradotte", "active: ARM-only apps are translated")
        } else {
            tr!(
                "permette di usare app e giochi con solo codice ARM; libndk è indicata per le CPU AMD",
                "lets you run ARM-only apps and games; libndk is the right choice for AMD CPUs"
            )
        },
        state: if w.arm_translation {
            StepState::Done
        } else if w.initialized {
            StepState::Optional
        } else {
            StepState::Blocked
        },
        op: (!w.arm_translation && w.initialized).then_some(PrivilegedOp::InstallArmTranslation),
        action: None,
    });

    let have_diag = host.gpu.opengl.is_some() && host.gpu.vulkan.vulkaninfo_available;
    steps.push(SetupStep {
        id: "diagnostics",
        title: tr!("Strumenti diagnostici (facoltativo)", "Diagnostic tools (optional)"),
        detail: {
            let pkgs = DIAGNOSTIC_PACKAGES.join(", ");
            tr!("{pkgs} per leggere versioni OpenGL e dispositivi Vulkan dell'host", "{pkgs} to read the host's OpenGL version and Vulkan devices")
        },
        state: if have_diag { StepState::Done } else { StepState::Optional },
        op: (!have_diag).then_some(PrivilegedOp::InstallDiagnostics),
        action: None,
    });
    steps.into_iter().map(SetupStep::with_action).collect()
}
